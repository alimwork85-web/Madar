//! Start with Windows: one value under the current user's Run key.

use windows::Win32::Foundation::*;
use windows::Win32::System::Registry::*;
use windows::core::{HSTRING, PCWSTR, w};

const RUN: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
/// Where Task Manager and Windows Settings keep what the user turned off.
const APPROVED: PCWSTR =
    w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run");
const NAME: PCWSTR = w!("Madar");

/// Quoted, so a path with spaces starts this program and nothing else.
fn command() -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    Ok(format!("\"{}\"", exe.display()))
}

/// A test copy must never touch the real Run key.
fn testing() -> bool {
    std::env::var_os("MADAR_DIR").is_some()
}

/// Whether Windows starts this program; `None` in a test copy.
pub fn enabled() -> Option<bool> {
    (!testing()).then(|| {
        command().is_ok_and(|c| read(RUN).as_deref() == Some(c.as_str())) && !blocked(APPROVED)
    })
}

pub fn set(on: bool) -> Result<(), String> {
    if testing() {
        return Ok(());
    }
    write(RUN, on.then(command).transpose()?.as_deref())?;
    // Without this, Windows would keep skipping an entry turned off in Task Manager.
    if on { write(APPROVED, None) } else { Ok(()) }
}

/// Points the entry at this program when the one it names is gone.
pub fn repair() -> Result<(), String> {
    if testing() {
        return Ok(());
    }
    match read(RUN) {
        // Only the path changes: an entry turned off in Task Manager stays off.
        Some(old) if gone(&old) => write(RUN, Some(&command()?)),
        _ => Ok(()),
    }
}

/// A path that cannot be checked is not gone.
fn gone(command: &str) -> bool {
    std::path::Path::new(command.trim_matches('"'))
        .try_exists()
        .is_ok_and(|found| !found)
}

/// Windows marks an entry that is off with an odd first byte.
fn blocked(key: PCWSTR) -> bool {
    let mut buf = [0u8; 64];
    let mut bytes = buf.len() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key,
            NAME,
            RRF_RT_REG_BINARY,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut bytes),
        )
    };
    status == ERROR_SUCCESS && bytes > 0 && buf[0] & 1 == 1
}

fn read(key: PCWSTR) -> Option<String> {
    let mut buf = vec![0u16; 2048];
    let mut bytes = (buf.len() * 2) as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key,
            NAME,
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut bytes),
        )
    };
    let end = buf.iter().position(|&c| c == 0)?;
    (status == ERROR_SUCCESS).then(|| String::from_utf16_lossy(&buf[..end]))
}

/// `None` removes the value.
fn write(key: PCWSTR, value: Option<&str>) -> Result<(), String> {
    let status = match value {
        Some(text) => {
            let text = HSTRING::from(text);
            unsafe {
                RegSetKeyValueW(
                    HKEY_CURRENT_USER,
                    key,
                    NAME,
                    REG_SZ.0,
                    Some(text.as_ptr().cast()),
                    (text.len() as u32 + 1) * 2,
                )
            }
        }
        None => unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key, NAME) },
    };
    match status {
        ERROR_SUCCESS => Ok(()),
        ERROR_FILE_NOT_FOUND if value.is_none() => Ok(()),
        e => Err(format!("Windows refused the change (error {}).", e.0)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Uses a key of its own, never the real Run key.
    #[test]
    fn value_round_trip() {
        let name = HSTRING::from(format!("Software\\MadarTest{}", std::process::id()));
        let key = PCWSTR(name.as_ptr());
        assert_eq!(read(key), None);
        write(key, Some("\"C:\\تجربة x\\madar.exe\"")).unwrap();
        let got = read(key);
        write(key, None).unwrap();
        let removed_twice = write(key, None);
        let _ = unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, key) };
        assert_eq!(got.as_deref(), Some("\"C:\\تجربة x\\madar.exe\""));
        assert_eq!(removed_twice, Ok(()));
        assert!(command().unwrap().starts_with('"'));
    }

    #[test]
    fn entry_of_a_missing_program_is_gone() {
        assert!(!gone(&command().unwrap()));
        assert!(gone("\"C:\\لا يوجد\\madar.exe\""));
    }

    /// Uses a key of its own, never the real StartupApproved key.
    #[test]
    fn odd_first_byte_blocks() {
        let name = HSTRING::from(format!("Software\\MadarBlocked{}", std::process::id()));
        let key = PCWSTR(name.as_ptr());
        let state = |first: u8| {
            let data = [first, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
            let set = unsafe {
                RegSetKeyValueW(
                    HKEY_CURRENT_USER,
                    key,
                    NAME,
                    REG_BINARY.0,
                    Some(data.as_ptr().cast()),
                    data.len() as u32,
                )
            };
            assert_eq!(set, ERROR_SUCCESS);
            blocked(key)
        };
        let got = (blocked(key), state(2), state(3), state(6));
        let cleared = write(key, None);
        let _ = unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, key) };
        assert_eq!(got, (false, false, true, false));
        assert_eq!(cleared, Ok(()));
    }
}
