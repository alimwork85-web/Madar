//! The system file dialog, in a child process: the shell libraries it loads never unload.

use std::cell::Cell;
use std::io::{Read, Write};
use std::os::windows::io::AsRawHandle;
use std::process::{Command, Stdio};
use windows::Win32::Foundation::*;
use windows::Win32::System::Threading::{INFINITE, TerminateProcess};
use windows::Win32::UI::Controls::Dialogs::*;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetFocus};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PWSTR, w};

/// Argument of the child process that shows the dialog.
pub const CHILD: &str = "--browse";
const MAX_BYTES: usize = 4096;

thread_local! { static BUSY: Cell<bool> = const { Cell::new(false) }; }

/// Clears the guard on every path out of `pick`.
struct Busy;
impl Drop for Busy {
    fn drop(&mut self) {
        BUSY.with(|b| b.set(false));
    }
}

/// Modal, the same dialog as before, in a child that exits with it. None on cancel.
pub fn pick(owner: HWND, site: bool) -> Option<String> {
    // The owner is disabled while the dialog is open, so this is a backstop.
    if BUSY.with(|b| b.replace(true)) {
        return None;
    }
    let _busy = Busy;
    open(owner, site)
}

fn open(owner: HWND, site: bool) -> Option<String> {
    let mut child = Command::new(std::env::current_exe().ok()?)
        .args([CHILD, if site { "site" } else { "app" }])
        .arg((owner.0 as usize).to_string())
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .ok()?;
    let handle = HANDLE(child.as_raw_handle());
    let _ = unsafe { AllowSetForegroundWindow(child.id()) };
    let _ = unsafe { EnableWindow(owner, false) };
    let ended = wait(handle, owner);
    if !ended {
        let _ = unsafe { TerminateProcess(handle, 1) };
    }
    let _ = child.wait();
    // The owner is usable again before this returns.
    if unsafe { IsWindow(Some(owner)) }.as_bool() {
        let _ = unsafe { EnableWindow(owner, true) };
        let _ = unsafe { SetFocus(Some(owner)) };
    }
    if !ended {
        return None;
    }
    let mut bytes = Vec::new();
    child.stdout.take()?.read_to_end(&mut bytes).ok()?;
    parse(&bytes)
}

/// Pumps this thread's messages until the child ends. True = it ended itself.
fn wait(handle: HANDLE, owner: HWND) -> bool {
    let mut msg = MSG::default();
    while unsafe { IsWindow(Some(owner)) }.as_bool() {
        // WAIT_OBJECT_0 is the child, WAIT_OBJECT_0 + 1 is our input.
        let got = unsafe {
            MsgWaitForMultipleObjectsEx(Some(&[handle]), INFINITE, QS_ALLINPUT, MWMO_INPUTAVAILABLE)
        };
        if got.0 == 0 {
            return true;
        }
        if got.0 != 1 {
            return false;
        }
        while unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
            if msg.message == WM_QUIT {
                // The outer loop must see it again.
                unsafe { PostQuitMessage(msg.wParam.0 as i32) };
                return false;
            }
            let _ = unsafe { TranslateMessage(&msg) };
            unsafe { DispatchMessageW(&msg) };
        }
    }
    false
}

/// The child's work: the dialog, then the path on stdout with no newline.
pub fn child(mut args: impl Iterator<Item = String>) -> Option<()> {
    let (site, owner) = arguments(args.next(), args.next())?;
    // Before any window.
    let _ = unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    if !unsafe { IsWindow(Some(owner)) }.as_bool() {
        return None;
    }
    let (filter, title) = if site {
        (w!("Programs\0*.exe\0\0"), w!("Choose a browser"))
    } else {
        (w!("Programs\0*.exe;*.lnk\0\0"), w!("Choose a program"))
    };
    let mut buf = vec![0u16; 1024];
    let mut ofn = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: owner,
        lpstrFilter: filter,
        lpstrFile: PWSTR(buf.as_mut_ptr()),
        nMaxFile: buf.len() as u32,
        lpstrTitle: title,
        Flags: OFN_FILEMUSTEXIST
            | OFN_PATHMUSTEXIST
            | OFN_NOCHANGEDIR
            | OFN_NODEREFERENCELINKS
            | OFN_EXPLORER
            | OFN_HIDEREADONLY,
        ..Default::default()
    };
    if !unsafe { GetOpenFileNameW(&mut ofn) }.as_bool() {
        return None;
    }
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(String::from_utf16_lossy(&buf[..end]).as_bytes());
    let _ = out.flush();
    Some(())
}

/// Kind and owner from the arguments after `--browse`. Pure.
fn arguments(kind: Option<String>, owner: Option<String>) -> Option<(bool, HWND)> {
    let site = match kind.as_deref()? {
        "site" => true,
        "app" => false,
        _ => return None,
    };
    let handle = owner?.parse::<usize>().ok().filter(|&n| n != 0)?;
    Some((site, HWND(handle as *mut core::ffi::c_void)))
}

/// The child's whole answer. Pure.
fn parse(bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?;
    (bytes.len() <= MAX_BYTES && !text.is_empty() && !text.contains('\0')).then(|| text.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_arguments_and_the_answer() {
        let hwnd = HWND(4660 as *mut core::ffi::c_void);
        let args = |kind: &str, owner: &str| arguments(Some(kind.into()), Some(owner.into()));
        assert_eq!(args("app", "4660"), Some((false, hwnd)));
        assert_eq!(args("site", "4660"), Some((true, hwnd)));
        assert_eq!(arguments(None, Some("1".into())), None);
        assert_eq!(arguments(Some("app".into()), None), None);
        for (kind, owner) in [("apps", "1"), ("", "1"), ("app", "0"), ("app", "-8")] {
            assert_eq!(args(kind, owner), None);
        }
    }

    #[test]
    fn reads_the_childs_answer() {
        let path = "C:\\مستندات\\متصفح.exe";
        assert_eq!(parse(path.as_bytes()).as_deref(), Some(path));
        assert_eq!(parse(&[b'a'; MAX_BYTES]).map(|s| s.len()), Some(MAX_BYTES));
        let bad: [&[u8]; 4] = [&[], b"C:\\a\0b", b"\xff\xfe", &[b'a'; MAX_BYTES + 1]];
        for bytes in bad {
            assert_eq!(parse(bytes), None);
        }
    }
}
