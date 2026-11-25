//! Volume and mute of the default output device.
//!
//! The audio service can block, so its calls run on a worker thread that lives
//! only while the wheel uses them.

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
/// Percent per wheel notch or key press.
pub const STEP: i32 = 2;
const NOTCH: i32 = 120;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Volume {
    pub percent: i32,
    pub muted: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Order {
    Read,
    /// Percent to add.
    Change(i32),
    ToggleMute,
}

type Answer = Result<Volume, String>;

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
                let answer = run(order).map_err(|e| message(e.code()).to_string());
                if answers.send(answer).is_err() {
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

    /// The newest answer, if any came.
    pub fn answer(&self) -> Option<Answer> {
        self.rx.try_iter().last()
    }
}

fn message(code: HRESULT) -> &'static str {
    if code == HRESULT::from_win32(ERROR_NOT_FOUND.0) {
        "No output device"
    } else {
        "Sound not available"
    }
}

fn run(order: Order) -> windows::core::Result<Volume> {
    unsafe {
        let devices: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        // Asked every time: the default device may have changed.
        let device = devices.GetDefaultAudioEndpoint(eRender, eMultimedia)?;
        let volume: IAudioEndpointVolume = device.Activate(CLSCTX_ALL, None)?;
        let none = std::ptr::null();
        let muted = volume.GetMute()?.as_bool();
        match order {
            Order::Read => {}
            Order::Change(by) => {
                let level = changed(volume.GetMasterVolumeLevelScalar()?, by);
                volume.SetMasterVolumeLevelScalar(level, none)?;
                if muted && by > 0 {
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
