//! Output devices, and changing the default one.
//!
//! Windows documents no way to change it. The undocumented call runs in a
//! child process (`madar --output`), so a Windows update that changes it cannot
//! take the wheel down, and the answer is trusted only after reading it back.

use std::ffi::c_void;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree, STGM_READ,
};
use windows::core::{GUID, HRESULT, HSTRING, IUnknown, IUnknown_Vtbl, Interface, PCWSTR, Result};

/// First argument of the child; then the device id.
pub const CHILD: &str = "--output";
const CHILD_TIME: Duration = Duration::from_secs(5);
/// The new default can show up a moment after the call.
const SETTLE_TIME: Duration = Duration::from_millis(500);
const POLL: Duration = Duration::from_millis(20);
/// PKEY_Device_FriendlyName.
const NAME: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0),
    pid: 14,
};

#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub current: bool,
}

/// The connected output devices, by name.
pub fn list(devices: &IMMDeviceEnumerator) -> Result<Vec<Device>> {
    let current = default(devices).unwrap_or_default();
    let mut all = Vec::new();
    unsafe {
        let found = devices.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)?;
        for i in 0..found.GetCount()? {
            // A device can leave meanwhile; the others still show.
            let Ok((id, name)) = found.Item(i).and_then(|d| Ok((id(&d)?, name(&d)?))) else {
                continue;
            };
            all.push(Device {
                current: id == current,
                id,
                name,
            });
        }
    }
    all.sort_by_cached_key(|d| (d.name.to_lowercase(), d.id.clone()));
    Ok(all)
}

fn default(devices: &IMMDeviceEnumerator) -> Result<String> {
    id(&unsafe { devices.GetDefaultAudioEndpoint(eRender, eMultimedia) }?)
}

fn id(device: &IMMDevice) -> Result<String> {
    unsafe {
        let text = device.GetId()?;
        let id = text.to_string();
        CoTaskMemFree(Some(text.0 as *const c_void));
        Ok(id?)
    }
}

fn name(device: &IMMDevice) -> Result<String> {
    let value = unsafe { device.OpenPropertyStore(STGM_READ)?.GetValue(&NAME)? };
    Ok(value.to_string())
}

/// Makes a device the default. True only when Windows confirms it.
pub fn choose(devices: &IMMDeviceEnumerator, id: &str) -> bool {
    if !run_child(id).unwrap_or(false) {
        return false;
    }
    let end = Instant::now() + SETTLE_TIME;
    loop {
        if default(devices).is_ok_and(|d| d == id) {
            return true;
        }
        if Instant::now() >= end {
            return false;
        }
        std::thread::sleep(POLL);
    }
}

fn run_child(id: &str) -> std::io::Result<bool> {
    let mut child = Command::new(std::env::current_exe()?)
        .args([CHILD, id])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let end = Instant::now() + CHILD_TIME;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status.success());
        }
        if Instant::now() >= end {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(false);
        }
        std::thread::sleep(POLL);
    }
}

/// The child's work. The calls device is left alone.
pub fn child(id: Option<String>) -> Result<()> {
    let id = HSTRING::from(id.unwrap_or_default());
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        let policy: PolicyConfig = CoCreateInstance(&POLICY_CONFIG, None, CLSCTX_ALL)?;
        for role in [eConsole, eMultimedia] {
            let set = policy.vtable().set_default;
            set(policy.as_raw(), PCWSTR(id.as_ptr()), role).ok()?;
        }
    }
    Ok(())
}

// Class, interface id and method order as listed by com-policy-config (MIT, sidit77).
const POLICY_CONFIG: GUID = GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);

#[repr(transparent)]
#[derive(Clone)]
struct PolicyConfig(IUnknown);

unsafe impl Interface for PolicyConfig {
    type Vtable = PolicyVtable;
    const IID: GUID = GUID::from_u128(0xf8679f50_850a_41cf_9c72_430f290290c8);
}

#[repr(C)]
struct PolicyVtable {
    base: IUnknown_Vtbl,
    /// Ten methods this program never calls.
    skipped: [usize; 10],
    set_default: unsafe extern "system" fn(*mut c_void, PCWSTR, ERole) -> HRESULT,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_default_is_the_eleventh_method() {
        let word = size_of::<usize>();
        assert_eq!(std::mem::offset_of!(PolicyVtable, set_default), 13 * word);
    }
}
