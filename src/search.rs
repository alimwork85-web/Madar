//! The Google box: one field, filled with the copied text. Nothing is sent
//! before Enter or Search, and nothing is kept.

use crate::actions;
use crate::settings::{SS_ELLIPSIS, make, message_font, px, send, set_text, text_of};
use crate::wheel::paint::Palette;
use std::cell::{Cell, RefCell};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::*;
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetFocus};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

const CLASS: PCWSTR = w!("MadarSearch");
/// The browser was started, or not; the result waits in `State::running`.
const WM_DONE: u32 = WM_APP + 1;
const SEARCH: u16 = IDOK.0 as u16;
const EDIT: u16 = 100;
/// Characters of the field; longer copied text is not taken.
const MAX_TEXT: usize = 300;
/// Longer addresses can fail to open.
const MAX_URL: usize = 2000;
/// A program that is slow to hand over its copied text is not waited for.
const CLIPBOARD_TIME: Duration = Duration::from_millis(200);
const CF_UNICODETEXT: u32 = 13;

// Logical pixels.
const WIDTH: i32 = 420;
const MARGIN: i32 = 12;
const LABEL: i32 = 20;
const FIELD: i32 = 34;
const BUTTON: i32 = 84;
const GAP: i32 = 4;

struct State {
    hwnd: HWND,
    edit: HWND,
    button: HWND,
    label: HWND,
    error: HWND,
    font: HFONT,
    plate: HBRUSH,
    slot: HBRUSH,
    palette: Palette,
    dpi: u32,
    failed: bool,
    running: Option<Receiver<Result<(), String>>>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    static START: Cell<Option<(Palette, u32)>> = const { Cell::new(None) };
}

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|s| s.try_borrow_mut().ok()?.as_mut().map(f))
}

/// Shows the box around `center`, inside its monitor.
pub fn open(center: POINT, palette: Palette) -> windows::core::Result<()> {
    if with_state(|s| s.hwnd).is_some() {
        return Ok(());
    }
    let text = copied();
    unsafe {
        let module = GetModuleHandleW(None)?;
        RegisterClassW(&WNDCLASSW {
            style: CS_DROPSHADOW,
            lpfnWndProc: Some(wndproc),
            hInstance: module.into(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: CLASS,
            ..Default::default()
        });
        let monitor = MonitorFromPoint(center, MONITOR_DEFAULTTONEAREST);
        let (mut dpi, mut other) = (96, 0);
        let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut other);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let _ = GetMonitorInfoW(monitor, &mut info);
        let work = info.rcWork;
        let (w, h) = (px(WIDTH, dpi), height(dpi, false));
        // The error line must fit as well.
        let bottom = work.bottom - height(dpi, true);
        let x = (center.x - w / 2).min(work.right - w).max(work.left);
        let y = (center.y - h / 2).min(bottom).max(work.top);

        START.with(|s| s.set(Some((palette, dpi))));
        let created = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_CONTROLPARENT,
            CLASS,
            w!("Search Google"),
            WS_POPUP | WS_CLIPCHILDREN,
            x,
            y,
            w,
            h,
            None,
            None,
            Some(module.into()),
            None,
        );
        START.with(|s| s.set(None));
        let hwnd = created?;
        let round = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            (&raw const round).cast(),
            size_of_val(&round) as u32,
        );
        let Some((edit, button)) = with_state(|s| (s.edit, s.button)) else {
            return Ok(());
        };
        set_text(edit, &text);
        changed(edit, button);
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(Some(edit));
        send(edit, EM_SETSEL, 0, -1);
    }
    Ok(())
}

pub fn translate(msg: &MSG) -> bool {
    with_state(|s| s.hwnd).is_some_and(|h| unsafe { IsDialogMessageW(h, msg) }.as_bool())
}

/// Posted, so it is safe inside any other call.
pub fn close() {
    if let Some(hwnd) = with_state(|s| s.hwnd) {
        let _ = unsafe { PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)) };
    }
}

fn height(dpi: u32, error: bool) -> i32 {
    let line = if error { GAP + LABEL } else { 0 };
    px(2 * MARGIN + LABEL + GAP + FIELD + line, dpi)
}

fn create(hwnd: HWND) -> bool {
    let Some((palette, dpi)) = START.with(|s| s.take()) else {
        return false;
    };
    let child = WS_CHILD | WS_VISIBLE;
    let text = child | SS_ELLIPSIS;
    let field = child | WS_TABSTOP | WINDOW_STYLE(ES_AUTOHSCROLL as u32);
    let drawn = child | WS_TABSTOP | WINDOW_STYLE(BS_OWNERDRAW as u32);
    let state = unsafe {
        State {
            hwnd,
            label: make(hwnd, w!("STATIC"), "Search Google", text, 1000),
            edit: make(hwnd, w!("EDIT"), "", field, EDIT),
            button: make(hwnd, w!("BUTTON"), "&Search", drawn, SEARCH),
            error: make(hwnd, w!("STATIC"), "", text, 1001),
            font: message_font(dpi),
            plate: CreateSolidBrush(rgb(palette.plate)),
            slot: CreateSolidBrush(rgb(palette.slot)),
            palette,
            dpi,
            failed: false,
            running: None,
        }
    };
    let all = [state.label, state.edit, state.button, state.error];
    for h in all {
        send(h, WM_SETFONT, state.font.0 as usize, 0);
    }
    send(state.edit, EM_SETLIMITTEXT, MAX_TEXT, 0);
    STATE.with(|s| *s.borrow_mut() = Some(state));
    arrange();
    all.iter().all(|h| !h.is_invalid())
}

/// The field's rectangle; the edit control is one text line inside it.
fn field(dpi: u32) -> RECT {
    let top = px(MARGIN + LABEL + GAP, dpi);
    RECT {
        left: px(MARGIN, dpi),
        top,
        right: px(WIDTH - MARGIN - BUTTON - 2 * GAP, dpi),
        bottom: top + px(FIELD, dpi),
    }
}

fn arrange() {
    let Some((s, dpi, font)) = with_state(|s| {
        let all = (s.label, s.edit, s.button, s.error, s.hwnd, s.failed);
        (all, s.dpi, s.font)
    }) else {
        return;
    };
    let (label, edit, button, error, hwnd, failed) = s;
    let (f, m, inner) = (field(dpi), px(MARGIN, dpi), px(WIDTH - 2 * MARGIN, dpi));
    let line = line_height(hwnd, font).min(f.bottom - f.top);
    let pad = px(8, dpi);
    unsafe {
        let _ = MoveWindow(label, m, m, inner, px(LABEL, dpi), true);
        let y = f.top + (f.bottom - f.top - line) / 2;
        let w = f.right - f.left - 2 * pad;
        let _ = MoveWindow(edit, f.left + pad, y, w, line, true);
        let x = px(WIDTH - MARGIN - BUTTON, dpi);
        let _ = MoveWindow(button, x, f.top, px(BUTTON, dpi), f.bottom - f.top, true);
        let h = if failed { px(LABEL, dpi) } else { 0 };
        let _ = MoveWindow(error, m, f.bottom + px(GAP, dpi), inner, h, true);
        let size = (px(WIDTH, dpi), height(dpi, failed));
        let flags = SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE;
        let _ = SetWindowPos(hwnd, None, 0, 0, size.0, size.1, flags);
        let _ = InvalidateRect(Some(hwnd), None, true);
    }
}

fn line_height(hwnd: HWND, font: HFONT) -> i32 {
    unsafe {
        let dc = GetDC(Some(hwnd));
        let old = SelectObject(dc, font.into());
        let mut tm = TEXTMETRICW::default();
        let _ = GetTextMetricsW(dc, &mut tm);
        SelectObject(dc, old);
        ReleaseDC(Some(hwnd), dc);
        tm.tmHeight
    }
}

/// After every change of the text.
fn changed(edit: HWND, button: HWND) {
    let text = text_of(edit);
    let rtl = (WS_EX_RIGHT | WS_EX_RTLREADING).0 as isize;
    unsafe {
        let _ = EnableWindow(button, !clean(&text).is_empty());
        let old = GetWindowLongPtrW(edit, GWL_EXSTYLE);
        let new = if is_rtl(&text) { old | rtl } else { old & !rtl };
        // An empty field keeps its side, so clearing it does not jump.
        if new != old && !text.is_empty() {
            SetWindowLongPtrW(edit, GWL_EXSTYLE, new);
            let _ = InvalidateRect(Some(edit), None, true);
        }
    }
}

fn search() {
    let Some((hwnd, edit)) = with_state(|s| (s.hwnd, s.edit)) else {
        return;
    };
    let text = clean(&text_of(edit));
    if text.is_empty() || with_state(|s| s.running.is_some()) == Some(true) {
        return;
    }
    let Some(url) = url(&text) else {
        return fail("The text is too long");
    };
    let (tx, rx) = channel();
    with_state(|s| s.running = Some(rx));
    let target = hwnd.0 as usize;
    // Starting a browser can block.
    std::thread::spawn(move || {
        let _ = tx.send(actions::open_url(&url));
        let hwnd = HWND(target as *mut _);
        let _ = unsafe { PostMessageW(Some(hwnd), WM_DONE, WPARAM(0), LPARAM(0)) };
    });
}

fn fail(text: &str) {
    if let Some((error, edit)) = with_state(|s| {
        s.failed = true;
        (s.error, s.edit)
    }) {
        set_text(error, &format!("Error: {text}"));
        arrange();
        let _ = unsafe { SetFocus(Some(edit)) };
    }
}

fn rgb(c: u32) -> COLORREF {
    COLORREF((c & 255) << 16 | c & 0xFF00 | c >> 16 & 255)
}

fn draw_button(d: &DRAWITEMSTRUCT) {
    let Some((p, dpi)) = with_state(|s| (s.palette, s.dpi)) else {
        return;
    };
    let is = |flag: ODS_FLAGS| d.itemState.0 & flag.0 != 0;
    let (back, text) = if is(ODS_DISABLED) {
        (p.slot, p.disabled)
    } else {
        (p.accent, p.on_accent)
    };
    let mut rc = d.rcItem;
    unsafe {
        let brush = CreateSolidBrush(rgb(back));
        FillRect(d.hDC, &rc, brush);
        let _ = DeleteObject(brush.into());
        if is(ODS_FOCUS) || is(ODS_SELECTED) {
            let brush = CreateSolidBrush(rgb(text));
            let mut line = rc;
            for _ in 0..px(2, dpi) {
                let _ = InflateRect(&mut line, -1, -1);
                FrameRect(d.hDC, &line, brush);
            }
            let _ = DeleteObject(brush.into());
        }
        SetBkMode(d.hDC, TRANSPARENT);
        SetTextColor(d.hDC, rgb(text));
        let mut format = DT_CENTER | DT_VCENTER | DT_SINGLELINE;
        if is(ODS_NOACCEL) {
            format |= DT_HIDEPREFIX;
        }
        let mut label: Vec<u16> = "&Search".encode_utf16().collect();
        DrawTextW(d.hDC, &mut label, &mut rc, format);
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match msg {
        WM_CREATE => return LRESULT(if create(hwnd) { 0 } else { -1 }),
        WM_ERASEBKGND => {
            if let Some((plate, slot, dpi)) = with_state(|s| (s.plate, s.slot, s.dpi)) {
                let mut rc = RECT::default();
                let dc = HDC(w.0 as *mut _);
                unsafe {
                    let _ = GetClientRect(hwnd, &mut rc);
                    FillRect(dc, &rc, plate);
                    FillRect(dc, &field(dpi), slot);
                    if with_state(|s| s.palette.outline) == Some(true) {
                        FrameRect(dc, &field(dpi), GetSysColorBrush(COLOR_WINDOWTEXT));
                        FrameRect(dc, &rc, GetSysColorBrush(COLOR_WINDOWTEXT));
                    }
                }
                return LRESULT(1);
            }
        }
        WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC => {
            if let Some((p, plate, slot)) = with_state(|s| (s.palette, s.plate, s.slot)) {
                let edit = msg == WM_CTLCOLOREDIT;
                let dc = HDC(w.0 as *mut _);
                unsafe {
                    SetTextColor(dc, rgb(p.text));
                    SetBkColor(dc, rgb(if edit { p.slot } else { p.plate }));
                }
                return LRESULT(if edit { slot } else { plate }.0 as isize);
            }
        }
        WM_DRAWITEM => {
            draw_button(unsafe { &*(l.0 as *const DRAWITEMSTRUCT) });
            return LRESULT(1);
        }
        WM_COMMAND => {
            let (id, code) = (w.0 as u16, (w.0 >> 16) as u16);
            match id {
                EDIT if code == EN_CHANGE as u16 => {
                    if let Some((edit, button)) = with_state(|s| (s.edit, s.button)) {
                        changed(edit, button);
                    }
                }
                SEARCH => search(),
                // The dialog manager turns Esc into IDCANCEL.
                id if id == IDCANCEL.0 as u16 => close(),
                _ => {}
            }
            return LRESULT(0);
        }
        WM_ACTIVATE => {
            // The browser takes the focus while it starts; its result closes the box.
            let running = with_state(|s| s.running.is_some()) == Some(true);
            if w.0 as u16 == WA_INACTIVE as u16 && !running {
                close();
            }
        }
        WM_SETFOCUS => {
            if let Some(edit) = with_state(|s| s.edit) {
                let _ = unsafe { SetFocus(Some(edit)) };
            }
        }
        WM_DONE => {
            let result = with_state(|s| s.running.take()).flatten();
            match result.and_then(|rx| rx.try_recv().ok()) {
                Some(Err(e)) => fail(&e),
                _ => close(),
            }
        }
        // Made for one scale; opening it again is quick.
        WM_DPICHANGED => close(),
        WM_NCDESTROY => {
            if let Some(s) = STATE.with(|s| s.borrow_mut().take()) {
                unsafe {
                    let _ = DeleteObject(s.font.into());
                    let _ = DeleteObject(s.plate.into());
                    let _ = DeleteObject(s.slot.into());
                }
            }
        }
        _ => {}
    }
    unsafe { DefWindowProcW(hwnd, msg, w, l) }
}

/// The copied text, or nothing. Read once, off this thread.
fn copied() -> String {
    let (tx, rx) = channel();
    std::thread::spawn(move || tx.send(clipboard().unwrap_or_default()));
    let text = rx.recv_timeout(CLIPBOARD_TIME).unwrap_or_default();
    let text = clean(&text);
    // Part of a long text is not what was copied.
    if text.encode_utf16().count() > MAX_TEXT {
        String::new()
    } else {
        text
    }
}

fn clipboard() -> Option<String> {
    unsafe {
        OpenClipboard(None).ok()?;
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

/// True when the first letter belongs to a right-to-left script.
fn is_rtl(text: &str) -> bool {
    let first = text.chars().find(|c| c.is_alphabetic());
    matches!(
        first,
        Some('\u{590}'..='\u{8FF}' | '\u{FB1D}'..='\u{FDFF}' | '\u{FE70}'..='\u{FEFF}')
    )
}

fn url(text: &str) -> Option<String> {
    let mut url = "https://www.google.com/search?q=".to_string();
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
        // Every Arabic text the field can hold fits.
        assert!(url(&"م".repeat(MAX_TEXT)).is_some());
        assert!(url(&"漢".repeat(MAX_TEXT)).is_none());
    }

    #[test]
    fn one_line() {
        assert_eq!(clean("  a\r\n\tb\u{0}  c "), "a b c");
        assert_eq!(clean(" \r\n "), "");
    }

    #[test]
    fn sides() {
        assert!(is_rtl("123 مدار abc"));
        assert!(!is_rtl("abc مدار"));
        assert!(!is_rtl("123"));
    }
}
