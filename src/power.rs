//! Shutting the PC down. Never forced: a program with unsaved work can stop it.

use windows::Win32::Foundation::{CloseHandle, ERROR_SUCCESS, GetLastError, HANDLE};
use windows::Win32::Security::*;
use windows::Win32::System::Shutdown::*;
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::core::Result;

pub const OFF: EXIT_WINDOWS_FLAGS = EXIT_WINDOWS_FLAGS(EWX_SHUTDOWN.0 | EWX_POWEROFF.0);

pub fn leave(how: EXIT_WINDOWS_FLAGS) -> std::result::Result<(), String> {
    let reason = SHUTDOWN_REASON(SHTDN_REASON_MAJOR_OTHER.0 | SHTDN_REASON_FLAG_PLANNED.0);
    allow()
        .and_then(|()| unsafe { ExitWindowsEx(how, reason) })
        .map_err(|_| "Windows refused".to_string())
}

/// Windows wants this right switched on before the PC can be shut down.
fn allow() -> Result<()> {
    unsafe {
        let mut token = HANDLE::default();
        let access = TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY;
        OpenProcessToken(GetCurrentProcess(), access, &mut token)?;
        let result = switch_on(token);
        let _ = CloseHandle(token);
        result
    }
}

unsafe fn switch_on(token: HANDLE) -> Result<()> {
    let mut rights = TOKEN_PRIVILEGES {
        PrivilegeCount: 1,
        ..Default::default()
    };
    rights.Privileges[0].Attributes = SE_PRIVILEGE_ENABLED;
    unsafe {
        LookupPrivilegeValueW(None, SE_SHUTDOWN_NAME, &mut rights.Privileges[0].Luid)?;
        AdjustTokenPrivileges(token, false, Some(&rights), 0, None, None)?;
        // It succeeds even when the right was not given.
        match GetLastError() {
            ERROR_SUCCESS => Ok(()),
            e => Err(e.to_hresult().into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn never_forced() {
        assert_eq!(OFF.0 & (EWX_FORCE.0 | EWX_FORCEIFHUNG.0), 0);
        assert_ne!(OFF.0 & EWX_POWEROFF.0, 0);
    }

    #[test]
    fn the_right_can_be_switched_on() {
        allow().unwrap();
    }
}
