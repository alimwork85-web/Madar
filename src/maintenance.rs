//! Restarting Explorer. It waits, so it is not for the wheel's thread.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::*;
use windows::Win32::System::RestartManager::*;
use windows::Win32::System::SystemInformation::GetWindowsDirectoryW;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::WindowsAndMessaging::{GetShellWindow, GetWindowThreadProcessId};
use windows::core::PWSTR;

pub const CONFIRM: &str = "Folders will close. Press again";
pub const WORKING: &str = "Restarting Explorer...";
/// Repeats of a held key and double clicks are not a second press.
const LOCK: Duration = Duration::from_secs(1);
const SHELL_TIME: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(50);

static BUSY: AtomicBool = AtomicBool::new(false);

/// Whether a press at `now` confirms the one at `last`.
pub fn confirmed(last: Option<Instant>, now: Instant) -> bool {
    last.is_some_and(|t| now.duration_since(t) >= LOCK)
}

/// Ending the program between the two halves would leave no desktop.
pub fn wait() {
    while BUSY.load(SeqCst) {
        std::thread::sleep(POLL);
    }
}

pub fn restart_explorer() -> Result<(), String> {
    if BUSY.swap(true, SeqCst) {
        return Err("Already restarting".to_string());
    }
    let result = restart();
    BUSY.store(false, SeqCst);
    result
}

fn restart() -> Result<(), String> {
    let failed = || "Could not restart".to_string();
    let explorer = explorer().ok_or_else(failed)?;
    let shell = shell_process();
    if shell != 0 {
        let process = unique(shell, &explorer).ok_or_else(failed)?;
        if !through_restart_manager(process) && shell_process() != 0 {
            return Err(failed());
        }
        if shell_returns() {
            return Ok(());
        }
    }
    // Started only without a desktop; otherwise it would open a folder.
    Command::new(&explorer).spawn().map_err(|_| failed())?;
    if shell_returns() {
        Ok(())
    } else {
        Err(failed())
    }
}

/// False when Explorer was not closed as asked.
fn through_restart_manager(process: RM_UNIQUE_PROCESS) -> bool {
    unsafe {
        let mut session = 0;
        let mut key = [0u16; CCH_RM_SESSION_KEY as usize + 1];
        if RmStartSession(&mut session, None, PWSTR(key.as_mut_ptr())) != ERROR_SUCCESS {
            return false;
        }
        // Forced only when it does not answer.
        let closed = RmRegisterResources(session, None, Some(&[process]), None) == ERROR_SUCCESS
            && RmShutdown(session, RmForceShutdown.0 as u32, None) == ERROR_SUCCESS;
        if closed {
            // The caller checks that the desktop came back.
            let _ = RmRestart(session, None, None);
        }
        let _ = RmEndSession(session);
        closed
    }
}

/// The process behind the desktop; 0 when there is none.
fn shell_process() -> u32 {
    let mut id = 0;
    unsafe {
        let shell = GetShellWindow();
        if !shell.is_invalid() {
            GetWindowThreadProcessId(shell, Some(&mut id));
        }
    }
    id
}

fn shell_returns() -> bool {
    let end = Instant::now() + SHELL_TIME;
    while shell_process() == 0 {
        if Instant::now() >= end {
            return false;
        }
        std::thread::sleep(POLL);
    }
    true
}

fn explorer() -> Option<PathBuf> {
    let mut buf = [0u16; 260];
    let n = unsafe { GetWindowsDirectoryW(Some(&mut buf)) } as usize;
    let dir = buf.get(..n).filter(|d| !d.is_empty())?;
    Some(Path::new(&String::from_utf16_lossy(dir)).join("explorer.exe"))
}

/// None unless the process is `explorer`: another program may be the desktop.
fn unique(id: u32, explorer: &Path) -> Option<RM_UNIQUE_PROCESS> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, id).ok()?;
        let mut name = [0u16; 1024];
        let mut len = name.len() as u32;
        let buf = PWSTR(name.as_mut_ptr());
        let named = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, buf, &mut len);
        let (mut start, mut end, mut kernel, mut user) = Default::default();
        let timed = GetProcessTimes(process, &mut start, &mut end, &mut kernel, &mut user);
        let _ = CloseHandle(process);
        named.and(timed).ok()?;
        let name = String::from_utf16_lossy(&name[..len as usize]);
        let same = name.eq_ignore_ascii_case(explorer.to_str()?);
        same.then_some(RM_UNIQUE_PROCESS {
            dwProcessId: id,
            ProcessStartTime: start,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_press() {
        let t = Instant::now();
        assert!(!confirmed(None, t));
        assert!(!confirmed(Some(t), t + LOCK / 2));
        assert!(confirmed(Some(t), t + LOCK));
    }

    #[test]
    fn only_explorer_is_restarted() {
        let explorer = explorer().unwrap();
        assert!(explorer.is_file());
        assert!(unique(std::process::id(), &explorer).is_none());
    }
}
