//! Draws the settings windows in the colors of the wheel's theme.
//!
//! The controls stay native, so their keys and names for screen readers do not
//! change; only their painting is replaced, through documented messages.

use super::*;
use crate::wheel::paint::{Palette, luminance, mix, palette, theme};
use windows::Win32::Graphics::Dwm::{
    DWMWA_BORDER_COLOR, DWMWA_CAPTION_COLOR, DWMWA_COLOR_DEFAULT, DWMWA_TEXT_COLOR,
    DWMWINDOWATTRIBUTE,
};
use windows::Win32::UI::Controls::*;

const FRAME: usize = 2;
const ITEM_H: i32 = 20;
/// What a combo box adds around its item.
const COMBO_EDGE: i32 = 6;

thread_local! {
    static THEME: Cell<usize> = const { Cell::new(1) };
    static COLORS: Cell<Option<Palette>> = const { Cell::new(None) };
}

fn colors() -> Palette {
    COLORS.get().unwrap_or_else(|| theme(THEMES[THEME.get()]))
}

/// True when the colors changed and the open windows need `recolor`.
pub fn set(name: &str) -> bool {
    THEME.set(theme_index(name));
    reload()
}

/// The same theme again, after Windows High Contrast went on or off.
pub fn reload() -> bool {
    let new = palette(THEMES[THEME.get()]);
    COLORS.replace(Some(new)) != Some(new)
}

fn border(p: Palette) -> u32 {
    mix(p.text, p.slot, 0.5)
}

fn hover(p: Palette) -> u32 {
    mix(p.text, p.slot, 0.12)
}

fn rgb(c: u32) -> COLORREF {
    COLORREF((c & 255) << 16 | c & 0xFF00 | c >> 16 & 255)
}

/// The stock brush in color `c`, good until the next call on this `dc`.
fn brush(dc: HDC, c: u32) -> HBRUSH {
    unsafe {
        SetDCBrushColor(dc, rgb(c));
        HBRUSH(GetStockObject(DC_BRUSH).0)
    }
}

fn fill(dc: HDC, rc: &RECT, c: u32) {
    let b = brush(dc, c);
    unsafe { FillRect(dc, rc, b) };
}

fn frame(dc: HDC, rc: &RECT, width: i32, edge: u32, inside: u32) {
    fill(dc, rc, edge);
    fill(dc, &inset(rc, width), inside);
}

fn inset(rc: &RECT, by: i32) -> RECT {
    RECT {
        left: rc.left + by,
        top: rc.top + by,
        right: rc.right - by,
        bottom: rc.bottom - by,
    }
}

fn class_is(h: HWND, name: &str) -> bool {
    let mut buf = [0u16; 32];
    let n = unsafe { GetClassNameW(h, &mut buf) }.max(0) as usize;
    String::from_utf16_lossy(&buf[..n]).eq_ignore_ascii_case(name)
}

fn dpi_of(h: HWND) -> u32 {
    unsafe { GetDpiForWindow(h) }.max(96)
}

fn focused(h: HWND) -> bool {
    unsafe { GetFocus() == h }
}

/// Sets a control's font and takes over its painting.
pub fn font(h: HWND, font: HFONT) {
    send(h, WM_SETFONT, font.0 as usize, 1);
    let dpi = dpi_of(h);
    let item = px(ITEM_H, dpi) as isize;
    let list = class_is(h, "ListBox");
    unsafe {
        if class_is(h, "ComboBox") {
            // Without its visual style it has no fades of its own to draw.
            let _ = SetWindowTheme(h, w!(""), w!(""));
            let _ = SetWindowSubclass(h, Some(combo_proc), FRAME, 0);
            send(h, CB_SETITEMHEIGHT, 0, item);
            let edit = px(CTL_H, dpi) - COMBO_EDGE;
            send(h, CB_SETITEMHEIGHT, usize::MAX, edit as isize);
        } else if list || class_is(h, "Edit") {
            let _ = SetWindowSubclass(h, Some(frame_proc), FRAME, 0);
        }
        if list {
            send(h, LB_SETITEMHEIGHT, 0, item);
        } else if class_is(h, "Edit") {
            let same = SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE;
            let _ = SetWindowPos(h, None, 0, 0, 0, 0, same | SWP_FRAMECHANGED);
        }
    }
}

/// The part of an edit's window that its line of text takes.
fn text_room(h: HWND, rc: &RECT) -> RECT {
    let mut m = TEXTMETRICW::default();
    unsafe {
        let dc = GetDC(Some(h));
        let font = HGDIOBJ(send(h, WM_GETFONT, 0, 0) as *mut core::ffi::c_void);
        let old = SelectObject(dc, font);
        let _ = GetTextMetricsW(dc, &mut m);
        SelectObject(dc, old);
        ReleaseDC(Some(h), dc);
    }
    centered(rc, m.tmHeight, px(4, dpi_of(h)))
}

/// `rc` with `side` off the left and right, and a line of `line` in the middle.
fn centered(rc: &RECT, line: i32, side: i32) -> RECT {
    // The border is never covered, however tall the line.
    let spare = (rc.bottom - rc.top - line).max(2);
    let top = rc.top + spare / 2;
    RECT {
        left: rc.left + side,
        top,
        right: (rc.right - side).max(rc.left + side),
        bottom: (rc.bottom - (spare - spare / 2)).max(top),
    }
}

/// Title bar, scroll bars and a full repaint in the current colors.
pub fn show(hwnd: HWND) {
    let p = colors();
    let dark = !p.outline && luminance(p.plate) < luminance(p.text);
    let title = |what: DWMWINDOWATTRIBUTE, value: u32| unsafe {
        let size = size_of::<u32>() as u32;
        let _ = DwmSetWindowAttribute(hwnd, what, (&raw const value).cast(), size);
    };
    let own = |c| {
        if p.outline {
            DWMWA_COLOR_DEFAULT
        } else {
            rgb(c).0
        }
    };
    title(DWMWA_USE_IMMERSIVE_DARK_MODE, u32::from(dark));
    title(DWMWA_CAPTION_COLOR, own(p.plate));
    title(DWMWA_TEXT_COLOR, own(p.text));
    title(DWMWA_BORDER_COLOR, own(border(p)));
    unsafe {
        let mut child = GetWindow(hwnd, GW_CHILD).unwrap_or_default();
        while !child.is_invalid() {
            if class_is(child, "ListBox") {
                let style = if dark {
                    w!("DarkMode_Explorer")
                } else {
                    w!("Explorer")
                };
                let _ = SetWindowTheme(child, style, None);
            }
            child = GetWindow(child, GW_HWNDNEXT).unwrap_or_default();
        }
        let flags = RDW_INVALIDATE | RDW_ERASE | RDW_FRAME | RDW_ALLCHILDREN;
        let _ = RedrawWindow(Some(hwnd), None, None, flags);
    }
}

/// The messages with which a window paints itself and its controls.
pub fn handle(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> Option<LRESULT> {
    let p = colors();
    let dc = HDC(w.0 as *mut core::ffi::c_void);
    match msg {
        WM_ERASEBKGND => {
            let mut rc = RECT::default();
            unsafe { GetClientRect(hwnd, &mut rc) }.ok()?;
            fill(dc, &rc, p.plate);
            Some(LRESULT(1))
        }
        WM_CTLCOLORSTATIC | WM_CTLCOLORBTN | WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX => {
            // A read-only edit asks like a label.
            let field = msg == WM_CTLCOLOREDIT
                || msg == WM_CTLCOLORLISTBOX
                || class_is(HWND(l.0 as *mut core::ffi::c_void), "Edit");
            let back = if field { p.slot } else { p.plate };
            unsafe {
                SetTextColor(dc, rgb(p.text));
                SetBkColor(dc, rgb(back));
            }
            Some(LRESULT(brush(dc, back).0 as isize))
        }
        WM_DRAWITEM => {
            let d = unsafe { &*(l.0 as *const DRAWITEMSTRUCT) };
            item(d, p);
            Some(LRESULT(1))
        }
        WM_NOTIFY => {
            let hdr = unsafe { &*(l.0 as *const NMHDR) };
            if hdr.code != NM_CUSTOMDRAW || !class_is(hdr.hwndFrom, "Button") {
                return None;
            }
            let d = unsafe { &*(l.0 as *const NMCUSTOMDRAW) };
            if d.dwDrawStage != CDDS_PREPAINT {
                return Some(LRESULT(CDRF_DODEFAULT as isize));
            }
            button(d, p);
            Some(LRESULT(CDRF_SKIPDEFAULT as isize))
        }
        _ => None,
    }
}

/// `text` in the control's font on one line.
fn text(h: HWND, dc: HDC, text: &str, rc: &RECT, c: u32, format: DRAW_TEXT_FORMAT) {
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    let mut rc = *rc;
    let line = DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS;
    unsafe {
        let font = HGDIOBJ(send(h, WM_GETFONT, 0, 0) as *mut core::ffi::c_void);
        let old = SelectObject(dc, font);
        SetTextColor(dc, rgb(c));
        SetBkMode(dc, TRANSPARENT);
        DrawTextW(dc, &mut wide, &mut rc, format | line);
        SelectObject(dc, old);
    }
}

fn lines(dc: HDC, points: &[POINT], width: i32, c: u32) {
    unsafe {
        let pen = CreatePen(PS_SOLID, width, rgb(c));
        let old = SelectObject(dc, pen.into());
        let _ = Polyline(dc, points);
        SelectObject(dc, old);
        let _ = DeleteObject(pen.into());
    }
}

fn button(d: &NMCUSTOMDRAW, p: Palette) {
    let h = d.hdr.hwndFrom;
    let (dc, rc, dpi) = (d.hdc, d.rc, dpi_of(h));
    let style = unsafe { GetWindowLongW(h, GWL_STYLE) };
    let is = |flag: NMCUSTOMDRAW_DRAW_STATE_FLAGS| d.uItemState.0 & flag.0 != 0;
    let (off, hot, pressed) = (is(CDIS_DISABLED), is(CDIS_HOT), is(CDIS_SELECTED));
    let focus = focused(h);
    let checked = send(h, BM_GETCHECK, 0, 0) == BST_CHECKED.0 as isize;
    let mut format = DT_CENTER;
    if send(h, WM_QUERYUISTATE, 0, 0) as u32 & UISF_HIDEACCEL != 0 {
        format |= DT_HIDEPREFIX;
    }
    let label = text_of(h);
    let thick = px(2, dpi);
    if style & BS_TYPEMASK == BS_AUTOCHECKBOX {
        fill(dc, &rc, p.plate);
        let side = px(14, dpi);
        let top = (rc.top + rc.bottom - side) / 2;
        // Room for the focus frame on the left.
        let left = rc.left + px(4, dpi);
        let box_ = RECT {
            left,
            top,
            right: left + side,
            bottom: top + side,
        };
        let inside = match (checked, hot || pressed) {
            (true, _) => p.accent,
            (false, true) => hover(p),
            (false, false) => p.slot,
        };
        let edge = if checked { p.accent } else { border(p) };
        frame(dc, &box_, 1, edge, inside);
        if checked {
            let at = |x: i32, y: i32| POINT {
                x: left + px(x, dpi),
                y: top + px(y, dpi),
            };
            lines(dc, &[at(3, 7), at(6, 10), at(11, 4)], thick, p.on_accent);
        }
        let rest = RECT {
            left: box_.right + px(6, dpi),
            ..rc
        };
        let align = DRAW_TEXT_FORMAT(format.0 & !DT_CENTER.0);
        text(h, dc, &label, &rest, p.text, align);
        if focus {
            unsafe { FrameRect(dc, &rc, brush(dc, p.accent)) };
        }
        return;
    }
    let on = style & BS_PUSHLIKE != 0 && checked;
    let default = style & BS_TYPEMASK == BS_DEFPUSHBUTTON;
    let (inside, ink) = match (off, on) {
        (true, _) => (p.plate, p.disabled),
        (_, true) => (p.accent, p.on_accent),
        _ if pressed => (p.plate, p.text),
        _ if hot => (hover(p), p.text),
        _ => (p.slot, p.text),
    };
    let edge = match (off, on || focus || default) {
        (true, _) => mix(p.text, p.plate, 0.25),
        (_, true) => p.accent,
        _ => border(p),
    };
    // One device pixel, like the border Windows gives an edit.
    let width = if focus { thick } else { 1 };
    frame(dc, &rc, width, edge, inside);
    if on && focus {
        let inner = inset(&rc, thick + 1);
        unsafe { FrameRect(dc, &inner, brush(dc, p.on_accent)) };
    }
    text(h, dc, &label, &rc, ink, format);
}

/// A row of a list or of an open combo box.
fn item(d: &DRAWITEMSTRUCT, p: Palette) {
    let h = d.hwndItem;
    let combo = d.CtlType == ODT_COMBOBOX;
    let closed = combo && d.itemState.0 & ODS_COMBOBOXEDIT.0 != 0;
    let chosen = d.itemState.0 & ODS_SELECTED.0 != 0 && !closed;
    let (back, ink) = if chosen {
        (p.accent, p.on_accent)
    } else {
        (p.slot, p.text)
    };
    fill(d.hDC, &d.rcItem, back);
    if d.itemID != u32::MAX {
        let get = if combo { CB_GETLBTEXT } else { LB_GETTEXT };
        let len = if combo {
            CB_GETLBTEXTLEN
        } else {
            LB_GETTEXTLEN
        };
        row(h, d.hDC, d.itemID as usize, (get, len), &d.rcItem, ink);
    }
    if closed {
        let _ = unsafe { InvalidateRect(Some(h), None, false) };
    }
}

fn row(h: HWND, dc: HDC, index: usize, (get, len): (u32, u32), rc: &RECT, ink: u32) {
    let Ok(len) = usize::try_from(send(h, len, index, 0)) else {
        return;
    };
    let mut buf = vec![0u16; len + 1];
    let got = send(h, get, index, buf.as_mut_ptr() as isize).max(0) as usize;
    let name = String::from_utf16_lossy(&buf[..got.min(len)]);
    let mut room = *rc;
    room.left += px(4, dpi_of(h));
    text(h, dc, &name, &room, ink, DT_NOPREFIX);
}

/// The border of an edit or a list.
unsafe extern "system" fn frame_proc(
    h: HWND,
    msg: u32,
    w: WPARAM,
    l: LPARAM,
    id: usize,
    _: usize,
) -> LRESULT {
    unsafe {
        let edit = class_is(h, "Edit");
        if edit && msg == WM_NCCALCSIZE {
            // Both forms of this message start with the window's rectangle.
            let rc = &mut *(l.0 as *mut RECT);
            *rc = text_room(h, rc);
            return LRESULT(0);
        }
        let done = DefSubclassProc(h, msg, w, l);
        match msg {
            // A click beside the text is still a click in the box.
            WM_NCHITTEST if edit => return LRESULT(HTCLIENT as isize),
            WM_NCPAINT | WM_PAINT => {
                let p = colors();
                let dc = GetWindowDC(Some(h));
                let mut rc = RECT::default();
                let _ = GetWindowRect(h, &mut rc);
                let own = RECT {
                    left: 0,
                    top: 0,
                    right: rc.right - rc.left,
                    bottom: rc.bottom - rc.top,
                };
                let edge = if focused(h) { p.accent } else { border(p) };
                if edit {
                    let room = text_room(h, &own);
                    ExcludeClipRect(dc, room.left, room.top, room.right, room.bottom);
                    frame(dc, &own, 1, edge, p.slot);
                } else {
                    FrameRect(dc, &own, brush(dc, edge));
                }
                ReleaseDC(Some(h), dc);
            }
            WM_SETFOCUS | WM_KILLFOCUS => {
                let flags = RDW_FRAME | RDW_INVALIDATE;
                let _ = RedrawWindow(Some(h), None, None, flags);
            }
            WM_NCDESTROY => {
                let _ = RemoveWindowSubclass(h, Some(frame_proc), id);
            }
            _ => {}
        }
        done
    }
}

/// A closed combo box.
unsafe extern "system" fn combo_proc(
    h: HWND,
    msg: u32,
    w: WPARAM,
    l: LPARAM,
    id: usize,
    _: usize,
) -> LRESULT {
    unsafe {
        match msg {
            WM_ERASEBKGND => return LRESULT(1),
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let dc = BeginPaint(h, &mut ps);
                combo(h, dc);
                let _ = EndPaint(h, &ps);
                return LRESULT(0);
            }
            WM_NCDESTROY => {
                let _ = RemoveWindowSubclass(h, Some(combo_proc), id);
            }
            _ => {}
        }
        let done = DefSubclassProc(h, msg, w, l);
        // After these the control draws by itself, outside WM_PAINT.
        if matches!(
            msg,
            WM_SETFOCUS
                | WM_KILLFOCUS
                | WM_LBUTTONDOWN
                | WM_LBUTTONUP
                | WM_CAPTURECHANGED
                | WM_KEYDOWN
                | WM_COMMAND
                | CB_SETCURSEL
                | CB_SHOWDROPDOWN
        ) {
            let _ = InvalidateRect(Some(h), None, false);
        }
        done
    }
}

fn combo(h: HWND, dc: HDC) {
    let p = colors();
    let dpi = dpi_of(h);
    let mut rc = RECT::default();
    if unsafe { GetClientRect(h, &mut rc) }.is_err() {
        return;
    }
    let (width, edge) = if focused(h) {
        (px(2, dpi), p.accent)
    } else {
        (1, border(p))
    };
    frame(dc, &rc, width, edge, p.slot);
    let arrow = RECT {
        left: rc.right - px(24, dpi),
        ..rc
    };
    let (x, y) = (
        (arrow.left + arrow.right) / 2,
        (rc.top + rc.bottom) / 2 - px(2, dpi),
    );
    let wing = px(4, dpi);
    let at = |x, y| POINT { x, y };
    let chevron = [at(x - wing, y), at(x, y + wing), at(x + wing, y)];
    lines(dc, &chevron, px(1, dpi), p.text);
    if let Some(i) = combo_sel(h) {
        let room = RECT {
            left: rc.left + px(2, dpi),
            right: arrow.left,
            ..rc
        };
        let text = (CB_GETLBTEXT, CB_GETLBTEXTLEN);
        row(h, dc, i, text, &room, p.text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wheel::paint::contrast_of;

    #[test]
    fn every_theme_is_readable() {
        for name in THEMES {
            let p = theme(name);
            let least = |what: &str, a: u32, b: u32, least: f32| {
                let c = contrast_of(luminance(a), luminance(b));
                assert!(c >= least, "{name} {what} {c}");
            };
            least("label", p.text, p.plate, 4.5);
            least("field", p.text, p.slot, 4.5);
            least("hover", p.text, hover(p), 4.5);
            least("chosen", p.on_accent, p.accent, 4.5);
            least("border", border(p), p.plate, 3.0);
            least("focus", p.accent, p.plate, 3.0);
            least("focus in list", p.accent, p.slot, 3.0);
            least("disabled", p.disabled, p.plate, 3.0);
        }
    }

    #[test]
    fn text_sits_in_the_middle() {
        let rc = RECT {
            left: 10,
            top: 100,
            right: 110,
            bottom: 136,
        };
        let r = centered(&rc, 25, 6);
        assert_eq!((r.left, r.top, r.right, r.bottom), (16, 105, 104, 130));
        // A line taller than the box, and a box with no size yet.
        let r = centered(&rc, 60, 6);
        assert_eq!((r.top, r.bottom), (101, 135));
        let r = centered(&RECT::default(), 25, 6);
        assert!(r.right >= r.left && r.bottom >= r.top);
    }

    #[test]
    fn colors_swap_to_windows_order() {
        assert_eq!(rgb(0x123456).0, 0x563412);
    }
}
