//! Draws the tray menu in the colors of the wheel's theme.
//!
//! It stays a Windows menu, with its keys and its names for screen readers;
//! only the painting is ours. Should that fail, Windows draws the plain menu.

use super::*;
use crate::wheel::paint::{Palette, color, mix, palette, theme};
use std::cell::RefCell;
use windows::Win32::Foundation::{COLORREF, LRESULT, RECT};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Controls::{
    DRAWITEMSTRUCT, MEASUREITEMSTRUCT, ODS_CHECKED, ODS_NOACCEL, ODS_SELECTED, ODT_MENU,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Shell::{
    DefSubclassProc, GetWindowSubclass, RemoveWindowSubclass, SetWindowSubclass,
};
use windows::core::Result;

/// In the item data: the menu's id, or one of these for what has none.
pub const SEPARATOR: usize = 0;
pub const RESUME: usize = 5;
pub const THEMES_MENU: usize = 6;

/// At 96 DPI.
const WIDTH: f32 = 176.0;
const HEIGHT: f32 = 34.0;
const LINE: f32 = 9.0;
/// Between a chosen row's fill and the menu's edge.
const SIDE: f32 = 3.0;
const ICON: f32 = 22.0;
const TEXT: f32 = 42.0;
const SUBCLASS: usize = 1;

struct Look {
    p: Palette,
    dpi: u32,
    back: HBRUSH,
    rt: ID2D1DCRenderTarget,
    write: IDWriteFactory,
    icon: IDWriteTextFormat,
    text: IDWriteTextFormat,
}

impl Drop for Look {
    fn drop(&mut self) {
        let _ = unsafe { DeleteObject(self.back.into()) };
    }
}

thread_local! {
    static LOOK: RefCell<Option<Look>> = const { RefCell::new(None) };
}

/// Until dropped, the menu is drawn here.
pub struct Painting;

impl Drop for Painting {
    fn drop(&mut self) {
        LOOK.replace(None);
    }
}

/// Takes over the painting of `m` and its submenu.
pub fn begin(m: HMENU, name: &str, at: POINT) -> Option<Painting> {
    let (mut dpi, mut other) = (96, 0);
    unsafe {
        let monitor = MonitorFromPoint(at, MONITOR_DEFAULTTONEAREST);
        let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut other);
    }
    let look = Look::new(palette(name), dpi).ok()?;
    let info = MENUINFO {
        cbSize: size_of::<MENUINFO>() as u32,
        fMask: MIM_BACKGROUND | MIM_APPLYTOSUBMENUS,
        hbrBack: look.back,
        ..Default::default()
    };
    unsafe { SetMenuInfo(m, &info) }.ok()?;
    LOOK.replace(Some(look));
    Some(Painting)
}

/// Marks an item to be drawn here. Its text stays for screen readers.
pub fn own(m: HMENU, index: u32, data: usize) -> Result<()> {
    let mut info = MENUITEMINFOW {
        cbSize: size_of::<MENUITEMINFOW>() as u32,
        fMask: MIIM_FTYPE,
        ..Default::default()
    };
    unsafe {
        GetMenuItemInfoW(m, index, true, &mut info)?;
        info.fMask |= MIIM_DATA;
        info.fType |= MFT_OWNERDRAW;
        info.dwItemData = data;
        SetMenuItemInfoW(m, index, true, &info)
    }
}

/// The text with its access key, and the icon.
fn row(data: usize) -> (&'static str, char) {
    match data as u32 {
        OPEN => ("&Open wheel", '\u{EA3A}'),
        SETTINGS => ("&Settings", '\u{E713}'),
        PAUSE => ("&Pause", '\u{E769}'),
        EXIT => ("E&xit", '\u{E7E8}'),
        _ if data == RESUME => ("&Resume", '\u{E768}'),
        _ if data == THEMES_MENU => ("&Theme", '\u{E790}'),
        id => {
            let i = id.wrapping_sub(THEME) as usize;
            (THEMES.get(i).copied().unwrap_or(""), ' ')
        }
    }
}

/// The text without its `&`, and where the access key is in it.
fn plain(text: &str) -> (String, Option<u32>) {
    (text.replacen('&', "", 1), text.find('&').map(|i| i as u32))
}

impl Look {
    fn new(p: Palette, dpi: u32) -> Result<Self> {
        unsafe {
            let d2d: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let write: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let props = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_IGNORE,
                },
                dpiX: dpi as f32,
                dpiY: dpi as f32,
                ..Default::default()
            };
            let format = |family, size| -> Result<IDWriteTextFormat> {
                let f = write.CreateTextFormat(
                    family,
                    None,
                    DWRITE_FONT_WEIGHT_REGULAR,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    size,
                    w!("en-us"),
                )?;
                f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
                f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
                Ok(f)
            };
            let icon = format(w!("Segoe Fluent Icons"), 16.0)?;
            icon.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            Ok(Self {
                rt: d2d.CreateDCRenderTarget(&props)?,
                text: format(w!("Segoe UI Variable Text"), 14.0)?,
                icon,
                write,
                back: CreateSolidBrush(rgb(p.plate)),
                p,
                dpi,
            })
        }
    }

    fn px(&self, v: f32) -> u32 {
        (v * self.dpi as f32 / 96.0).round() as u32
    }

    fn item(&self, d: &DRAWITEMSTRUCT) -> Result<()> {
        let p = self.p;
        let data = d.itemData;
        let chosen = d.itemState.0 & ODS_SELECTED.0 != 0;
        let (fill, ink) = if chosen {
            (p.accent, p.on_accent)
        } else {
            (p.plate, p.text)
        };
        let dip = |v: i32| v as f32 * 96.0 / self.dpi as f32;
        let (w, h) = (
            dip(d.rcItem.right - d.rcItem.left),
            dip(d.rcItem.bottom - d.rcItem.top),
        );
        let rt = &self.rt;
        unsafe {
            rt.BindDC(d.hDC, &d.rcItem)?;
            let brush = |c| rt.CreateSolidColorBrush(&color(c), None);
            let ink_brush = brush(ink)?;
            rt.BeginDraw();
            rt.Clear(Some(&color(p.plate)));
            if data == SEPARATOR {
                let line = rect(SIDE + 8.0, h / 2.0 - 0.5, w - SIDE - 8.0, h / 2.0 + 0.5);
                rt.FillRectangle(&line, &brush(mix(p.text, p.plate, 0.2))?);
                return rt.EndDraw(None, None);
            }
            if chosen {
                let pill = D2D1_ROUNDED_RECT {
                    rect: rect(SIDE, 1.0, w - SIDE, h - 1.0),
                    radiusX: 5.0,
                    radiusY: 5.0,
                };
                rt.FillRoundedRectangle(&pill, &brush(fill)?);
            }
            let (text, glyph) = row(data);
            let middle = h / 2.0;
            let icon = |x: f32, glyph: char| {
                let mut buf = [0u16; 2];
                rt.DrawText(
                    glyph.encode_utf16(&mut buf),
                    &self.icon,
                    &rect(x - 12.0, 0.0, x + 12.0, h),
                    &ink_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                    DWRITE_MEASURING_MODE_NATURAL,
                );
            };
            if data >= THEME as usize {
                // What the theme looks like: its slot color around its accent.
                let t = theme(THEMES[(data - THEME as usize).min(THEMES.len() - 1)]);
                let mut disc = D2D1_ELLIPSE::default();
                disc.point.X = ICON;
                disc.point.Y = middle;
                for (r, c) in [(8.0, ink), (7.0, t.slot), (4.0, t.accent)] {
                    (disc.radiusX, disc.radiusY) = (r, r);
                    rt.FillEllipse(&disc, &brush(c)?);
                }
            } else {
                icon(ICON, glyph);
            }
            if d.itemState.0 & ODS_CHECKED.0 != 0 {
                icon(w - 22.0, '\u{E73E}');
            }
            if data == THEMES_MENU {
                icon(w - 22.0, '\u{E76C}');
            }
            let (text, key) = plain(text);
            let wide: Vec<u16> = text.encode_utf16().collect();
            let line = self
                .write
                .CreateTextLayout(&wide, &self.text, w - TEXT, h)?;
            if let Some(key) = key.filter(|_| d.itemState.0 & ODS_NOACCEL.0 == 0) {
                let range = DWRITE_TEXT_RANGE {
                    startPosition: key,
                    length: 1,
                };
                line.SetUnderline(true, range)?;
            }
            let mut origin = D2D1_ELLIPSE::default().point;
            origin.X = TEXT;
            rt.DrawTextLayout(origin, &line, &ink_brush, D2D1_DRAW_TEXT_OPTIONS_CLIP);
            rt.EndDraw(None, None)
        }
    }
}

fn rect(left: f32, top: f32, right: f32, bottom: f32) -> D2D_RECT_F {
    D2D_RECT_F {
        left,
        top,
        right,
        bottom,
    }
}

fn rgb(c: u32) -> COLORREF {
    COLORREF((c & 255) << 16 | c & 0xFF00 | c >> 16 & 255)
}

/// The messages with which Windows asks the menu's owner to draw it.
pub fn handle(msg: u32, w: WPARAM, l: LPARAM) -> Option<LRESULT> {
    if !matches!(msg, WM_MEASUREITEM | WM_DRAWITEM | WM_MENUCHAR) {
        return None;
    }
    LOOK.with_borrow(|look| {
        let look = look.as_ref()?;
        match msg {
            WM_MEASUREITEM => {
                let m = unsafe { &mut *(l.0 as *mut MEASUREITEMSTRUCT) };
                if m.CtlType != ODT_MENU {
                    return None;
                }
                // Windows adds the room of a check mark.
                let check = unsafe { GetSystemMetrics(SM_CXMENUCHECK) }.max(0) as u32;
                m.itemWidth = look.px(WIDTH).saturating_sub(check);
                let line = m.itemData == SEPARATOR;
                m.itemHeight = look.px(if line { LINE } else { HEIGHT });
                Some(LRESULT(1))
            }
            WM_DRAWITEM => {
                let d = unsafe { &*(l.0 as *const DRAWITEMSTRUCT) };
                if d.CtlType != ODT_MENU {
                    return None;
                }
                dress(look);
                let _ = look.item(d);
                let r = d.rcItem;
                // Else Windows draws its own arrow over ours.
                unsafe { ExcludeClipRect(d.hDC, r.left, r.top, r.right, r.bottom) };
                Some(LRESULT(1))
            }
            _ => {
                let m = HMENU(l.0 as *mut core::ffi::c_void);
                let typed = char::from_u32(w.0 as u32 & 0xFFFF)?;
                let found = key_of(m, typed)?;
                Some(LRESULT((MNC_EXECUTE as isize) << 16 | found as isize))
            }
        }
    })
}

/// The position of the item whose access key is `typed`.
fn key_of(m: HMENU, typed: char) -> Option<u32> {
    let count = unsafe { GetMenuItemCount(Some(m)) }.max(0) as u32;
    (0..count).find(|&i| {
        let mut info = MENUITEMINFOW {
            cbSize: size_of::<MENUITEMINFOW>() as u32,
            fMask: MIIM_DATA,
            ..Default::default()
        };
        unsafe { GetMenuItemInfoW(m, i, true, &mut info) }.is_ok() && is_key(info.dwItemData, typed)
    })
}

fn is_key(data: usize, typed: char) -> bool {
    let (text, _) = row(data);
    let key = text
        .split_once('&')
        .and_then(|(_, rest)| rest.chars().next());
    key.is_some_and(|k| k.eq_ignore_ascii_case(&typed))
}

/// Gives every menu window that has none yet our edge and round corners.
fn dress(look: &Look) {
    let mut after = None;
    while let Ok(h) = unsafe { FindWindowExW(None, after, w!("#32768"), None) } {
        after = Some(h);
        unsafe {
            if GetWindowThreadProcessId(h, None) != GetCurrentThreadId()
                || GetWindowSubclass(h, Some(edge_proc), SUBCLASS, None).as_bool()
                || !SetWindowSubclass(h, Some(edge_proc), SUBCLASS, 0).as_bool()
            {
                continue;
            }
            let set = |what: DWMWINDOWATTRIBUTE, value: u32| {
                let size = size_of::<u32>() as u32;
                let _ = DwmSetWindowAttribute(h, what, (&raw const value).cast(), size);
            };
            set(DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND.0 as u32);
            set(
                DWMWA_BORDER_COLOR,
                rgb(mix(look.p.text, look.p.plate, 0.25)).0,
            );
            let flags = RDW_FRAME | RDW_INVALIDATE;
            let _ = RedrawWindow(Some(h), None, None, flags);
        }
    }
}

/// The frame of a menu window, in the color of the menu.
unsafe extern "system" fn edge_proc(
    h: HWND,
    msg: u32,
    w: WPARAM,
    l: LPARAM,
    id: usize,
    _: usize,
) -> LRESULT {
    unsafe {
        if msg == WM_NCDESTROY {
            let _ = RemoveWindowSubclass(h, Some(edge_proc), id);
        }
        let done = DefSubclassProc(h, msg, w, l);
        if msg == WM_NCPAINT {
            LOOK.with_borrow(|look| {
                let Some(look) = look else { return };
                let (mut own, mut client) = (RECT::default(), RECT::default());
                let mut at = POINT::default();
                let _ = GetWindowRect(h, &mut own);
                let _ = GetClientRect(h, &mut client);
                let _ = ClientToScreen(h, &mut at);
                let (x, y) = (at.x - own.left, at.y - own.top);
                let dc = GetWindowDC(Some(h));
                ExcludeClipRect(dc, x, y, x + client.right, y + client.bottom);
                let all = RECT {
                    left: 0,
                    top: 0,
                    right: own.right - own.left,
                    bottom: own.bottom - own.top,
                };
                FillRect(dc, &all, look.back);
                ReleaseDC(Some(h), dc);
            });
        }
        done
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_has_a_row() {
        for data in [1, 2, 3, 4, RESUME, THEMES_MENU] {
            let (text, glyph) = row(data);
            assert!(text.contains('&') && glyph != ' ', "{data}");
        }
        for (i, name) in THEMES.iter().enumerate() {
            assert_eq!(row(THEME as usize + i).0, *name);
        }
        assert_eq!(row(99).0, "");
        assert_eq!(plain("E&xit"), ("Exit".to_string(), Some(1)));
        assert_eq!(plain("Aqua"), ("Aqua".to_string(), None));
    }

    #[test]
    fn access_keys_differ() {
        let root = [1, 2, 3, THEMES_MENU, 4];
        for (key, data) in "ospTx".chars().zip(root) {
            assert_eq!(root.iter().filter(|&&d| is_key(d, key)).count(), 1);
            assert!(is_key(data, key));
        }
        assert!(is_key(RESUME, 'r') && !is_key(SEPARATOR, 'o'));
    }
}
