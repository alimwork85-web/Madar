//! Volume and mute of the default output device, and the choice of it.
//!
//! The audio service can block, so its calls run on a worker thread that lives
//! only while the wheel uses them.

pub mod output;

use output::Device;
use std::sync::mpsc::{Receiver, Sender, channel};
use windows::Win32::Foundation::{ERROR_NOT_FOUND, HWND, LPARAM, WPARAM};
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{IMMDeviceEnumerator, MMDeviceEnumerator, eMultimedia, eRender};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};
use windows::core::HRESULT;

/// An answer waits in `Audio`.
pub const WM_AUDIO: u32 = WM_APP + 14;
/// Percent per wheel notch or arrow key.
pub const STEP: i32 = 2;
const NOTCH: i32 = 120;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Volume {
    pub percent: i32,
    pub muted: bool,
}

pub enum Order {
    Read,
    /// One press, up or down.
    Step(bool),
    /// Percent to add.
    Change(i32),
    ToggleMute,
    /// Id of the device to use.
    Choose(String),
}

pub struct Answer {
    pub volume: Result<Volume, String>,
    /// None when the devices were not read again.
    pub devices: Option<Vec<Device>>,
    /// The device could not be changed.
    pub failed: bool,
}

pub struct Audio {
    tx: Sender<Order>,
    rx: Receiver<Answer>,
}

impl Audio {
    /// The thread ends once `Audio` is dropped.
    pub fn new(target: HWND) -> Self {
        let (tx, orders) = channel();
        let (answers, rx) = channel();
        let target = target.0 as usize;
        std::thread::spawn(move || {
            let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
            for order in orders {
                if answers.send(run(order)).is_err() {
                    break;
                }
                let hwnd = HWND(target as *mut _);
                let _ = unsafe { PostMessageW(Some(hwnd), WM_AUDIO, WPARAM(0), LPARAM(0)) };
            }
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

fn message(code: HRESULT) -> &'static str {
    if code == HRESULT::from_win32(ERROR_NOT_FOUND.0) {
        "No output device"
    } else {
        "Sound not available"
    }
}

fn run(order: Order) -> Answer {
    let found = unsafe {
        CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL)
    };
    let text = |e: windows::core::Error| message(e.code()).to_string();
    let devices = match found {
        Ok(d) => d,
        Err(e) => {
            return Answer {
                volume: Err(text(e)),
                devices: None,
                failed: matches!(order, Order::Choose(_)),
            };
        }
    };
    let failed = match &order {
        Order::Choose(id) => !output::choose(&devices, id),
        _ => false,
    };
    let changed_list = matches!(order, Order::Read | Order::Choose(_));
    Answer {
        failed,
        devices: changed_list.then(|| output::list(&devices).unwrap_or_default()),
        volume: volume(&devices, order).map_err(text),
    }
}

fn volume(devices: &IMMDeviceEnumerator, order: Order) -> windows::core::Result<Volume> {
    unsafe {
        // Asked every time: the default device may have changed.
        let device = devices.GetDefaultAudioEndpoint(eRender, eMultimedia)?;
        let volume: IAudioEndpointVolume = device.Activate(CLSCTX_ALL, None)?;
        let none = std::ptr::null();
        let muted = volume.GetMute()?.as_bool();
        match order {
            Order::Read | Order::Choose(_) => {}
            Order::Change(_) | Order::Step(_) => {
                let level = volume.GetMasterVolumeLevelScalar()?;
                let (level, up) = match order {
                    Order::Change(by) => (changed(level, by), by > 0),
                    _ => {
                        let up = matches!(order, Order::Step(true));
                        (step(percent(level), up) as f32 / 100.0, up)
                    }
                };
                volume.SetMasterVolumeLevelScalar(level, none)?;
                if muted && up {
                    volume.SetMute(false, none)?;
                }
            }
            Order::ToggleMute => volume.SetMute(!muted, none)?,
        }
        Ok(Volume {
            percent: percent(volume.GetMasterVolumeLevelScalar()?),
            muted: volume.GetMute()?.as_bool(),
        })
    }
}

fn percent(level: f32) -> i32 {
    (level * 100.0).round() as i32
}

fn changed(level: f32, by: i32) -> f32 {
    (percent(level) + by).clamp(0, 100) as f32 / 100.0
}

/// The next of 0, 5, 10, 20, 30 up to 100. Low levels differ more.
pub fn step(now: i32, up: bool) -> i32 {
    let mut levels = [0, 5].into_iter().chain((10..=100).step_by(10));
    if up {
        levels.find(|&l| l > now).unwrap_or(100)
    } else {
        levels.take_while(|&l| l < now).last().unwrap_or(0)
    }
}

/// Percent in a wheel message. Touchpads send small deltas, so what is left
/// over stays in `rest` for the next one.
pub fn scrolled(rest: &mut i32, delta: i32) -> i32 {
    if delta.signum() != rest.signum() {
        *rest = 0;
    }
    *rest += delta;
    let by = *rest * STEP / NOTCH;
    *rest -= by * NOTCH / STEP;
    by
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_stays_in_range() {
        assert_eq!(changed(0.4, 2), 0.42);
        assert_eq!(changed(0.404, -2), 0.38);
        assert_eq!(changed(0.99, 2), 1.0);
        assert_eq!(changed(0.01, -2), 0.0);
        assert_eq!(percent(0.29), 29);
    }

    #[test]
    fn steps() {
        let next = |up, from: [i32; 7]| from.map(|now| step(now, up));
        let from = [0, 3, 5, 10, 34, 95, 100];
        assert_eq!(next(true, from), [5, 5, 10, 20, 40, 100, 100]);
        assert_eq!(next(false, from), [0, 0, 0, 5, 30, 90, 90]);
    }

    #[test]
    fn scrolling() {
        let mut rest = 0;
        assert_eq!(scrolled(&mut rest, 120), 2);
        assert_eq!(scrolled(&mut rest, -240), -4);
        let total: i32 = (0..40).map(|_| scrolled(&mut rest, 6)).sum();
        assert_eq!((total, rest), (4, 0));
        // Turning back drops what was left over.
        assert_eq!(scrolled(&mut rest, 50), 0);
        assert_eq!(scrolled(&mut rest, -50), 0);
        assert_eq!(rest, -50);
    }

    #[test]
    fn errors() {
        assert_eq!(message(HRESULT(0x80070490u32 as i32)), "No output device");
        assert_eq!(message(HRESULT(-1)), "Sound not available");
    }
}
