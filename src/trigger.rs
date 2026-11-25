//! Ways to summon the wheel: global shortcut parsing and the mouse hook.

use crate::config::Mouse;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU8, Ordering::Relaxed};
use std::sync::mpsc;
use std::thread::JoinHandle;
use windows::Win32::Foundation::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

/// Summon button pressed, moved while held, released. LPARAM is a screen point.
pub const WM_SUMMON_DOWN: u32 = WM_APP + 1;
pub const WM_SUMMON_MOVE: u32 = WM_APP + 2;
pub const WM_SUMMON_UP: u32 = WM_APP + 3;
/// Any other button went down while the wheel is visible. LPARAM is a screen point.
pub const WM_PRESS: u32 = WM_APP + 4;

/// Parses `Ctrl+Alt+Space` style text. Needs Ctrl, Alt or Win, except for F1-F24.
pub fn parse_hotkey(text: &str) -> Option<(HOT_KEY_MODIFIERS, u32)> {
    let mut mods = HOT_KEY_MODIFIERS(0);
    let mut key = None;
    for part in text.split('+') {
        let part = part.trim().to_ascii_lowercase();
        let bit = match part.as_str() {
            "ctrl" => Some(MOD_CONTROL),
            "alt" => Some(MOD_ALT),
            "shift" => Some(MOD_SHIFT),
            "win" => Some(MOD_WIN),
            _ => None,
        };
        match bit {
            Some(b) if mods.contains(b) => return None,
            Some(b) => mods |= b,
            None if key.is_some() => return None,
            None => key = Some(vk(&part)?),
        }
    }
    let key = key?;
    let function_key = (0x70..=0x87).contains(&key);
    if !function_key && (mods.0 & (MOD_CONTROL | MOD_ALT | MOD_WIN).0) == 0 {
        return None;
    }
    Some((mods, key))
}

fn vk(name: &str) -> Option<u32> {
    let named = match name {
        "space" => VK_SPACE,
        "tab" => VK_TAB,
        "enter" => VK_RETURN,
        "insert" => VK_INSERT,
        "home" => VK_HOME,
        "end" => VK_END,
        "pageup" => VK_PRIOR,
        "pagedown" => VK_NEXT,
        "pause" => VK_PAUSE,
        _ => {
            if let [c] = name.as_bytes() {
                return c
                    .is_ascii_alphanumeric()
                    .then(|| c.to_ascii_uppercase() as u32);
            }
            let n: u32 = name.strip_prefix('f')?.parse().ok()?;
            return (1..=24).contains(&n).then(|| 0x6F + n);
        }
    };
    Some(named.0 as u32)
}

pub fn pack(p: POINT) -> LPARAM {
    LPARAM(((p.y as isize) << 32) | (p.x as u32 as isize))
}

pub fn unpack(l: LPARAM) -> POINT {
    POINT {
        x: l.0 as i32,
        y: (l.0 >> 32) as i32,
    }
}

static TARGET: AtomicIsize = AtomicIsize::new(0);
static BUTTON: AtomicU8 = AtomicU8::new(0);
static WATCH_PRESSES: AtomicBool = AtomicBool::new(false);
/// Summon button whose up event must be swallowed too; 0 when none.
static HELD: AtomicU8 = AtomicU8::new(0);

/// The hook has its own thread so that drawing never delays the system pointer.
pub struct MouseHook {
    thread: Option<(u32, JoinHandle<()>)>,
}

impl MouseHook {
    pub fn new(target: HWND) -> Self {
        TARGET.store(target.0 as isize, Relaxed);
        Self { thread: None }
    }

    /// Runs the hook only while a summon button is set or the wheel is visible.
    pub fn update(&mut self, button: Mouse, wheel_visible: bool) -> Result<(), String> {
        BUTTON.store(button as u8, Relaxed);
        WATCH_PRESSES.store(wheel_visible, Relaxed);
        // A held button keeps the hook alive until its up event is swallowed.
        let needed = button != Mouse::None || wheel_visible || HELD.load(Relaxed) != 0;
        if needed && self.thread.is_none() {
            let (tx, rx) = mpsc::channel();
            let handle = std::thread::spawn(move || run(&tx));
            match rx.recv() {
                Ok(Ok(id)) => self.thread = Some((id, handle)),
                Ok(Err(e)) => return Err(format!("The mouse button is unavailable: {e}")),
                Err(_) => return Err("The mouse button is unavailable.".into()),
            }
        } else if !needed {
            self.stop();
        }
        Ok(())
    }

    fn stop(&mut self) {
        if let Some((id, handle)) = self.thread.take() {
            let posted = unsafe { PostThreadMessageW(id, WM_QUIT, WPARAM(0), LPARAM(0)) };
            if posted.is_ok() {
                let _ = handle.join();
            }
        }
    }
}

impl Drop for MouseHook {
    fn drop(&mut self) {
        self.stop();
    }
}

fn run(tx: &mpsc::Sender<windows::core::Result<u32>>) {
    unsafe {
        let mut msg = MSG::default();
        // Creates the message queue before anyone can post WM_QUIT.
        let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
        let module = GetModuleHandleW(None).map(HINSTANCE::from).ok();
        match SetWindowsHookExW(WH_MOUSE_LL, Some(hook), module, 0) {
            Ok(h) => {
                let _ = tx.send(Ok(GetCurrentThreadId()));
                while GetMessageW(&mut msg, None, 0, 0).0 > 0 {}
                let _ = UnhookWindowsHookEx(h);
                HELD.store(0, Relaxed);
            }
            Err(e) => {
                let _ = tx.send(Err(e));
            }
        }
    }
}

/// Which summon button, as `Mouse as u8`, a button message belongs to; 0 for others.
fn button_of(msg: u32, data: u32) -> u8 {
    match msg {
        WM_XBUTTONDOWN | WM_XBUTTONUP if data >> 16 == XBUTTON1 as u32 => Mouse::Back as u8,
        WM_XBUTTONDOWN | WM_XBUTTONUP => Mouse::Forward as u8,
        WM_MBUTTONDOWN | WM_MBUTTONUP => Mouse::CtrlMiddle as u8,
        _ => 0,
    }
}

unsafe extern "system" fn hook(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        if code == HC_ACTION as i32 {
            let info = &*(l.0 as *const MSLLHOOKSTRUCT);
            let post = |m| {
                let hwnd = HWND(TARGET.load(Relaxed) as *mut _);
                let _ = PostMessageW(Some(hwnd), m, WPARAM(0), pack(info.pt));
            };
            let msg = w.0 as u32;
            let held = HELD.load(Relaxed);
            let button = button_of(msg, info.mouseData);
            match msg {
                WM_MOUSEMOVE if held != 0 => post(WM_SUMMON_MOVE),
                WM_XBUTTONDOWN | WM_MBUTTONDOWN | WM_LBUTTONDOWN | WM_RBUTTONDOWN => {
                    let chosen = button != 0 && button == BUTTON.load(Relaxed);
                    let ctrl = || GetAsyncKeyState(VK_CONTROL.0 as i32) < 0;
                    if chosen && (button != Mouse::CtrlMiddle as u8 || ctrl()) {
                        HELD.store(button, Relaxed);
                        post(WM_SUMMON_DOWN);
                        return LRESULT(1);
                    }
                    if WATCH_PRESSES.load(Relaxed) {
                        post(WM_PRESS);
                    }
                }
                WM_XBUTTONUP | WM_MBUTTONUP if held != 0 && button == held => {
                    HELD.store(0, Relaxed);
                    post(WM_SUMMON_UP);
                    return LRESULT(1);
                }
                _ => {}
            }
        }
        CallNextHookEx(None, code, w, l)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hotkeys() {
        let ctrl_alt = MOD_CONTROL | MOD_ALT;
        assert_eq!(parse_hotkey("Ctrl+Alt+Space"), Some((ctrl_alt, 0x20)));
        assert_eq!(
            parse_hotkey(" win + shift + a "),
            Some((MOD_WIN | MOD_SHIFT, 0x41))
        );
        assert_eq!(parse_hotkey("F13"), Some((HOT_KEY_MODIFIERS(0), 0x7C)));
        for bad in [
            "",
            "A",
            "Shift+A",
            "Ctrl",
            "Ctrl+A+B",
            "Ctrl+Ctrl+A",
            "Ctrl+F25",
            "Ctrl+ي",
            "Ctrl+",
        ] {
            assert_eq!(parse_hotkey(bad), None, "{bad}");
        }
    }

    #[test]
    fn points_survive_packing() {
        for (x, y) in [(0, 0), (-1920, 5), (7, -1080), (-1, -1)] {
            let p = POINT { x, y };
            assert_eq!(unpack(pack(p)), p);
        }
    }

    #[test]
    fn buttons() {
        assert_eq!(button_of(WM_XBUTTONUP, 1 << 16), Mouse::Back as u8);
        assert_eq!(button_of(WM_XBUTTONDOWN, 2 << 16), Mouse::Forward as u8);
        assert_eq!(button_of(WM_LBUTTONDOWN, 0), Mouse::None as u8);
    }
}
