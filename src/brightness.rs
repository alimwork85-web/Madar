//! Brightness of the built-in screen, through WMI.
//!
//! WMI can be slow, so its calls run on a worker thread that lives only while
//! the wheel uses them.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{E_FAIL, E_POINTER, HWND, LPARAM, WPARAM};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
    CoSetProxyBlanket, CoUninitialize, EOAC_NONE, RPC_C_AUTHN_LEVEL_CALL,
    RPC_C_IMP_LEVEL_IMPERSONATE,
};
use windows::Win32::System::Rpc::{RPC_C_AUTHN_WINNT, RPC_C_AUTHZ_NONE};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::System::Wmi::*;
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};
use windows::core::{BSTR, Error, PCWSTR, Result, w};

/// An answer waits in `Light`.
pub const WM_LIGHT: u32 = WM_APP + 16;
/// Percent per press.
pub const STEP: i32 = 2;
/// Lower could look like a screen that is off.
const LOWEST: i32 = 5;
/// The value read back follows a change late, so the one just set counts.
const SETTLE_TIME: Duration = Duration::from_secs(1);
const QUERY_MS: i32 = 5000;

pub enum Order {
    Read,
    /// Percent to add.
    Change(i32),
}

/// The percent, or None when Windows cannot say or change it.
pub type Answer = Option<i32>;

pub struct Light {
    tx: Sender<Order>,
    rx: Receiver<Answer>,
}

impl Light {
    /// The thread ends once `Light` is dropped.
    pub fn new(target: HWND) -> Self {
        let (tx, orders) = channel();
        let (answers, rx) = channel();
        let target = target.0 as usize;
        std::thread::spawn(move || {
            let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
            work(&orders, |answer| {
                let sent = answers.send(answer).is_ok();
                let hwnd = HWND(target as *mut _);
                let _ = unsafe { PostMessageW(Some(hwnd), WM_LIGHT, WPARAM(0), LPARAM(0)) };
                sent
            });
            if com {
                unsafe { CoUninitialize() };
            }
        });
        Self { tx, rx }
    }

    pub fn send(&self, order: Order) {
        // Fails only if the thread died, and then nothing is left to answer.
        let _ = self.tx.send(order);
    }

    pub fn answers(&self) -> Vec<Answer> {
        self.rx.try_iter().collect()
    }
}

/// Orders that came together are answered together.
fn work(orders: &Receiver<Order>, answer: impl Fn(Answer) -> bool) {
    let mut screen = None;
    let mut last: Option<(Instant, i32)> = None;
    while let Ok(first) = orders.recv() {
        let result = (|| {
            if screen.is_none() {
                screen = Some(connect()?);
            }
            let wmi = screen.as_ref().ok_or(Error::from(E_POINTER))?;
            let now = match last {
                Some((at, value)) if at.elapsed() < SETTLE_TIME => value,
                _ => read(wmi)?,
            };
            let all = std::iter::once(first).chain(orders.try_iter());
            let value = all.fold(now, |v, order| changed(v, &order));
            if value != now {
                set(wmi, value)?;
                last = Some((Instant::now(), value));
            }
            Ok::<_, Error>(value)
        })();
        if result.is_err() {
            // The service may have restarted; the next order connects again.
            (screen, last) = (None, None);
        }
        if !answer(result.ok()) {
            return;
        }
    }
}

fn changed(now: i32, order: &Order) -> i32 {
    let value = match order {
        Order::Read => return now,
        Order::Change(by) => now + by,
    };
    if value < now {
        // A screen already darker stays as it is.
        value.max(LOWEST).min(now)
    } else {
        value.min(100).max(now)
    }
}

fn connect() -> Result<IWbemServices> {
    unsafe {
        let none = BSTR::new();
        let locator: IWbemLocator = CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER)?;
        let place = BSTR::from("ROOT\\WMI");
        let wait = WBEM_FLAG_CONNECT_USE_MAX_WAIT.0;
        let wmi = locator.ConnectServer(&place, &none, &none, &none, wait, &none, None)?;
        CoSetProxyBlanket(
            &wmi,
            RPC_C_AUTHN_WINNT,
            RPC_C_AUTHZ_NONE,
            PCWSTR::null(),
            RPC_C_AUTHN_LEVEL_CALL,
            RPC_C_IMP_LEVEL_IMPERSONATE,
            None,
            EOAC_NONE,
        )?;
        Ok(wmi)
    }
}

/// The first answer to a query.
fn find(wmi: &IWbemServices, query: &str) -> Result<IWbemClassObject> {
    unsafe {
        let flags = WBEM_FLAG_FORWARD_ONLY | WBEM_FLAG_RETURN_IMMEDIATELY;
        let found = wmi.ExecQuery(&BSTR::from("WQL"), &BSTR::from(query), flags, None)?;
        let (mut object, mut n) = ([None], 0);
        found.Next(QUERY_MS, &mut object, &mut n).ok()?;
        let [object] = object;
        object.ok_or(Error::from(E_FAIL))
    }
}

fn get(object: &IWbemClassObject, name: PCWSTR) -> Result<VARIANT> {
    let mut value = VARIANT::default();
    unsafe { object.Get(name, 0, &mut value, None, None)? };
    Ok(value)
}

fn read(wmi: &IWbemServices) -> Result<i32> {
    let screen = find(wmi, "SELECT * FROM WmiMonitorBrightness WHERE Active=TRUE")?;
    i32::try_from(&get(&screen, w!("CurrentBrightness"))?)
}

fn set(wmi: &IWbemServices, percent: i32) -> Result<()> {
    unsafe {
        let screen = find(
            wmi,
            "SELECT * FROM WmiMonitorBrightnessMethods WHERE Active=TRUE",
        )?;
        let path = BSTR::try_from(&get(&screen, w!("__PATH"))?)?;
        let mut class = None;
        let class_name = BSTR::from("WmiMonitorBrightnessMethods");
        let flags = WBEM_GENERIC_FLAG_TYPE(0);
        wmi.GetObject(&class_name, flags, None, Some(&mut class), None)?;
        let class: IWbemClassObject = class.ok_or(Error::from(E_POINTER))?;
        let (mut input, mut output) = (None, None);
        class.GetMethod(w!("WmiSetBrightness"), 0, &mut input, &mut output)?;
        let input = input.ok_or(Error::from(E_POINTER))?.SpawnInstance(0)?;
        // Seconds Windows may take.
        input.Put(w!("Timeout"), 0, &VARIANT::from(1i32), 0)?;
        input.Put(w!("Brightness"), 0, &VARIANT::from(percent as u8), 0)?;
        let method = BSTR::from("WmiSetBrightness");
        wmi.ExecMethod(&path, &method, flags, None, &input, None, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stays_in_range() {
        let turn = |now, by| changed(now, &Order::Change(by));
        assert_eq!((turn(10, STEP), turn(34, STEP)), (12, 36));
        assert_eq!((turn(10, -STEP), turn(5, -STEP)), (8, 5));
        // Darker than the limit: down changes nothing, up leaves it.
        assert_eq!((turn(0, -STEP), turn(3, -2)), (0, 3));
        assert_eq!((turn(0, STEP), turn(0, 2)), (2, 2));
        assert_eq!((turn(6, -2), turn(99, 2)), (5, 100));
        assert_eq!(changed(3, &Order::Read), 3);
    }
}
