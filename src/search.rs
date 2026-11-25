//! What the Google slot opens: results for the copied text, or Google itself.

use std::sync::mpsc::channel;
use std::time::Duration;
use windows::Win32::Foundation::HGLOBAL;
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::Memory::*;
use windows::core::w;

const HOME: &str = "https://www.google.com/";
/// Longer copied text is not searched for.
const MAX_TEXT: usize = 300;
/// Longer addresses can fail to open.
const MAX_URL: usize = 2000;
/// A program that is slow to hand over its copied text is not waited for.
const CLIPBOARD_TIME: Duration = Duration::from_millis(500);
const CF_UNICODETEXT: u32 = 13;

/// Reads the clipboard once. May wait, so not for the wheel's thread.
pub fn address() -> String {
    let (tx, rx) = channel();
    std::thread::spawn(move || tx.send(clipboard().unwrap_or_default()));
    target(&rx.recv_timeout(CLIPBOARD_TIME).unwrap_or_default())
}

fn target(text: &str) -> String {
    let text = clean(text);
    // Part of a long text is not what was copied.
    let fits = !text.is_empty() && text.encode_utf16().count() <= MAX_TEXT;
    (fits.then(|| url(&text)).flatten()).unwrap_or_else(|| HOME.to_string())
}

fn clipboard() -> Option<String> {
    unsafe {
        // Another program may hold it for a moment.
        let open = || match OpenClipboard(None) {
            Ok(()) => true,
            Err(_) => {
                std::thread::sleep(Duration::from_millis(20));
                false
            }
        };
        if !(0..5).any(|_| open()) {
            return None;
        }
        let text = clipboard_text();
        let _ = CloseClipboard();
        text
    }
}

/// With the clipboard open.
unsafe fn clipboard_text() -> Option<String> {
    unsafe {
        // Password programs mark what must not be read by others.
        for name in [
            w!("ExcludeClipboardContentFromMonitorProcessing"),
            w!("Clipboard Viewer Ignore"),
        ] {
            if IsClipboardFormatAvailable(RegisterClipboardFormatW(name)).is_ok() {
                return None;
            }
        }
        let history = RegisterClipboardFormatW(w!("CanIncludeInClipboardHistory"));
        if IsClipboardFormatAvailable(history).is_ok() && units(history, 2)?.iter().all(|&u| u == 0)
        {
            return None;
        }
        // Four times the limit leaves room for line breaks that go away.
        let all = units(CF_UNICODETEXT, 4 * MAX_TEXT)?;
        let end = all.iter().position(|&u| u == 0)?;
        Some(String::from_utf16_lossy(&all[..end]))
    }
}

/// The first `max` 16-bit units of a format.
unsafe fn units(format: u32, max: usize) -> Option<Vec<u16>> {
    unsafe {
        let data = HGLOBAL(GetClipboardData(format).ok()?.0);
        let len = (GlobalSize(data) / 2).min(max);
        let p = GlobalLock(data) as *const u16;
        if p.is_null() {
            return None;
        }
        let all = std::slice::from_raw_parts(p, len).to_vec();
        let _ = GlobalUnlock(data);
        Some(all)
    }
}

/// One line: line breaks and runs of spaces become one space.
fn clean(text: &str) -> String {
    let text: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn url(text: &str) -> Option<String> {
    let mut url = format!("{HOME}search?q=");
    for b in text.bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                url.push(b as char)
            }
            _ => url.push_str(&format!("%{b:02X}")),
        }
    }
    (url.len() <= MAX_URL).then_some(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses() {
        let q = |t: &str| url(t).unwrap().split_once("?q=").unwrap().1.to_string();
        assert_eq!(q("a b"), "a%20b");
        assert_eq!(q("مدار"), "%D9%85%D8%AF%D8%A7%D8%B1");
        assert_eq!(
            q("c++ & \"x\" #1 %?"),
            "c%2B%2B%20%26%20%22x%22%20%231%20%25%3F"
        );
        assert!(crate::config::is_web_url(&url("a b\"c").unwrap()));
        // Every Arabic text within the limit fits.
        assert!(url(&"م".repeat(MAX_TEXT)).is_some());
        assert!(url(&"漢".repeat(MAX_TEXT)).is_none());
    }

    #[test]
    fn what_opens() {
        assert_eq!(target(" a\r\nb "), "https://www.google.com/search?q=a%20b");
        for text in [
            "",
            " \r\n ",
            &"a".repeat(MAX_TEXT + 1),
            &"漢".repeat(MAX_TEXT),
        ] {
            assert_eq!(target(text), HOME);
        }
        assert!(target(&"a".repeat(MAX_TEXT)).contains("?q="));
    }

    #[test]
    fn one_line() {
        assert_eq!(clean("  a\r\n\tb\u{0}  c "), "a b c");
        assert_eq!(clean(" \r\n "), "");
    }
}
