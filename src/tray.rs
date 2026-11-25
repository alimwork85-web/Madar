//! The tray icon: its clicks, its tooltip and its right-click menu.
//!
//! Nothing here is global: a `Tray` owns the icon it registered and removes it
//! when dropped, so Explorer restarts are handled by calling `add` again.

pub mod look;

use crate::config::THEMES;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{LIM_SMALL, LoadIconMetric};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION,
    NIN_SELECT, NINF_KEY, NOTIFY_ICON_DATA_FLAGS, NOTIFYICON_VERSION_4, NOTIFYICONDATAW,
    Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, PCWSTR, w};

/// Explorer's callback message for the icon.
pub const WM_TRAY: u32 = WM_APP + 20;

/// Absent from this version of the crate, though it is in the header.
const NIN_KEYSELECT: u32 = NIN_SELECT | NINF_KEY;
/// One icon, one id.
const ID: u32 = 1;
/// Icon resource 1, by number rather than by name.
const ICON: PCWSTR = PCWSTR(core::ptr::without_provenance(1));
const OPEN: u32 = 1;
const SETTINGS: u32 = 2;
const PAUSE: u32 = 3;
const EXIT: u32 = 4;
/// The first of one id per theme, in the order of `THEMES`.
const THEME: u32 = 10;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    Open,
    /// Right click, at a screen point.
    Menu(POINT),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
    OpenWheel,
    Settings,
    TogglePause,
    /// Index in `THEMES`.
    Theme(usize),
    Exit,
}

pub struct Tray {
    hwnd: HWND,
    paused: bool,
    icon: HICON,
    /// `IDI_APPLICATION` belongs to the system and must not be destroyed.
    owned: bool,
}

impl Tray {
    pub fn new(hwnd: HWND) -> Self {
        unsafe {
            let module = GetModuleHandleW(None).map(HINSTANCE::from).ok();
            let (icon, owned) = match LoadIconMetric(module, ICON, LIM_SMALL) {
                Ok(icon) => (icon, true),
                Err(_) => (LoadIconW(None, IDI_APPLICATION).unwrap_or_default(), false),
            };
            Self {
                hwnd,
                paused: false,
                icon,
                owned,
            }
        }
    }

    /// Puts the icon in the tray. Again after `taskbar_created` fires.
    pub fn add(&mut self) -> bool {
        let mut data = self.data(NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP);
        let added = unsafe { Shell_NotifyIconW(NIM_ADD, &data) }.as_bool();
        if added {
            data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
            let _ = unsafe { Shell_NotifyIconW(NIM_SETVERSION, &data) };
        }
        added
    }

    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
        let data = self.data(NIF_TIP | NIF_SHOWTIP);
        let _ = unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) };
    }

    fn data(&self, flags: NOTIFY_ICON_DATA_FLAGS) -> NOTIFYICONDATAW {
        let mut data = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: ID,
            uFlags: flags,
            uCallbackMessage: WM_TRAY,
            hIcon: self.icon,
            ..Default::default()
        };
        let text = if self.paused {
            "Madar (paused)"
        } else {
            "Madar"
        };
        tip(&mut data.szTip, text);
        data
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        let data = self.data(NOTIFY_ICON_DATA_FLAGS(0));
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &data);
            if self.owned {
                let _ = DestroyIcon(self.icon);
            }
        }
    }
}

/// The message Explorer broadcasts when it has restarted.
pub fn taskbar_created() -> u32 {
    unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) }
}

/// Version 4 packing: the event is the low word of `l`, the point is `w`.
pub fn event(w: WPARAM, l: LPARAM) -> Option<Event> {
    match (l.0 as u32) & 0xFFFF {
        NIN_SELECT | NIN_KEYSELECT => Some(Event::Open),
        WM_CONTEXTMENU => Some(Event::Menu(POINT {
            x: signed(w.0),
            y: signed(w.0 >> 16),
        })),
        _ => None,
    }
}

/// A signed 16-bit coordinate out of a packed word.
fn signed(word: usize) -> i32 {
    (word & 0xFFFF) as u16 as i16 as i32
}

/// Shows the menu at `at` and waits. `None` when dismissed.
pub fn menu(hwnd: HWND, paused: bool, theme: &str, at: POINT) -> Option<Command> {
    unsafe {
        let m = CreatePopupMenu().ok()?;
        let chosen = track(m, hwnd, paused, theme, at);
        let _ = DestroyMenu(m);
        chosen
    }
}

unsafe fn track(m: HMENU, hwnd: HWND, paused: bool, theme: &str, at: POINT) -> Option<Command> {
    unsafe {
        let items = [
            (OPEN, w!("&Open wheel")),
            (SETTINGS, w!("&Settings")),
            (PAUSE, if paused { w!("&Resume") } else { w!("&Pause") }),
        ];
        for (id, text) in items {
            AppendMenuW(m, MF_STRING, id as usize, text).ok()?;
        }
        // Attached at once, so that destroying `m` destroys it too.
        let themes = CreatePopupMenu().ok()?;
        AppendMenuW(m, MF_POPUP, themes.0 as usize, w!("&Theme")).ok()?;
        for (id, name) in (THEME..).zip(THEMES) {
            AppendMenuW(themes, MF_STRING, id as usize, &HSTRING::from(name)).ok()?;
            if name == theme {
                let only = MF_BYCOMMAND.0;
                CheckMenuRadioItem(themes, id, id, id, only).ok()?;
            }
        }
        AppendMenuW(m, MF_SEPARATOR, 0, PCWSTR::null()).ok()?;
        AppendMenuW(m, MF_STRING, EXIT as usize, w!("E&xit")).ok()?;
        SetMenuDefaultItem(m, OPEN, 0).ok()?;
        // Marked in the order added above. Kept until the menu is gone.
        let _painting = look::begin(m, theme, at).inspect(|_| {
            let pause = if paused { look::RESUME } else { PAUSE as usize };
            let root = [
                OPEN as usize,
                SETTINGS as usize,
                pause,
                look::THEMES_MENU,
                look::SEPARATOR,
                EXIT as usize,
            ];
            for (i, data) in (0..).zip(root) {
                let _ = look::own(m, i, data);
            }
            for i in 0..THEMES.len() as u32 {
                let _ = look::own(themes, i, (THEME + i) as usize);
            }
        });
        let _ = SetForegroundWindow(hwnd);
        let align = if GetSystemMetrics(SM_MENUDROPALIGNMENT) != 0 {
            TPM_RIGHTALIGN
        } else {
            TPM_LEFTALIGN
        };
        // Without the fade, which would show the menu before its edge is drawn.
        let flags = TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON | TPM_NOANIMATION | align;
        let id = TrackPopupMenuEx(m, flags.0, at.x, at.y, hwnd, None);
        // Lets the menu close properly when the click is outside it.
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        command(id.0 as u32)
    }
}

fn command(id: u32) -> Option<Command> {
    Some(match id {
        OPEN => Command::OpenWheel,
        SETTINGS => Command::Settings,
        PAUSE => Command::TogglePause,
        EXIT => Command::Exit,
        _ => {
            let i = id.checked_sub(THEME)? as usize;
            THEMES.get(i)?;
            Command::Theme(i)
        }
    })
}

/// Writes `text` into a fixed tip buffer, NUL terminated, cutting on whole chars.
fn tip(buf: &mut [u16], text: &str) {
    let Some(last) = buf.len().checked_sub(1) else {
        return;
    };
    let mut n = 0;
    for ch in text.chars() {
        let mut wide = [0u16; 2];
        let units = ch.encode_utf16(&mut wide);
        if n + units.len() > last {
            break;
        }
        buf[n..n + units.len()].copy_from_slice(units);
        n += units.len();
    }
    buf[n] = 0;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packed(x: i32, y: i32) -> WPARAM {
        WPARAM((x as i16 as u16 as usize) | ((y as i16 as u16 as usize) << 16))
    }

    #[test]
    fn events() {
        let at = packed(12, -4);
        assert_eq!(event(at, LPARAM(NIN_SELECT as isize)), Some(Event::Open));
        assert_eq!(event(at, LPARAM(NIN_KEYSELECT as isize)), Some(Event::Open));
        assert_eq!(
            event(at, LPARAM(WM_CONTEXTMENU as isize)),
            Some(Event::Menu(POINT { x: 12, y: -4 }))
        );
        // Only the low word of l is the event.
        let spiked = LPARAM(((0xABCD << 16) | NIN_SELECT) as isize);
        assert_eq!(event(at, spiked), Some(Event::Open));
    }

    #[test]
    fn negative_points() {
        assert_eq!(
            event(packed(-1920, -1080), LPARAM(WM_CONTEXTMENU as isize)),
            Some(Event::Menu(POINT { x: -1920, y: -1080 }))
        );
        assert_eq!(
            event(packed(-1, -1), LPARAM(WM_CONTEXTMENU as isize)),
            Some(Event::Menu(POINT { x: -1, y: -1 }))
        );
    }

    #[test]
    fn other_events_are_ignored() {
        let at = packed(3, 3);
        for other in [
            0,
            WM_LBUTTONUP,
            WM_RBUTTONUP,
            WM_MOUSEMOVE,
            1026, // NIN_SELECTSUBTEXT, in the header but not in the crate
            0xFFFF,
        ] {
            assert_eq!(event(at, LPARAM(other as isize)), None, "{other}");
        }
    }

    #[test]
    fn commands() {
        assert_eq!(command(OPEN), Some(Command::OpenWheel));
        assert_eq!(command(SETTINGS), Some(Command::Settings));
        assert_eq!(command(PAUSE), Some(Command::TogglePause));
        assert_eq!(command(EXIT), Some(Command::Exit));
        for (i, id) in (THEME..).take(THEMES.len()).enumerate() {
            assert_eq!(command(id), Some(Command::Theme(i)));
        }
        for bad in [0, 5, THEME - 1, THEME + THEMES.len() as u32, 99, 0xFFFF] {
            assert_eq!(command(bad), None, "{bad}");
        }
    }

    #[test]
    fn tips_are_short_and_terminated() {
        let mut buf = [7u16; 128];
        tip(&mut buf, "Madar (paused)");
        let end = buf.iter().position(|&c| c == 0).expect("NUL");
        assert_eq!(String::from_utf16_lossy(&buf[..end]), "Madar (paused)");

        let mut small = [0u16; 5];
        tip(&mut small, "Madar (paused)");
        assert_eq!(&small, &[77, 97, 100, 97, 0]); // "Mada"

        // A surrogate pair that would be cut in half is dropped whole.
        let mut pair = [0u16; 4];
        tip(&mut pair, "\u{1F600}\u{1F600}");
        assert_eq!(&pair[..3], &[0xD83D, 0xDE00, 0]);

        let mut empty = [1u16; 1];
        tip(&mut empty, "Madar");
        assert_eq!(empty, [0]);
    }
}
