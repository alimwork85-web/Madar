//! The Bluetooth radio: reading it and switching it, in a child process
//! (`madar --bluetooth`). In the wheel's process the Windows library crashed
//! when COM ended with its thread, and it stayed loaded.

use std::io::{BufReader, Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};
use windows::Devices::Radios::{Radio, RadioAccessStatus, RadioKind, RadioState};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

/// First argument of the child.
pub const CHILD: &str = "--bluetooth";
const TOGGLE: &str = "toggle";
/// Answers wait in the receiver, or it ended.
pub const WM_RADIO: u32 = WM_APP + 15;
/// Repeats of a held key and double clicks must not switch it back.
pub const LOCK: Duration = Duration::from_secs(1);
const TIME: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(20);

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Answer {
    /// Being switched on or off.
    Working(bool),
    Is(bool),
    /// No radio, refused, or it did not change in time.
    Failed,
}

impl Answer {
    /// What the child writes for it.
    fn code(self) -> u8 {
        match self {
            Self::Working(false) => b'a',
            Self::Working(true) => b'b',
            Self::Is(false) => b'0',
            Self::Is(true) => b'1',
            Self::Failed => b'x',
        }
    }

    fn from_code(code: u8) -> Self {
        let all = [
            Self::Working(false),
            Self::Working(true),
            Self::Is(false),
            Self::Is(true),
        ];
        (all.into_iter().find(|a| a.code() == code)).unwrap_or(Self::Failed)
    }
}

/// Reads the state, or switches to the other one. A read that fails says nothing.
pub fn start(target: HWND, toggle: bool) -> Receiver<Answer> {
    let (tx, rx) = channel();
    let target = target.0 as usize;
    std::thread::spawn(move || {
        let post = || {
            let hwnd = HWND(target as *mut _);
            let _ = unsafe { PostMessageW(Some(hwnd), WM_RADIO, WPARAM(0), LPARAM(0)) };
        };
        let mut last = None;
        let ran = run_child(toggle, |answer| {
            last = Some(answer);
            let _ = tx.send(answer);
            post();
        });
        // The child ended early.
        let ended = matches!(last, Some(Answer::Is(_) | Answer::Failed));
        if toggle && (ran.is_err() || !ended) {
            let _ = tx.send(Answer::Failed);
        }
        drop(tx);
        post();
    });
    rx
}

fn run_child(toggle: bool, mut tell: impl FnMut(Answer)) -> std::io::Result<()> {
    let mut child = Command::new(std::env::current_exe()?)
        .args([CHILD, if toggle { TOGGLE } else { "read" }])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    if let Some(out) = child.stdout.take() {
        // Ends with the child, which never lives long.
        for code in BufReader::new(out).bytes().map_while(Result::ok) {
            tell(Answer::from_code(code));
        }
    }
    child.wait().map(drop)
}

/// The child's work: one byte on stdout per answer.
pub fn child(order: Option<String>) {
    std::thread::spawn(|| {
        std::thread::sleep(TIME + LOCK);
        std::process::exit(1);
    });
    let tell = |answer: Answer| {
        let mut out = std::io::stdout();
        let _ = out.write_all(&[answer.code()]).and_then(|()| out.flush());
    };
    // Left to the end of the process: the library keeps objects between calls.
    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    let end = Instant::now() + TIME;
    let radio = find(end);
    let read = || radio.as_ref().and_then(state);
    if order.as_deref() == Some(TOGGLE) {
        let set = |on| radio.as_ref().and_then(|r| set(r, on, end)).is_some();
        tell(switch(read, set, end, tell));
    } else if let Some(on) = read() {
        tell(Answer::Is(on));
    }
}

fn switch(
    read: impl Fn() -> Option<bool>,
    set: impl FnOnce(bool) -> bool,
    end: Instant,
    tell: impl Fn(Answer),
) -> Answer {
    let Some(was) = read() else {
        return Answer::Failed;
    };
    tell(Answer::Working(!was));
    if !set(!was) {
        return Answer::Failed;
    }
    // Windows accepts a request before the radio has changed, and can still not do it.
    while read() != Some(!was) {
        if Instant::now() >= end {
            return Answer::Failed;
        }
        std::thread::sleep(POLL);
    }
    Answer::Is(!was)
}

/// Waits for an operation, given its `AsyncStatus`.
fn finished(end: Instant, status: impl Fn() -> windows::core::Result<i32>) -> Option<()> {
    loop {
        match status().ok()? {
            0 if Instant::now() < end => std::thread::sleep(POLL),
            1 => return Some(()),
            _ => return None,
        }
    }
}

fn find(end: Instant) -> Option<Radio> {
    let op = Radio::GetRadiosAsync().ok()?;
    finished(end, || Ok(op.Status()?.0))?;
    (op.GetResults().ok()?.into_iter()).find(|r| r.Kind() == Ok(RadioKind::Bluetooth))
}

/// None when the hardware or a policy keeps it disabled.
fn state(radio: &Radio) -> Option<bool> {
    match radio.State().ok()? {
        RadioState::On => Some(true),
        RadioState::Off => Some(false),
        _ => None,
    }
}

fn set(radio: &Radio, on: bool, end: Instant) -> Option<()> {
    let allowed = |s| (s == RadioAccessStatus::Allowed).then_some(());
    let op = Radio::RequestAccessAsync().ok()?;
    finished(end, || Ok(op.Status()?.0))?;
    allowed(op.GetResults().ok()?)?;
    let state = if on { RadioState::On } else { RadioState::Off };
    let op = radio.SetStateAsync(state).ok()?;
    finished(end, || Ok(op.Status()?.0))?;
    allowed(op.GetResults().ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    fn run(start: Option<bool>, accepts: bool, changes: bool) -> (Answer, Vec<Answer>) {
        let now = Cell::new(start);
        let told = RefCell::new(Vec::new());
        let end = Instant::now() + POLL * 2;
        let set = |on| {
            if accepts && changes {
                now.set(Some(on));
            }
            accepts
        };
        let answer = switch(|| now.get(), set, end, |a| told.borrow_mut().push(a));
        (answer, told.into_inner())
    }

    #[test]
    fn switches_both_ways() {
        use Answer::*;
        assert_eq!(
            run(Some(false), true, true),
            (Is(true), vec![Working(true)])
        );
        assert_eq!(
            run(Some(true), true, true),
            (Is(false), vec![Working(false)])
        );
    }

    #[test]
    fn codes() {
        use Answer::*;
        for a in [Working(true), Working(false), Is(true), Is(false), Failed] {
            assert_eq!(Answer::from_code(a.code()), a);
        }
        assert_eq!(Answer::from_code(0), Failed);
    }

    #[test]
    fn failures() {
        use Answer::*;
        assert_eq!(run(None, true, true), (Failed, vec![]));
        assert_eq!(run(Some(true), false, true), (Failed, vec![Working(false)]));
        assert_eq!(run(Some(false), true, false), (Failed, vec![Working(true)]));
    }

    #[test]
    fn waiting() {
        let end = Instant::now() + POLL * 2;
        assert_eq!(finished(end, || Ok(1)), Some(()));
        assert_eq!(finished(end, || Ok(0)), None);
        assert_eq!(finished(end, || Ok(3)), None);
    }
}
