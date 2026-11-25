//! Chooses a program from the Start menu's list, Store apps included.

use super::*;
use std::process::{Command, Stdio};
use windows::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoTaskMemFree,
};
use windows::Win32::UI::Shell::{
    BHID_EnumItems, FOLDERID_AppsFolder, IEnumShellItems, IShellItem, KF_FLAG_DEFAULT,
    SHGetKnownFolderItem, SIGDN, SIGDN_NORMALDISPLAY, SIGDN_PARENTRELATIVEPARSING,
};

/// Argument of the child process that lists the programs, so that the shell
/// libraries it loads do not stay in memory.
pub const CHILD: &str = "--apps";
const PICKER_CLASS: PCWSTR = w!("MadarStartMenu");
const PICKER_STYLE: WINDOW_STYLE =
    WINDOW_STYLE(WS_POPUP.0 | WS_CAPTION.0 | WS_SYSMENU.0 | WS_THICKFRAME.0);
const PICKER_W: i32 = 400;
const PICKER_H: i32 = 440;
const PICKER_MIN: i32 = 300;
const ID_SEARCH: u16 = 400;
const ID_LIST: u16 = 401;

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub name: String,
    /// Full path for `App::path`.
    pub path: String,
}

struct Picker {
    hwnd: HWND,
    dpi: u32,
    font: HFONT,
    label: HWND,
    search: HWND,
    list: HWND,
    note: HWND,
    ok: HWND,
    cancel: HWND,
    entries: Vec<Entry>,
    shown: Vec<usize>,
    done: bool,
    chosen: Option<Entry>,
}

thread_local! {
    static PICKER: RefCell<Option<Picker>> = const { RefCell::new(None) };
    static INIT: Cell<u32> = const { Cell::new(96) };
}

fn with_picker<R>(f: impl FnOnce(&mut Picker) -> R) -> Option<R> {
    PICKER.with(|p| p.try_borrow_mut().ok()?.as_mut().map(f))
}

/// The child's work: "id, NUL, name, NUL" per program on stdout.
pub fn child() -> windows::core::Result<()> {
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
        let folder: IShellItem = SHGetKnownFolderItem(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, None)?;
        let items: IEnumShellItems = folder.BindToHandler(None, &BHID_EnumItems)?;
        loop {
            let mut next = [None];
            let mut got = 0;
            items.Next(&mut next, Some(&mut got))?;
            let Some(item) = next[0].take().filter(|_| got == 1) else {
                break;
            };
            let text = |kind: SIGDN| -> windows::core::Result<String> {
                let p = item.GetDisplayName(kind)?;
                let s = String::from_utf16_lossy(p.as_wide());
                CoTaskMemFree(Some(p.0.cast()));
                Ok(s)
            };
            let (Ok(id), Ok(name)) = (text(SIGDN_PARENTRELATIVEPARSING), text(SIGDN_NORMALDISPLAY))
            else {
                continue;
            };
            let _ = write!(out, "{id}\0{name}\0");
        }
    }
    let _ = out.flush();
    Ok(())
}

fn parse(bytes: &[u8]) -> Vec<Entry> {
    let text = String::from_utf8_lossy(bytes);
    let mut parts = text.split('\0');
    let mut out = Vec::new();
    while let (Some(id), Some(name)) = (parts.next(), parts.next()) {
        let path = format!("{}{id}", actions::START_APPS);
        if actions::is_start_app(&path) && !name.trim().is_empty() {
            out.push(Entry {
                name: name.trim().to_string(),
                path,
            });
        }
    }
    out.sort_by_key(|e| e.name.to_lowercase());
    out
}

fn entries() -> Vec<Entry> {
    let listed = std::env::current_exe().and_then(|exe| {
        Command::new(exe)
            .arg(CHILD)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
    });
    listed.map_or_else(|_| Vec::new(), |o| parse(&o.stdout))
}

fn matching(entries: &[Entry], text: &str) -> Vec<usize> {
    let text = text.trim().to_lowercase();
    (entries.iter().enumerate())
        .filter(|(_, e)| e.name.to_lowercase().contains(&text))
        .map(|(i, _)| i)
        .collect()
}

/// Modal. None when the user cancels.
pub fn pick(owner: HWND) -> Option<Entry> {
    unsafe {
        let module = GetModuleHandleW(None).ok()?;
        let wait = SetCursor(LoadCursorW(None, IDC_WAIT).ok());
        let entries = entries();
        SetCursor(Some(wait));
        let class = WNDCLASSW {
            lpfnWndProc: Some(picker_proc),
            hInstance: module.into(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hbrBackground: GetSysColorBrush(COLOR_BTNFACE),
            lpszClassName: PICKER_CLASS,
            ..Default::default()
        };
        RegisterClassW(&class);
        let dpi = GetDpiForWindow(owner).max(96);
        INIT.set(dpi);
        let (w, h) = (px(PICKER_W, dpi), px(PICKER_H, dpi));
        let mut own = RECT::default();
        let _ = GetWindowRect(owner, &mut own);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let _ = GetMonitorInfoW(
            MonitorFromWindow(owner, MONITOR_DEFAULTTONEAREST),
            &mut info,
        );
        let area = info.rcWork;
        // The last `max` wins, so the title bar stays on screen.
        let x = ((own.left + own.right - w) / 2)
            .min(area.right - w)
            .max(area.left);
        let y = ((own.top + own.bottom - h) / 2)
            .min(area.bottom - h)
            .max(area.top);
        let hwnd = CreateWindowExW(
            FORM_EX,
            PICKER_CLASS,
            w!("Choose from the Start menu"),
            PICKER_STYLE,
            x,
            y,
            w,
            h,
            Some(owner),
            None,
            Some(module.into()),
            None,
        )
        .ok()?;
        with_picker(|p| p.entries = entries);
        show_matches();
        use_dark_title(hwnd);
        let _ = EnableWindow(owner, false);
        let _ = ShowWindow(hwnd, SW_SHOW);
        if let Some(search) = with_picker(|p| p.search) {
            let _ = SetFocus(Some(search));
        }
        run(hwnd, owner);
        let chosen = with_picker(|p| p.chosen.take()).flatten();
        // The owner is usable again before this window goes away.
        if IsWindow(Some(owner)).as_bool() {
            let _ = EnableWindow(owner, true);
            let _ = SetFocus(Some(owner));
        }
        if IsWindow(Some(hwnd)).as_bool() {
            let _ = DestroyWindow(hwnd);
        }
        chosen
    }
}

fn run(hwnd: HWND, owner: HWND) {
    let mut msg = MSG::default();
    unsafe {
        while IsWindow(Some(hwnd)).as_bool() && IsWindow(Some(owner)).as_bool() {
            if with_picker(|p| p.done).unwrap_or(true) {
                break;
            }
            if GetMessageW(&mut msg, None, 0, 0).0 <= 0 {
                if msg.message == WM_QUIT {
                    PostQuitMessage(msg.wParam.0 as i32);
                }
                break;
            }
            if msg.message == WM_KEYDOWN && key(&msg) {
                continue;
            }
            if !IsDialogMessageW(hwnd, &msg).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}

/// Enter, Esc, and the list's keys while the search box has the focus.
fn key(msg: &MSG) -> bool {
    let Some((search, list, cancel)) = with_picker(|p| (p.search, p.list, p.cancel)) else {
        return false;
    };
    match VIRTUAL_KEY(msg.wParam.0 as u16) {
        VK_ESCAPE => {
            with_picker(|p| p.done = true);
        }
        VK_RETURN if msg.hwnd == cancel => {
            with_picker(|p| p.done = true);
        }
        VK_RETURN => choose(),
        VK_UP | VK_DOWN | VK_PRIOR | VK_NEXT if msg.hwnd == search => {
            send(list, msg.message, msg.wParam.0, msg.lParam.0);
        }
        _ => return false,
    }
    true
}

fn choose() {
    let Some(list) = with_picker(|p| p.list) else {
        return;
    };
    let Some(sel) = list_sel(list) else { return };
    with_picker(|p| {
        p.chosen = p.shown.get(sel).map(|&i| p.entries[i].clone());
        p.done = p.chosen.is_some();
    });
}

fn show_matches() {
    let Some((search, list, note, ok)) = with_picker(|p| (p.search, p.list, p.note, p.ok)) else {
        return;
    };
    let text = text_of(search);
    let names = with_picker(|p| {
        p.shown = matching(&p.entries, &text);
        let name = |&i: &usize| p.entries[i].name.clone();
        p.shown.iter().map(name).collect::<Vec<_>>()
    })
    .unwrap_or_default();
    send(list, WM_SETREDRAW, 0, 0);
    fill(list, names.iter());
    send(list, LB_SETCURSEL, 0, 0);
    send(list, WM_SETREDRAW, 1, 0);
    repaint(list);
    let none = with_picker(|p| p.entries.is_empty()).unwrap_or(true);
    set_text(
        note,
        match (none, names.is_empty()) {
            (true, _) => "The list could not be read.",
            (_, true) => "No program found.",
            _ => "",
        },
    );
    let _ = unsafe { EnableWindow(ok, !names.is_empty()) };
}

fn create(hwnd: HWND) -> bool {
    let dpi = INIT.get();
    let label = WS_CHILD | WS_VISIBLE;
    let list_bits = list_style() | WINDOW_STYLE(LBS_NOINTEGRALHEIGHT as u32);
    let make = |class, text, style, id| unsafe { make(hwnd, class, text, style, id) };
    let p = Picker {
        hwnd,
        dpi,
        font: message_font(dpi),
        label: make(w!("STATIC"), "&Search", label | SS_CENTER, 0),
        search: make(w!("EDIT"), "", edit_style(false), ID_SEARCH),
        list: make(w!("LISTBOX"), "", list_bits, ID_LIST),
        note: make(w!("STATIC"), "", label | SS_CENTER, 0),
        ok: make(w!("BUTTON"), "OK", button_style(true), IDOK.0 as u16),
        cancel: make(
            w!("BUTTON"),
            "Cancel",
            button_style(false),
            IDCANCEL.0 as u16,
        ),
        entries: Vec::new(),
        shown: Vec::new(),
        done: false,
        chosen: None,
    };
    let all = [p.label, p.search, p.list, p.note, p.ok, p.cancel];
    let (search, font) = (p.search, p.font);
    let complete = all.iter().all(|h| !h.is_invalid());
    PICKER.with(|slot| *slot.borrow_mut() = Some(p));
    for h in all {
        send(h, WM_SETFONT, font.0 as usize, 1);
    }
    send(search, EM_SETLIMITTEXT, NAME_MAX, 0);
    // A failed create ends in WM_NCDESTROY, which frees the rest.
    complete
}

fn arrange() {
    let Some((hwnd, dpi)) = with_picker(|p| (p.hwnd, p.dpi)) else {
        return;
    };
    let mut rc = RECT::default();
    if unsafe { GetClientRect(hwnd, &mut rc) }.is_err() {
        return;
    }
    let at = |v| px(v, dpi);
    let (m, inner) = (at(MARGIN), rc.right - 2 * at(MARGIN));
    let (label, btn, gap) = (at(60), at(FORM_BTN_W), at(GAP));
    let top = m + at(ROW);
    let by = rc.bottom - m - at(BTN_H);
    let plan = with_picker(|p| {
        [
            (p.label, m, m, label, at(CTL_H)),
            (p.search, m + label, m, inner - label, at(CTL_H)),
            (p.list, m, top, inner, (by - at(12) - top).max(0)),
            (p.note, m, by, inner - 2 * btn - 2 * gap, at(BTN_H)),
            (p.ok, rc.right - m - 2 * btn - gap, by, btn, at(BTN_H)),
            (p.cancel, rc.right - m - btn, by, btn, at(BTN_H)),
        ]
    });
    for (h, x, y, w, h2) in plan.into_iter().flatten() {
        let _ = unsafe { MoveWindow(h, x, y, w, h2, true) };
    }
    repaint(hwnd);
}

fn change_dpi(dpi: u32) {
    let font = message_font(dpi);
    let old = with_picker(|p| {
        p.dpi = dpi;
        replace(&mut p.font, font)
    });
    let all = with_picker(|p| [p.label, p.search, p.list, p.note, p.ok, p.cancel]);
    for h in all.into_iter().flatten() {
        send(h, WM_SETFONT, font.0 as usize, 1);
    }
    if let Some(old) = old.filter(|f| !f.is_invalid()) {
        let _ = unsafe { DeleteObject(old.into()) };
    }
    arrange();
}

unsafe extern "system" fn picker_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match msg {
        WM_CREATE if !create(hwnd) => return LRESULT(-1),
        WM_SIZE => arrange(),
        WM_GETMINMAXINFO => {
            let dpi = with_picker(|p| p.dpi).unwrap_or(96);
            let info = unsafe { &mut *(l.0 as *mut MINMAXINFO) };
            (info.ptMinTrackSize.x, info.ptMinTrackSize.y) =
                (px(PICKER_MIN, dpi), px(PICKER_MIN, dpi));
            return LRESULT(0);
        }
        WM_COMMAND => {
            let (id, code) = (loword(w.0 as u32), hiword(w.0 as u32) as u32);
            match id {
                ID_SEARCH if code == EN_CHANGE => show_matches(),
                ID_LIST if code == LBN_DBLCLK => choose(),
                id if id == IDOK.0 as u16 => choose(),
                id if id == IDCANCEL.0 as u16 => {
                    with_picker(|p| p.done = true);
                }
                _ => {}
            }
        }
        WM_CLOSE => {
            with_picker(|p| p.done = true);
            return LRESULT(0);
        }
        WM_DPICHANGED => {
            let r = unsafe { *(l.0 as *const RECT) };
            let (cx, cy) = (r.right - r.left, r.bottom - r.top);
            let keep = SWP_NOZORDER | SWP_NOACTIVATE;
            let _ = unsafe { SetWindowPos(hwnd, None, r.left, r.top, cx, cy, keep) };
            change_dpi((w.0 & 0xFFFF) as u32);
        }
        WM_NCDESTROY => {
            let gone = PICKER.with(|p| p.try_borrow_mut().ok().and_then(|mut p| p.take()));
            if let Some(p) = gone.filter(|p| !p.font.is_invalid()) {
                let _ = unsafe { DeleteObject(p.font.into()) };
            }
        }
        _ => {}
    }
    unsafe { DefWindowProcW(hwnd, msg, w, l) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sorts_and_searches() {
        let all =
            parse("b!App\0zebra\0C\\x.exe\0 Apple \0bad\0\0\0no id\0تطبيق\0متصفح\0".as_bytes());
        let names: Vec<_> = all.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["Apple", "zebra", "متصفح"]);
        assert_eq!(all[1].path, "shell:AppsFolder\\b!App");
        assert_eq!(matching(&all, " ZEB "), [1]);
        assert_eq!(matching(&all, "صفح"), [2]);
        assert_eq!(matching(&all, "").len(), 3);
        assert!(matching(&all, "nothing").is_empty());
        assert!(parse(b"").is_empty());
    }
}
