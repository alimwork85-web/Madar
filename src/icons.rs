//! Program icons for the Apps page.
//!
//! The shell keeps about 10 MB once it has read an icon, so a short child
//! process (`madar --icons`) reads them and sends the pixels back.

use crate::actions;
use crate::config::Config;
use crate::wheel::paint::{contrast_of, luminance};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender, channel};
use windows::Win32::Foundation::{HWND, LPARAM, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx,
};
use windows::Win32::UI::Shell::{
    IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_ICONONLY,
};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};
use windows::core::HSTRING;

/// First argument of the child; then the size and the paths.
pub const CHILD: &str = "--icons";

/// An icon finished loading and waits in `Icons`.
pub const WM_ICON: u32 = WM_APP + 13;
/// Logical sizes, from DESIGN.md: beside a name, and alone.
pub const WITH_NAME: f32 = 22.0;
pub const ALONE: f32 = 32.0;
/// A handler may ignore the requested size; nothing larger is useful here.
pub const MAX_SIDE: u32 = 1024;
const LIGHT: u32 = 0xF2F2F4;
const DARK: u32 = 0x1B1B1E;

#[derive(Debug)]
pub struct Icon {
    pub width: u32,
    pub height: u32,
    /// Premultiplied BGRA, top row first.
    pub pixels: Vec<u8>,
    /// Mean relative luminance of the visible pixels.
    light: f32,
}

impl Icon {
    /// None when nothing in the bitmap is visible.
    fn new(width: u32, height: u32, mut pixels: Vec<u8>) -> Option<Self> {
        // Old icons have no alpha channel at all.
        if pixels.chunks(4).all(|p| p[3] == 0) {
            if pixels.iter().all(|&v| v == 0) {
                return None;
            }
            pixels.chunks_mut(4).for_each(|p| p[3] = 255);
        }
        // Some handlers return straight alpha; a channel above its alpha proves it.
        if pixels.chunks(4).any(|p| p[..3].iter().any(|&v| v > p[3])) {
            for p in pixels.chunks_mut(4) {
                let a = p[3] as u16;
                p[..3]
                    .iter_mut()
                    .for_each(|v| *v = (*v as u16 * a / 255) as u8);
            }
        }
        let (mut sum, mut weight) = (0.0, 0.0);
        for p in pixels.chunks(4).filter(|p| p[3] > 0) {
            let a = p[3] as f32 / 255.0;
            let ch = |v: u8| ((v as f32 / a).round() as u32).min(255);
            sum += a * luminance(ch(p[2]) << 16 | ch(p[1]) << 8 | ch(p[0]));
            weight += a;
        }
        Some(Self {
            width,
            height,
            pixels,
            light: sum / weight,
        })
    }

    /// Color to put behind the icon when it would fade into `fill`.
    pub fn base(&self, fill: u32) -> Option<u32> {
        if contrast_of(self.light, luminance(fill)) >= 3.0 {
            return None;
        }
        let against = |c| contrast_of(self.light, luminance(c));
        Some(if against(LIGHT) >= against(DARK) {
            LIGHT
        } else {
            DARK
        })
    }
}

type Loaded = (u32, String, Option<Icon>);

pub struct Icons {
    hwnd: HWND,
    side: u32,
    /// None while loading, and for a program without a readable icon.
    cache: HashMap<String, Option<Rc<Icon>>>,
    tx: Sender<Loaded>,
    rx: Receiver<Loaded>,
}

impl Icons {
    pub fn new(hwnd: HWND) -> Self {
        let (tx, rx) = channel();
        Self {
            hwnd,
            side: 0,
            cache: HashMap::new(),
            tx,
            rx,
        }
    }

    /// Starts loading the icons that are not in the cache yet.
    pub fn request(&mut self, c: &Config, scale: f32) {
        self.collect();
        let apps = || c.my_apps.iter().chain(&c.apps);
        let size = if c.show_names { WITH_NAME } else { ALONE };
        let side = (size * scale).round() as u32;
        if side != self.side {
            self.side = side;
            self.cache.clear();
        }
        self.cache.retain(|path, _| apps().any(|a| a.path == *path));
        let mut missing = Vec::new();
        for a in apps() {
            if !self.cache.contains_key(&a.path) {
                self.cache.insert(a.path.clone(), None);
                missing.push(a.path.clone());
            }
        }
        if missing.is_empty() {
            return;
        }
        let (tx, target) = (self.tx.clone(), self.hwnd.0 as usize);
        std::thread::spawn(move || {
            // shortcut: no time limit, so a dead network path delays the icons after it.
            let _ = ask_child(side, &missing, |path, icon| {
                let _ = tx.send((side, path, icon));
                let hwnd = HWND(target as *mut _);
                let _ = unsafe { PostMessageW(Some(hwnd), WM_ICON, WPARAM(0), LPARAM(0)) };
            });
        });
    }

    /// Takes finished icons in.
    pub fn collect(&mut self) {
        while let Ok((side, path, icon)) = self.rx.try_recv() {
            if side == self.side
                && let Some(entry) = self.cache.get_mut(&path)
            {
                *entry = icon.map(Rc::new);
            }
        }
    }

    pub fn get(&self, path: &str) -> Option<Rc<Icon>> {
        self.cache.get(path)?.clone()
    }
}

fn ask_child(
    side: u32,
    paths: &[String],
    mut found: impl FnMut(String, Option<Icon>),
) -> std::io::Result<()> {
    let mut child = Command::new(std::env::current_exe()?)
        .arg(CHILD)
        .arg(side.to_string())
        .args(paths)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut read = || -> std::io::Result<()> {
        let Some(out) = child.stdout.as_mut() else {
            return Ok(());
        };
        for path in paths {
            found(path.clone(), receive(out)?);
        }
        Ok(())
    };
    let result = read();
    // The pipe may have ended early; never leave the child behind.
    let _ = child.kill();
    let _ = child.wait();
    result
}

/// The child's work: one answer per path on stdout.
pub fn child(side: u32, paths: impl Iterator<Item = String>) -> std::io::Result<()> {
    let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
    let mut out = std::io::stdout().lock();
    for path in paths {
        send(&mut out, load(&path, side))?;
        out.flush()?;
    }
    Ok(())
}

/// Width, height, pixels. A size of 0 means no icon.
fn send(out: &mut impl Write, icon: Option<Icon>) -> std::io::Result<()> {
    let (w, h, pixels) = icon.map_or((0, 0, Vec::new()), |i| (i.width, i.height, i.pixels));
    out.write_all(&w.to_le_bytes())?;
    out.write_all(&h.to_le_bytes())?;
    out.write_all(&pixels)
}

fn receive(input: &mut impl Read) -> std::io::Result<Option<Icon>> {
    let mut number = || -> std::io::Result<u32> {
        let mut b = [0; 4];
        input.read_exact(&mut b)?;
        Ok(u32::from_le_bytes(b))
    };
    let (w, h) = (number()?, number()?);
    if w == 0 || h == 0 {
        return Ok(None);
    }
    if w > MAX_SIDE || h > MAX_SIDE {
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    let mut pixels = vec![0; (w * h * 4) as usize];
    input.read_exact(&mut pixels)?;
    Ok(Icon::new(w, h, pixels))
}

/// The shell's icon for a program or a shortcut. Needs COM on this thread.
fn load(path: &str, side: u32) -> Option<Icon> {
    actions::program(path, true).ok()?;
    unsafe {
        let item: IShellItemImageFactory =
            SHCreateItemFromParsingName(&HSTRING::from(path), None).ok()?;
        let size = SIZE {
            cx: side as i32,
            cy: side as i32,
        };
        let bitmap = item.GetImage(size, SIIGBF_ICONONLY).ok()?;
        let icon = read(bitmap);
        let _ = DeleteObject(bitmap.into());
        icon
    }
}

fn read(bitmap: HBITMAP) -> Option<Icon> {
    unsafe {
        let mut b = BITMAP::default();
        let size = size_of::<BITMAP>() as i32;
        if GetObjectW(bitmap.into(), size, Some((&raw mut b).cast())) == 0 {
            return None;
        }
        let (w, h) = (b.bmWidth, b.bmHeight.abs());
        let usable = 1..=MAX_SIDE as i32;
        if !usable.contains(&w) || !usable.contains(&h) {
            return None;
        }
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        let dc = CreateCompatibleDC(None);
        let bits = Some(pixels.as_mut_ptr().cast());
        let rows = GetDIBits(dc, bitmap, 0, h as u32, bits, &mut info, DIB_RGB_COLORS);
        let _ = DeleteDC(dc);
        if rows != h {
            return None;
        }
        Icon::new(w as u32, h as u32, pixels)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wheel::paint::theme;

    fn plain(bgra: [u8; 4]) -> Icon {
        Icon::new(2, 2, bgra.repeat(4)).unwrap()
    }

    #[test]
    fn base_only_below_three_to_one_and_always_readable() {
        for name in crate::config::THEMES {
            let p = theme(name);
            for v in 0..=255u8 {
                let icon = plain([v, v, v, 255]);
                let c = u32::from_be_bytes([0, v, v, v]);
                for fill in [p.slot, p.accent] {
                    let own = contrast_of(luminance(c), luminance(fill));
                    let base = icon.base(fill);
                    assert_eq!(base.is_some(), own < 3.0, "{name} {v}");
                    if let Some(b) = base {
                        assert!(contrast_of(luminance(c), luminance(b)) >= 3.0, "{name} {v}");
                    }
                }
            }
        }
    }

    #[test]
    fn alpha_kinds() {
        assert!(Icon::new(1, 1, vec![0; 4]).is_none());
        assert_eq!(plain([9, 9, 9, 0]).pixels[..4], [9, 9, 9, 255]);
        assert_eq!(plain([255, 0, 0, 128]).pixels[..4], [128, 0, 0, 128]);
        assert_eq!(plain([100, 0, 0, 128]).pixels[..4], [100, 0, 0, 128]);
        // Half-transparent white is still white.
        assert!(plain([128, 128, 128, 128]).light > 0.99);
    }

    #[test]
    fn pipe_format() {
        let mut pipe = Vec::new();
        send(&mut pipe, Some(plain([10, 20, 30, 255]))).unwrap();
        send(&mut pipe, None).unwrap();
        let mut input = &pipe[..];
        let icon = receive(&mut input).unwrap().unwrap();
        assert_eq!((icon.width, icon.height), (2, 2));
        assert_eq!(icon.pixels, [10, 20, 30, 255].repeat(4));
        assert!(receive(&mut input).unwrap().is_none());
        assert!(receive(&mut input).is_err());
        assert!(receive(&mut &[255u8; 8][..]).is_err());
    }

    #[test]
    fn loads_a_system_program_and_refuses_others() {
        let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        let exe = format!(
            "{}\\System32\\cmd.exe",
            std::env::var("SystemRoot").unwrap()
        );
        let icon = load(&exe, 33).unwrap();
        assert_eq!((icon.width, icon.height), (33, 33));
        assert_eq!(icon.pixels.len(), 33 * 33 * 4);
        assert!(load("C:\\nope.exe", 33).is_none());
        assert!(load("cmd.exe", 33).is_none());
    }
}
