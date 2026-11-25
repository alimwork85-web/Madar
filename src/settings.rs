//! The settings window: one column of native controls over an injected `Host`.
//!
//! Everything that can re-enter this module (host calls, message boxes, file
//! dialogs, `SendMessageW` to our own controls, `DestroyWindow`, modal loops)
//! runs with no borrow of the state held: the state is copied out first.

pub mod start_menu;

use crate::actions;
use crate::config::{self, App, Config, Mouse, Site, THEMES};
use std::cell::{Cell, RefCell};
use std::mem::replace;
use std::path::Path;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::{DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_VALUE_TYPE, RRF_RT_REG_DWORD, RegGetValueW,
};
use windows::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
use windows::Win32::UI::Controls::Dialogs::*;
use windows::Win32::UI::Controls::{BST_CHECKED, BST_UNCHECKED, EM_SETLIMITTEXT};
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, PCWSTR, PWSTR, w};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Field {
    Hotkey,
    Mouse,
    Theme,
    Autostart,
    Names,
    MyApps,
    Apps,
    Sites,
}

impl Field {
    fn slot(self) -> usize {
        match self {
            Self::Hotkey => 0,
            Self::Mouse => 1,
            Self::Theme => 2,
            Self::Autostart => 3,
            Self::Names => 4,
            // One list is shown at a time.
            Self::MyApps | Self::Apps | Self::Sites => 5,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Problem {
    pub field: Field,
    pub text: String,
}

/// The window never reads or writes files or the registry; it goes through these.
#[derive(Clone, Copy)]
pub struct Host {
    pub get: fn() -> Config,
    /// Saves and applies. On Err nothing changed. May show its own message box.
    pub apply: fn(Config) -> Result<(), Problem>,
}

const CLASS: PCWSTR = w!("MadarSettings");
const TITLE: PCWSTR = w!("Madar settings");
const FORM_CLASS: PCWSTR = w!("MadarSettingsItem");
/// Resource 1 by number, the same icon the tray uses.
const ICON: PCWSTR = PCWSTR(core::ptr::without_provenance(1));
const FORM_STYLE: WINDOW_STYLE = WINDOW_STYLE(WS_POPUP.0 | WS_CAPTION.0 | WS_SYSMENU.0);
const FORM_EX: WINDOW_EX_STYLE = WINDOW_EX_STYLE(WS_EX_CONTROLPARENT.0 | WS_EX_DLGMODALFRAME.0);

/// Logical metrics from DESIGN.md, scaled per monitor.
const MARGIN: i32 = 16;
const ROW: i32 = 36;
const LABEL: i32 = 110;
const CTL_H: i32 = 24;
const COMBO_H: i32 = 200;
const BTN_W: i32 = 112;
const BTN_H: i32 = 28;
const GAP: i32 = 4;
const ERR_H: i32 = 20;
const MIN_W: i32 = 480;
const MIN_H: i32 = 560;
const FORM_W: i32 = 400;
const FORM_BTN_W: i32 = 80;
const NAME_MAX: usize = 40;
const TEXT_MAX: usize = 2048;
const MAX_ITEMS: usize = 8;

/// `SS_ENDELLIPSIS` and `SS_NOPREFIX`, which live behind another crate feature.
const SS_ELLIPSIS: WINDOW_STYLE = WINDOW_STYLE(0x4000);
/// `SS_CENTERIMAGE`: the label sits on the line of its control.
const SS_CENTER: WINDOW_STYLE = WINDOW_STYLE(0x0200);
const SS_NOPREFIX_BIT: WINDOW_STYLE = WINDOW_STYLE(0x0080);

const ID_HOTKEY: u16 = 100;
const ID_MOUSE: u16 = 101;
const ID_THEME: u16 = 102;
const ID_AUTOSTART: u16 = 103;
const ID_NAMES: u16 = 104;
const TAB_ID: u16 = 110;
const TAB_TEXT: [&str; 3] = ["M&y Apps", "A&pps", "S&ites"];
const ITEMS_ID: u16 = 120;
const BUTTONS: u16 = 200;
const BUTTON_TEXT: [&str; 7] = [
    "&Add",
    "Add &from Start",
    "&Edit",
    "&Remove",
    "Move &up",
    "Move &down",
    "Move &to Apps",
];
const ID_BROWSE: u16 = 300;
const ID_START: u16 = 301;
const SUBCLASS: usize = 1;

const ADD: usize = 0;
const START: usize = 1;
const EDIT: usize = 2;
const REMOVE: usize = 3;
const UP: usize = 4;
const DOWN: usize = 5;
const MOVE: usize = 6;

/// Combo order from DESIGN.md, which is also `Mouse` order.
const MOUSE_ITEMS: [Mouse; 4] = [Mouse::None, Mouse::Back, Mouse::Forward, Mouse::CtrlMiddle];

/// Shortcut keys named exactly as `trigger::parse_hotkey` spells them.
const NAMED: [(VIRTUAL_KEY, &str); 9] = [
    (VK_SPACE, "Space"),
    (VK_TAB, "Tab"),
    (VK_RETURN, "Enter"),
    (VK_INSERT, "Insert"),
    (VK_HOME, "Home"),
    (VK_END, "End"),
    (VK_PRIOR, "PageUp"),
    (VK_NEXT, "PageDown"),
    (VK_PAUSE, "Pause"),
];

const MODIFIERS: [VIRTUAL_KEY; 11] = [
    VK_SHIFT,
    VK_CONTROL,
    VK_MENU,
    VK_LWIN,
    VK_RWIN,
    VK_LSHIFT,
    VK_RSHIFT,
    VK_LCONTROL,
    VK_RCONTROL,
    VK_LMENU,
    // AltGr arrives as right Alt but also fakes Ctrl down; it must pass
    // through on an Arabic layout instead of showing an error.
    VK_RMENU,
];

#[derive(Clone, Copy, Default)]
struct Widgets {
    hotkey: HWND,
    mouse: HWND,
    theme: HWND,
    autostart: HWND,
    names: HWND,
    tabs: [HWND; 3],
    items: HWND,
    buttons: [HWND; 7],
    label: [HWND; 4],
    error: [HWND; 6],
}

impl Widgets {
    /// Every control, for one font pass.
    fn all(self) -> Vec<HWND> {
        let mut v = vec![self.hotkey, self.mouse, self.theme, self.autostart];
        v.push(self.names);
        v.extend(self.tabs);
        v.push(self.items);
        v.extend(self.buttons);
        v.extend(self.label);
        v.extend(self.error);
        v.retain(|h| !h.is_invalid());
        v
    }
}

struct State {
    hwnd: HWND,
    host: Host,
    /// The list on screen.
    list: List,
    dpi: u32,
    font: HFONT,
    big: HICON,
    small: HICON,
    /// Loaded from our resource and so ours to destroy.
    owned: bool,
    w: Widgets,
    errors: [Option<String>; 6],
    /// Set while controls are written from code, so nothing counts as a change.
    busy: u32,
    /// Last key captured in the shortcut field; its WM_CHAR is swallowed.
    captured: u32,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    static HOST: Cell<Option<Host>> = const { Cell::new(None) };
    static FORM: RefCell<Option<Form>> = const { RefCell::new(None) };
    static FORM_INIT: RefCell<Option<FormInit>> = const { RefCell::new(None) };
}

/// Runs `f` only when no other call into this module holds the state.
fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|cell| match cell.try_borrow_mut() {
        Ok(mut slot) => slot.as_mut().map(f),
        Err(_) => None,
    })
}

fn with_form<R>(f: impl FnOnce(&mut Form) -> R) -> Option<R> {
    FORM.with(|cell| match cell.try_borrow_mut() {
        Ok(mut slot) => slot.as_mut().map(f),
        Err(_) => None,
    })
}

/// Opens the window, or brings the open one to the front.
pub fn open(host: Host, list: Option<List>) -> windows::core::Result<()> {
    if let Some(hwnd) = with_state(|s| s.hwnd) {
        unsafe {
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
            // An open Add or Edit window comes first.
            let popup = GetLastActivePopup(hwnd);
            let _ = SetForegroundWindow(popup);
            // An open Add or Edit window belongs to the list that shows now.
            if let Some(list) = list.filter(|_| popup == hwnd) {
                show_list(list);
            }
        }
        return Ok(());
    }
    unsafe {
        let module = GetModuleHandleW(None)?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: module.into(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hbrBackground: GetSysColorBrush(COLOR_BTNFACE),
            lpszClassName: CLASS,
            ..Default::default()
        };
        RegisterClassW(&class);

        let mut p = POINT::default();
        let _ = GetCursorPos(&mut p);
        let monitor = MonitorFromPoint(p, MONITOR_DEFAULTTONEAREST);
        let (mut dpi, mut other) = (96u32, 0u32);
        let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut other);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let _ = GetMonitorInfoW(monitor, &mut info);

        let work = info.rcWork;
        let (fw, fh) = window_size(dpi);
        let w = fw.min(work.right - work.left);
        let h = fh.min(work.bottom - work.top);
        let x = work.left + ((work.right - work.left) - w) / 2;
        let y = work.top + ((work.bottom - work.top) - h) / 2;

        HOST.with(|c| c.set(Some(host)));
        let created = CreateWindowExW(
            WS_EX_CONTROLPARENT,
            CLASS,
            TITLE,
            WS_OVERLAPPEDWINDOW,
            x,
            y,
            w,
            h,
            None,
            None,
            Some(module.into()),
            None,
        );
        HOST.with(|c| c.set(None));

        let hwnd = created?;
        if let Some(list) = list {
            show_list(list);
        }
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        if let Some(field) = with_state(|s| s.w.hotkey) {
            let _ = SetFocus(Some(field));
        }
        Ok(())
    }
}

/// IsDialogMessageW for the settings window, after the lists' own keys.
/// True = handled; false when closed.
pub fn translate(msg: &MSG) -> bool {
    let Some((hwnd, w)) = with_state(|s| (s.hwnd, s.w)) else {
        return false;
    };
    let Widgets {
        mouse,
        theme,
        items,
        ..
    } = w;
    if msg.message == WM_KEYDOWN {
        let vk = msg.wParam.0 as u32;
        if vk == VK_ESCAPE.0 as u32 {
            // Esc closes, unless the key belongs to another window of ours.
            // An open drop-down list takes Esc for itself.
            let dropped = [mouse, theme]
                .iter()
                .any(|c| send(*c, CB_GETDROPPEDSTATE, 0, 0) != 0);
            if !dropped && unsafe { GetAncestor(msg.hwnd, GA_ROOT) } == hwnd {
                close();
                return true;
            }
        }
        let action = match vk {
            vk if vk == VK_INSERT.0 as u32 => Some(ADD),
            vk if vk == VK_DELETE.0 as u32 => Some(REMOVE),
            vk if vk == VK_RETURN.0 as u32 => Some(EDIT),
            _ => None,
        };
        if let Some(action) = action.filter(|_| msg.hwnd == items) {
            item(action);
            return true;
        }
    }
    unsafe { IsDialogMessageW(hwnd, msg).as_bool() }
}

/// Destroys the window if open. Everything is freed at WM_NCDESTROY.
pub fn close() {
    if let Some(hwnd) = with_state(|s| s.hwnd) {
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }
}

fn window_size(dpi: u32) -> (i32, i32) {
    let mut rc = RECT {
        left: 0,
        top: 0,
        right: px(MIN_W, dpi),
        bottom: px(MIN_H, dpi),
    };
    unsafe {
        let _ = AdjustWindowRectExForDpi(
            &mut rc,
            WS_OVERLAPPEDWINDOW,
            false,
            WS_EX_CONTROLPARENT,
            dpi,
        );
    }
    (rc.right - rc.left, rc.bottom - rc.top)
}

fn px(logical: i32, dpi: u32) -> i32 {
    (logical * dpi as i32 + 48) / 96
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn set_text(h: HWND, text: &str) {
    let t = wide(text);
    unsafe {
        let _ = SetWindowTextW(h, PCWSTR(t.as_ptr()));
    }
}

fn text_of(h: HWND) -> String {
    let len = unsafe { GetWindowTextLengthW(h) };
    if len <= 0 {
        return String::new();
    }
    let mut buf = vec![0u16; len as usize + 1];
    let got = unsafe { GetWindowTextW(h, &mut buf) };
    String::from_utf16_lossy(&buf[..got.max(0) as usize])
}

fn send(h: HWND, msg: u32, w: usize, l: isize) -> isize {
    unsafe { SendMessageW(h, msg, Some(WPARAM(w)), Some(LPARAM(l))).0 }
}

/// A child control with the given class, text, style and id.
unsafe fn make(parent: HWND, class: PCWSTR, text: &str, style: WINDOW_STYLE, id: u16) -> HWND {
    let t = wide(text);
    let menu = HMENU(id as usize as *mut core::ffi::c_void);
    unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            PCWSTR(t.as_ptr()),
            style,
            0,
            0,
            0,
            0,
            Some(parent),
            Some(menu),
            None,
            None,
        )
        .unwrap_or_default()
    }
}

fn edit_style(read_only: bool) -> WINDOW_STYLE {
    let base = WS_CHILD | WS_VISIBLE | WS_BORDER | WS_TABSTOP | WINDOW_STYLE(ES_AUTOHSCROLL as u32);
    if read_only {
        base | WINDOW_STYLE(ES_READONLY as u32)
    } else {
        base
    }
}

fn button_style(default: bool) -> WINDOW_STYLE {
    let base = WS_CHILD | WS_VISIBLE | WS_TABSTOP;
    if default {
        base | WINDOW_STYLE(BS_DEFPUSHBUTTON as u32)
    } else {
        base | WINDOW_STYLE(BS_PUSHBUTTON as u32)
    }
}

fn combo_style() -> WINDOW_STYLE {
    WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_VSCROLL | WINDOW_STYLE(CBS_DROPDOWNLIST as u32)
}

fn list_style() -> WINDOW_STYLE {
    WS_CHILD | WS_VISIBLE | WS_BORDER | WS_VSCROLL | WS_TABSTOP | WINDOW_STYLE(LBS_NOTIFY as u32)
}

/// The system UI font at `dpi`, owned by the caller; null when unavailable.
fn message_font(dpi: u32) -> HFONT {
    let mut ncm = NONCLIENTMETRICSW {
        cbSize: size_of::<NONCLIENTMETRICSW>() as u32,
        ..Default::default()
    };
    let ok = unsafe {
        SystemParametersInfoForDpi(
            SPI_GETNONCLIENTMETRICS.0,
            ncm.cbSize,
            Some((&raw mut ncm).cast()),
            0,
            dpi,
        )
        .is_ok()
    };
    if ok {
        let font = unsafe { CreateFontIndirectW(&ncm.lfMessageFont) };
        if !font.is_invalid() {
            return font;
        }
    }
    HFONT::default()
}

fn apply_font(font: HFONT) {
    let Some(w) = with_state(|s| s.w) else { return };
    for h in w.all() {
        send(h, WM_SETFONT, font.0 as usize, 1);
    }
}

fn set_dpi(dpi: u32) {
    let font = message_font(dpi);
    if font.is_invalid() {
        with_state(|s| s.dpi = dpi);
        layout();
        return;
    }
    let old = with_state(|s| {
        s.dpi = dpi;
        replace(&mut s.font, font)
    })
    .unwrap_or_default();
    apply_font(font);
    if !old.is_invalid() {
        unsafe {
            let _ = DeleteObject(HGDIOBJ(old.0));
        }
    }
    layout();
}

fn use_dark_title(hwnd: HWND) {
    if high_contrast() {
        return;
    }
    let dark = i32::from(!apps_use_light());
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            (&raw const dark).cast(),
            size_of::<i32>() as u32,
        );
    }
}

fn high_contrast() -> bool {
    let mut hc = HIGHCONTRASTW {
        cbSize: size_of::<HIGHCONTRASTW>() as u32,
        ..Default::default()
    };
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            hc.cbSize,
            Some((&raw mut hc).cast()),
            Default::default(),
        )
        .is_ok()
    };
    ok && hc.dwFlags.contains(HCF_HIGHCONTRASTON)
}

/// HKCU Personalize `AppsUseLightTheme`; light when unreadable, like Windows.
fn apps_use_light() -> bool {
    let mut value: u32 = 1;
    let mut size = size_of::<u32>() as u32;
    let mut kind = REG_VALUE_TYPE::default();
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            Some(&mut kind),
            Some((&raw mut value).cast()),
            Some(&mut size),
        )
    };
    result == ERROR_SUCCESS && value != 0
}

fn load_icons() -> (HICON, HICON, bool) {
    unsafe {
        let module = GetModuleHandleW(None).ok();
        let big = load_icon(module, SM_CXICON, SM_CYICON);
        let small = load_icon(module, SM_CXSMICON, SM_CYSMICON);
        match (big, small) {
            (Some(b), Some(s)) => (b, s, true),
            (b, s) => {
                if let Some(icon) = b {
                    let _ = DestroyIcon(icon);
                }
                if let Some(icon) = s {
                    let _ = DestroyIcon(icon);
                }
                // `IDI_APPLICATION` belongs to the system and is not destroyed.
                let fallback = LoadIconW(None, IDI_APPLICATION).unwrap_or_default();
                (fallback, fallback, false)
            }
        }
    }
}

fn load_icon(
    module: Option<HMODULE>,
    index: SYSTEM_METRICS_INDEX,
    other: SYSTEM_METRICS_INDEX,
) -> Option<HICON> {
    let (cx, cy) = unsafe { (GetSystemMetrics(index), GetSystemMetrics(other)) };
    let handle = unsafe {
        LoadImageW(
            module.map(HINSTANCE::from),
            ICON,
            IMAGE_ICON,
            cx,
            cy,
            LR_DEFAULTCOLOR,
        )
    }
    .ok()?;
    Some(HICON(handle.0))
}

fn setup(hwnd: HWND) -> bool {
    let Some(host) = HOST.with(|c| c.take()) else {
        return false;
    };
    let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
    let font = message_font(dpi);
    let (big, small, owned) = load_icons();

    let w = unsafe { build_controls(hwnd) };
    if w.all().len() != 26 {
        free(w, font, big, small, owned);
        return false;
    }

    STATE.with(|slot| {
        *slot.borrow_mut() = Some(State {
            hwnd,
            host,
            list: List::MyApps,
            dpi,
            font,
            big,
            small,
            owned,
            w,
            errors: Default::default(),
            busy: 0,
            captured: 0,
        })
    });

    unsafe {
        let _ = SendMessageW(
            hwnd,
            WM_SETICON,
            Some(WPARAM(ICON_BIG as usize)),
            Some(LPARAM(big.0 as isize)),
        );
        let _ = SendMessageW(
            hwnd,
            WM_SETICON,
            Some(WPARAM(ICON_SMALL as usize)),
            Some(LPARAM(small.0 as isize)),
        );
        let _ = SetWindowSubclass(w.hotkey, Some(hotkey_proc), SUBCLASS, 0);
    }
    apply_font(font);
    use_dark_title(hwnd);
    fill_combos(w);
    refresh(None);
    true
}

unsafe fn build_controls(hwnd: HWND) -> Widgets {
    let label = |text: &str| unsafe {
        make(
            hwnd,
            w!("STATIC"),
            text,
            WS_CHILD | WS_VISIBLE | SS_CENTER,
            0,
        )
    };
    let error = |id: u16| unsafe {
        make(
            hwnd,
            w!("STATIC"),
            "",
            WS_CHILD | WS_VISIBLE | SS_ELLIPSIS | SS_NOPREFIX_BIT,
            id,
        )
    };
    let button = |text: &str, id: u16, default: bool| unsafe {
        make(hwnd, w!("BUTTON"), text, button_style(default), id)
    };
    let combo = |id: u16| unsafe { make(hwnd, w!("COMBOBOX"), "", combo_style(), id) };

    // Each label is created right before its control: that is what makes the
    // access key work and keeps the tab order the reading order.
    let mut w = Widgets::default();
    w.label[0] = label("Short&cut");
    w.hotkey = unsafe { make(hwnd, w!("EDIT"), "", edit_style(true), ID_HOTKEY) };
    w.error[0] = error(0);
    w.label[1] = label("&Mouse button");
    w.mouse = combo(ID_MOUSE);
    w.error[1] = error(1);
    w.label[2] = label("T&heme");
    w.theme = combo(ID_THEME);
    w.error[2] = error(2);
    let check = WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_AUTOCHECKBOX as u32);
    w.autostart = unsafe {
        make(
            hwnd,
            w!("BUTTON"),
            "Start with &Windows",
            check,
            ID_AUTOSTART,
        )
    };
    w.error[3] = error(3);
    w.names = unsafe { make(hwnd, w!("BUTTON"), "Show &names", check, ID_NAMES) };
    w.error[4] = error(4);
    w.label[3] = label("Lists");
    for (i, text) in TAB_TEXT.iter().enumerate() {
        // One group: Tab reaches the pressed one, the arrows switch.
        let radio = BS_AUTORADIOBUTTON | BS_PUSHLIKE;
        let mut style = WS_CHILD | WS_VISIBLE | WINDOW_STYLE(radio as u32);
        if i == 0 {
            style |= WS_GROUP | WS_TABSTOP;
        }
        w.tabs[i] = unsafe { make(hwnd, w!("BUTTON"), text, style, TAB_ID + i as u16) };
    }
    w.items = unsafe { make(hwnd, w!("LISTBOX"), "", list_style() | WS_GROUP, ITEMS_ID) };
    for (i, text) in BUTTON_TEXT.iter().enumerate() {
        w.buttons[i] = button(text, BUTTONS + i as u16, false);
    }
    w.error[5] = error(5);
    w
}

fn free(w: Widgets, font: HFONT, big: HICON, small: HICON, owned: bool) {
    unsafe {
        if !font.is_invalid() {
            let _ = DeleteObject(HGDIOBJ(font.0));
        }
        if owned {
            let _ = DestroyIcon(big);
            let _ = DestroyIcon(small);
        }
    }
    for h in w.all() {
        unsafe {
            let _ = DestroyWindow(h);
        }
    }
}

fn teardown() {
    let state = STATE.with(|s| s.try_borrow_mut().ok().and_then(|mut s| s.take()));
    if let Some(s) = state {
        unsafe {
            if !s.font.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(s.font.0));
            }
            if s.owned {
                let _ = DestroyIcon(s.big);
                let _ = DestroyIcon(s.small);
            }
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match msg {
        WM_CREATE => {
            if !setup(hwnd) {
                return LRESULT(-1);
            }
        }
        WM_SIZE => layout(),
        WM_GETMINMAXINFO => {
            if let Some(dpi) = with_state(|s| s.dpi) {
                let (width, height) = window_size(dpi);
                unsafe {
                    let info = &mut *(l.0 as *mut MINMAXINFO);
                    info.ptMinTrackSize = POINT {
                        x: width,
                        y: height,
                    };
                }
            }
        }
        WM_DPICHANGED => {
            let dpi = (w.0 & 0xFFFF) as u32;
            let r = unsafe { &*(l.0 as *const RECT) };
            unsafe {
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    r.left,
                    r.top,
                    r.right - r.left,
                    r.bottom - r.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
            set_dpi(dpi.max(96));
        }
        WM_SETTINGCHANGE => use_dark_title(hwnd),
        WM_COMMAND => command(loword(w.0 as u32), hiword(w.0 as u32)),
        WM_CLOSE => unsafe {
            let _ = DestroyWindow(hwnd);
        },
        WM_NCDESTROY => teardown(),
        _ => {}
    }
    unsafe { DefWindowProcW(hwnd, msg, w, l) }
}

fn loword(value: u32) -> u16 {
    value as u16
}

fn hiword(value: u32) -> u16 {
    (value >> 16) as u16
}

fn command(id: u16, code: u16) {
    if with_state(|s| s.busy > 0).unwrap_or(true) {
        return;
    }
    match id {
        // The dialog manager turns Esc into IDCANCEL.
        id if id == IDCANCEL.0 as u16 && code == BN_CLICKED as u16 => close(),
        ID_MOUSE if code == CBN_SELCHANGE as u16 => {
            let Some(h) = with_state(|s| s.w.mouse) else {
                return;
            };
            let chosen = mouse_at(combo_sel(h).unwrap_or(0));
            apply(Field::Mouse, current_sel(), |c| c.mouse = chosen);
        }
        ID_THEME if code == CBN_SELCHANGE as u16 => {
            let Some(h) = with_state(|s| s.w.theme) else {
                return;
            };
            let name = combo_sel(h)
                .and_then(|i| THEMES.get(i))
                .unwrap_or(&THEMES[1])
                .to_string();
            apply(Field::Theme, current_sel(), move |c| c.theme = name);
        }
        ID_AUTOSTART if code == BN_CLICKED as u16 => {
            let Some(h) = with_state(|s| s.w.autostart) else {
                return;
            };
            let on = send(h, BM_GETCHECK, 0, 0) == BST_CHECKED.0 as isize;
            apply(Field::Autostart, current_sel(), |c| c.autostart = on);
        }
        ID_NAMES if code == BN_CLICKED as u16 => {
            let Some(w) = with_state(|s| s.w) else {
                return;
            };
            let on = send(w.names, BM_GETCHECK, 0, 0) == BST_CHECKED.0 as isize;
            apply(Field::Names, list_sel(w.items), |c| c.show_names = on);
        }
        ITEMS_ID if code == LBN_SELCHANGE as u16 => update_buttons(),
        ITEMS_ID if code == LBN_DBLCLK as u16 => item(EDIT),
        id if code == BN_CLICKED as u16 => {
            if let Some(&list) = LISTS.get(id.wrapping_sub(TAB_ID) as usize) {
                show_list(list);
            } else if (BUTTONS..BUTTONS + 7).contains(&id) {
                item((id - BUTTONS) as usize);
            }
        }
        _ => {}
    }
}

fn fill_combos(w: Widgets) {
    for mouse in MOUSE_ITEMS {
        let text = wide(mouse_label(mouse));
        send(w.mouse, CB_ADDSTRING, 0, text.as_ptr() as isize);
    }
    for theme in THEMES {
        let text = wide(theme);
        send(w.theme, CB_ADDSTRING, 0, text.as_ptr() as isize);
    }
}

fn combo_sel(h: HWND) -> Option<usize> {
    let i = send(h, CB_GETCURSEL, 0, 0);
    (i >= 0).then_some(i as usize)
}

fn list_sel(h: HWND) -> Option<usize> {
    let i = send(h, LB_GETCURSEL, 0, 0);
    (i >= 0).then_some(i as usize)
}

fn current_sel() -> Option<usize> {
    list_sel(with_state(|s| s.w)?.items)
}

struct Geometry {
    margin: i32,
    label: i32,
    row: i32,
    ctl: i32,
    err: i32,
    btn_w: i32,
    btn_h: i32,
    gap: i32,
}

impl Geometry {
    fn new(dpi: u32) -> Self {
        Self {
            margin: px(MARGIN, dpi),
            label: px(LABEL, dpi),
            row: px(ROW, dpi),
            ctl: px(CTL_H, dpi),
            err: px(ERR_H, dpi),
            btn_w: px(BTN_W, dpi),
            btn_h: px(BTN_H, dpi),
            gap: px(GAP, dpi),
        }
    }
}

fn layout() {
    let Some(hwnd) = with_state(|s| s.hwnd) else {
        return;
    };
    let mut rc = RECT::default();
    if unsafe { GetClientRect(hwnd, &mut rc) }.is_err() {
        return;
    }
    let plan = with_state(|s| arrange(s, rc.right, rc.bottom)).unwrap_or_default();
    for (h, x, y, w, y2) in plan {
        unsafe {
            let _ = MoveWindow(h, x, y, w, y2, true);
        }
    }
    repaint(hwnd);
}

/// Moved controls leave pieces of themselves behind otherwise.
fn repaint(hwnd: HWND) {
    let flags = RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN;
    let _ = unsafe { RedrawWindow(Some(hwnd), None, None, flags) };
}

/// Control rectangles for the client size, computed with no API calls.
fn arrange(s: &State, width: i32, height: i32) -> Vec<(HWND, i32, i32, i32, i32)> {
    let g = Geometry::new(s.dpi);
    let xl = g.margin;
    let xc = g.margin + g.label;
    let cw = (width - xc - g.margin).max(px(60, s.dpi));
    let mut out = Vec::with_capacity(32);
    let mut y = g.margin;

    for i in 0..3 {
        let (label, control, combo) = match i {
            0 => (s.w.label[0], s.w.hotkey, false),
            1 => (s.w.label[1], s.w.mouse, true),
            _ => (s.w.label[2], s.w.theme, true),
        };
        let ch = if combo { px(COMBO_H, s.dpi) } else { g.ctl };
        out.push((label, xl, y, g.label, g.ctl));
        out.push((control, xc, y, cw, ch));
        let shown = s.errors[i].is_some();
        out.push((
            s.w.error[i],
            xc,
            y + g.ctl + 2,
            cw,
            if shown { g.err } else { 0 },
        ));
        y += g.row + if shown { g.err } else { 0 };
    }

    for (control, i) in [(s.w.autostart, 3), (s.w.names, 4)] {
        out.push((control, xc, y, cw, g.ctl));
        let shown = s.errors[i].is_some();
        out.push((
            s.w.error[i],
            xc,
            y + g.ctl + 2,
            cw,
            if shown { g.err } else { 0 },
        ));
        y += g.row + if shown { g.err } else { 0 };
    }

    out.push((s.w.label[3], xl, y, g.label, g.btn_h));
    let tab_w = (cw - 2 * g.gap) / 3;
    for (i, tab) in s.w.tabs.into_iter().enumerate() {
        out.push((tab, xc + i as i32 * (tab_w + g.gap), y, tab_w, g.btn_h));
    }
    y += g.row;

    let shown = s.errors[5].is_some();
    let err_h = if shown { g.err + g.gap } else { 0 };
    let bx = width - g.margin - g.btn_w;
    let lw = (bx - g.gap - xc).max(px(40, s.dpi));
    let list_h = (height - g.margin - y - err_h).max(px(40, s.dpi));
    out.push((s.w.items, xc, y, lw, list_h));
    for (i, button) in s.w.buttons.into_iter().enumerate() {
        let by = y + i as i32 * (g.btn_h + g.gap);
        out.push((button, bx, by, g.btn_w, g.btn_h));
    }
    out.push((
        s.w.error[5],
        xc,
        y + list_h + g.gap,
        lw,
        if shown { g.err } else { 0 },
    ));
    out
}

fn update_buttons() {
    let Some((w, host, list)) = with_state(|s| (s.w, s.host, s.list)) else {
        return;
    };
    let c = (host.get)();
    let on = enabled(list, &c, list_sel(w.items));
    for (button, enabled) in w.buttons.into_iter().zip(on) {
        unsafe {
            let _ = EnableWindow(button, enabled);
            // Focus must never sit on a control that cannot act.
            if !enabled && GetFocus() == button {
                let _ = SetFocus(Some(w.items));
            }
        }
    }
}

/// Which buttons can act, in button order.
fn enabled(list: List, c: &Config, sel: Option<usize>) -> [bool; 7] {
    let len = list.len(c);
    let room = len < MAX_ITEMS;
    [
        room,
        room && list != List::Sites,
        sel.is_some(),
        sel.is_some(),
        up_index(sel).is_some(),
        down_index(sel, len).is_some(),
        sel.is_some() && list.other().is_some_and(|o| o.len(c) < MAX_ITEMS),
    ]
}

fn errors_text() {
    let Some(w) = with_state(|s| s.w) else { return };
    let errors = with_state(|s| s.errors.clone()).unwrap_or_default();
    for (h, text) in w.error.iter().zip(errors.iter()) {
        let line = text
            .as_ref()
            .map_or_else(String::new, |t| format!("Error: {t}"));
        set_text(*h, &line);
    }
}

/// Rewrites every control from `(host.get)()`, then lays out and refocuses.
fn refresh(sel: Option<usize>) {
    let Some((w, host, list)) = with_state(|s| (s.w, s.host, s.list)) else {
        return;
    };
    let c = (host.get)();
    with_state(|s| s.busy += 1);

    set_text(w.hotkey, &c.hotkey);
    send(w.mouse, CB_SETCURSEL, mouse_index(c.mouse), 0);
    send(w.theme, CB_SETCURSEL, theme_index(&c.theme), 0);
    let check = |h: HWND, on: bool| {
        let state = if on { BST_CHECKED } else { BST_UNCHECKED };
        send(h, BM_SETCHECK, state.0 as usize, 0);
    };
    check(w.autostart, c.autostart);
    check(w.names, c.show_names);
    for (tab, l) in w.tabs.into_iter().zip(LISTS) {
        check(tab, l == list);
    }
    set_text(w.buttons[MOVE], list.move_text());
    let labels = list.labels(&c);
    fill(w.items, labels.iter());
    select(w.items, sel, labels.len());

    with_state(|s| s.busy = s.busy.saturating_sub(1));
    errors_text();
    update_buttons();
    layout();
}

fn fill(list: HWND, items: impl Iterator<Item = impl AsRef<str>>) {
    send(list, LB_RESETCONTENT, 0, 0);
    for item in items {
        let text = wide(item.as_ref());
        send(list, LB_ADDSTRING, 0, text.as_ptr() as isize);
    }
}

fn select(list: HWND, sel: Option<usize>, len: usize) {
    let index = match sel {
        Some(i) if i < len => i as isize,
        _ => -1,
    };
    send(list, LB_SETCURSEL, index as usize, 0);
}

/// Applies one change to the config from the host, then shows what happened.
fn apply(field: Field, sel: Option<usize>, change: impl FnOnce(&mut Config)) {
    let Some(host) = with_state(|s| s.host) else {
        return;
    };
    let mut c = (host.get)();
    change(&mut c);
    match (host.apply)(c) {
        Ok(()) => with_state(|s| s.errors[field.slot()] = None),
        Err(p) => with_state(|s| s.errors[p.field.slot()] = Some(p.text)),
    }
    .unwrap_or_default();
    refresh(sel);
}

fn show_list(list: List) {
    if with_state(|s| s.list) == Some(list) {
        return;
    }
    with_state(|s| {
        s.list = list;
        // It belonged to the other list.
        s.errors[list.field().slot()] = None;
    });
    refresh(None);
}

fn item(action: usize) {
    let Some((owner, host, list, w)) = with_state(|s| (s.hwnd, s.host, s.list, s.w)) else {
        return;
    };
    let c = (host.get)();
    let sel = list_sel(w.items);
    // The list's own keys come here too.
    if !enabled(list, &c, sel).get(action).is_some_and(|&on| on) {
        return;
    }
    match action {
        ADD => {
            if let Some(made) = dialog(owner, list, None, false) {
                let end = list.len(&(host.get)());
                apply(list.field(), Some(end), |c| list.put(c, end, made));
            }
        }
        START => add_from_start(owner, host, list),
        EDIT => {
            let Some(index) = sel else { return };
            let texts = list.texts(&c, index);
            if let Some(made) = dialog(owner, list, texts, true) {
                apply(list.field(), sel, |c| list.put(c, index, made));
            }
        }
        REMOVE => {
            let Some(index) = sel else { return };
            let Some(label) = list.labels(&c).into_iter().nth(index) else {
                return;
            };
            let text = HSTRING::from(format!("Remove \"{label}\"?"));
            let style = MB_YESNO | MB_DEFBUTTON2;
            if unsafe { MessageBoxW(Some(owner), &text, w!("Madar"), style) } == IDYES {
                let after = after_remove(index, list.len(&c));
                apply(list.field(), after, |c| {
                    list.take(c, index);
                });
            }
        }
        UP | DOWN => {
            let to = if action == UP {
                up_index(sel)
            } else {
                down_index(sel, list.len(&c))
            };
            if let (Some(from), Some(to)) = (sel, to) {
                apply(list.field(), Some(to), |c| list.swap(c, from, to));
            }
        }
        MOVE => {
            if let (Some(index), Some(other)) = (sel, list.other()) {
                let after = after_remove(index, list.len(&c));
                apply(list.field(), after, |c| {
                    if let Some(made) = list.take(c, index) {
                        let end = other.len(c);
                        other.put(c, end, made);
                    }
                });
            }
        }
        _ => {}
    }
    // These showed another window; focus returns to the list.
    if matches!(action, ADD | EDIT | REMOVE) {
        let _ = unsafe { SetFocus(Some(w.items)) };
    }
}

fn add_from_start(owner: HWND, host: Host, list: List) {
    let Some(w) = with_state(|s| s.w) else { return };
    if let Some(entry) = start_menu::pick(owner) {
        let end = list.len(&(host.get)());
        let app = App {
            label: entry.name.chars().take(NAME_MAX).collect(),
            path: entry.path,
            args: String::new(),
        };
        apply(list.field(), Some(end), |c| {
            list.put(c, end, Made::App(app))
        });
    }
    // Back on the button, so several programs can be added in a row.
    let full = list.len(&(host.get)()) >= MAX_ITEMS;
    let target = if full { w.items } else { w.buttons[START] };
    let _ = unsafe { SetFocus(Some(target)) };
}

/// The list the window shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum List {
    MyApps,
    Apps,
    Sites,
}

pub const LISTS: [List; 3] = [List::MyApps, List::Apps, List::Sites];

impl List {
    fn field(self) -> Field {
        match self {
            Self::MyApps => Field::MyApps,
            Self::Apps => Field::Apps,
            Self::Sites => Field::Sites,
        }
    }

    fn labels(self, c: &Config) -> Vec<String> {
        let names = |apps: &[App]| apps.iter().map(|a| a.label.clone()).collect();
        match self {
            Self::MyApps => names(&c.my_apps),
            Self::Apps => names(&c.apps),
            Self::Sites => c.sites.iter().map(|s| s.label.clone()).collect(),
        }
    }

    fn len(self, c: &Config) -> usize {
        match self {
            Self::MyApps => c.my_apps.len(),
            Self::Apps => c.apps.len(),
            Self::Sites => c.sites.len(),
        }
    }

    /// The fields of one item, for the Edit window.
    fn texts(self, c: &Config, index: usize) -> Option<[String; 3]> {
        let of = |a: &App| [a.label.clone(), a.path.clone(), a.args.clone()];
        match self {
            Self::MyApps => c.my_apps.get(index).map(of),
            Self::Apps => c.apps.get(index).map(of),
            Self::Sites => c.sites.get(index).map(|s| {
                [
                    s.label.clone(),
                    s.url.clone(),
                    s.browser.clone().unwrap_or_default(),
                ]
            }),
        }
    }

    /// Replaces the item at `index`, or appends after the last one.
    fn put(self, c: &mut Config, index: usize, made: Made) {
        fn set<T>(items: &mut Vec<T>, index: usize, item: T) {
            match items.get_mut(index) {
                Some(slot) => *slot = item,
                None => items.push(item),
            }
        }
        match (self, made) {
            (Self::MyApps, Made::App(a)) => set(&mut c.my_apps, index, a),
            (Self::Apps, Made::App(a)) => set(&mut c.apps, index, a),
            (Self::Sites, Made::Site(s)) => set(&mut c.sites, index, s),
            // The Add window makes the kind its list holds.
            _ => {}
        }
    }

    fn take(self, c: &mut Config, index: usize) -> Option<Made> {
        (index < self.len(c)).then(|| match self {
            Self::MyApps => Made::App(c.my_apps.remove(index)),
            Self::Apps => Made::App(c.apps.remove(index)),
            Self::Sites => Made::Site(c.sites.remove(index)),
        })
    }

    fn swap(self, c: &mut Config, a: usize, b: usize) {
        if a.max(b) < self.len(c) {
            match self {
                Self::MyApps => c.my_apps.swap(a, b),
                Self::Apps => c.apps.swap(a, b),
                Self::Sites => c.sites.swap(a, b),
            }
        }
    }

    /// Where Move to sends an item.
    fn other(self) -> Option<Self> {
        match self {
            Self::MyApps => Some(Self::Apps),
            Self::Apps => Some(Self::MyApps),
            Self::Sites => None,
        }
    }

    fn move_text(self) -> &'static str {
        match self {
            Self::Apps => "Move &to My Apps",
            _ => "Move &to Apps",
        }
    }

    fn title(self, editing: bool) -> &'static str {
        match (self, editing) {
            (Self::Sites, false) => "Add site",
            (Self::Sites, true) => "Edit site",
            (_, false) => "Add app",
            (_, true) => "Edit app",
        }
    }

    /// The field that holds a program path.
    fn browse_row(self) -> usize {
        if self == Self::Sites { 2 } else { 1 }
    }

    fn names(self) -> [&'static str; 3] {
        match self {
            Self::Sites => ["&Name", "&Address", "Bro&wser"],
            _ => ["&Name", "&Program", "A&rguments"],
        }
    }
}

fn mouse_label(mouse: Mouse) -> &'static str {
    match mouse {
        Mouse::None => "None",
        Mouse::Back => "Back",
        Mouse::Forward => "Forward",
        Mouse::CtrlMiddle => "Ctrl + Middle",
    }
}

fn mouse_index(mouse: Mouse) -> usize {
    MOUSE_ITEMS.iter().position(|&m| m == mouse).unwrap_or(0)
}

fn mouse_at(index: usize) -> Mouse {
    MOUSE_ITEMS.get(index).copied().unwrap_or(Mouse::None)
}

fn theme_index(name: &str) -> usize {
    THEMES.iter().position(|&t| t == name).unwrap_or(1)
}

/// Selection after moving up; None when the row cannot move.
fn up_index(sel: Option<usize>) -> Option<usize> {
    sel.filter(|&i| i > 0).map(|i| i - 1)
}

/// Selection after moving down; None when the row cannot move.
fn down_index(sel: Option<usize>, len: usize) -> Option<usize> {
    sel.filter(|&i| i + 1 < len).map(|i| i + 1)
}

/// Selection after removing `index` from a list `len` long: the next one, else the last.
fn after_remove(index: usize, len: usize) -> Option<usize> {
    if len <= 1 {
        return None;
    }
    Some(if index + 1 < len { index } else { index - 1 })
}

// --- shortcut capture ---

#[derive(Clone, Copy, Default)]
struct Mods {
    ctrl: bool,
    alt: bool,
    shift: bool,
    win: bool,
}

fn modifiers() -> Mods {
    let down = |key: VIRTUAL_KEY| unsafe { GetKeyState(key.0 as i32) < 0 };
    Mods {
        ctrl: down(VK_CONTROL),
        alt: down(VK_MENU),
        shift: down(VK_SHIFT),
        win: down(VK_LWIN) || down(VK_RWIN),
    }
}

/// The shortcut text for a key, or None when Windows owns it or it cannot work.
fn hotkey_text(mods: Mods, vk: u32) -> Option<String> {
    let name = key_name(vk)?;
    let function = (VK_F13.0 as u32..=VK_F24.0 as u32).contains(&vk);
    if !(mods.ctrl || mods.win || function) {
        return None;
    }
    if mods.ctrl && !mods.alt && !mods.shift && !mods.win && ctrl_reserved(vk) {
        return None;
    }
    let mut text = String::new();
    for (held, part) in [
        (mods.ctrl, "Ctrl"),
        (mods.alt, "Alt"),
        (mods.shift, "Shift"),
        (mods.win, "Win"),
    ] {
        if held {
            text.push_str(part);
            text.push('+');
        }
    }
    text.push_str(&name);
    Some(text)
}

fn key_name(vk: u32) -> Option<String> {
    if let Some((_, name)) = NAMED.iter().find(|(key, _)| key.0 as u32 == vk) {
        return Some((*name).to_string());
    }
    if (0x30..=0x39).contains(&vk) || (0x41..=0x5A).contains(&vk) {
        return Some(char::from(vk as u8).to_string());
    }
    if (VK_F1.0 as u32..=VK_F24.0 as u32).contains(&vk) {
        return Some(format!("F{}", vk - VK_F1.0 as u32 + 1));
    }
    None
}

/// Ctrl+ these belongs to Windows itself; taking one would break typing.
fn ctrl_reserved(vk: u32) -> bool {
    (0x41..=0x5A).contains(&vk)
        && matches!(vk as u8 as char, 'A' | 'C' | 'V' | 'X' | 'Y' | 'Z' | 'S')
}

fn refusal(mods: Mods, vk: u32) -> String {
    if mods.ctrl && !mods.alt && !mods.shift && !mods.win && ctrl_reserved(vk) {
        format!("Windows uses Ctrl+{}.", vk as u8 as char)
    } else {
        "This key cannot be used.".to_string()
    }
}

/// A key press in the shortcut field. True when the field took it.
fn capture(vk: u32) -> bool {
    if MODIFIERS.iter().any(|m| m.0 as u32 == vk) {
        return false;
    }
    let mods = modifiers();
    match hotkey_text(mods, vk) {
        Some(text) => {
            if let Some(field) = with_state(|s| s.w.hotkey) {
                set_text(field, &text);
            }
            with_state(|s| s.captured = vk);
            apply(Field::Hotkey, current_sel(), move |c| c.hotkey = text);
            true
        }
        // Alone, or with only Shift/Alt: leave Tab, Esc, Enter, Alt+F4 and
        // Alt+access keys to the dialog manager.
        None if !(mods.ctrl || mods.win) => false,
        None => {
            let text = refusal(mods, vk);
            with_state(|s| s.errors[Field::Hotkey.slot()] = Some(text));
            errors_text();
            layout();
            true
        }
    }
}

unsafe extern "system" fn hotkey_proc(
    hwnd: HWND,
    msg: u32,
    w: WPARAM,
    l: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    match msg {
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            let vk = w.0 as u32;
            let repeat = (l.0 as usize) & (1 << 30) != 0;
            if repeat {
                let ours = with_state(|s| s.captured == vk).unwrap_or(false);
                if !ours {
                    return unsafe { DefSubclassProc(hwnd, msg, w, l) };
                }
                return LRESULT(1);
            }
            if capture(vk) {
                return LRESULT(1);
            }
        }
        // The character a captured key would otherwise put into the field.
        // It stays marked until the key is released, so auto-repeat and an
        // Alt form (SYSCHAR) are swallowed too, and no later character is.
        WM_CHAR | WM_SYSCHAR => {
            if with_state(|s| s.captured != 0).unwrap_or(false) {
                return LRESULT(1);
            }
        }
        WM_KEYUP | WM_SYSKEYUP => {
            with_state(|s| {
                if s.captured == w.0 as u32 {
                    s.captured = 0;
                }
            });
        }
        WM_KILLFOCUS => {
            with_state(|s| s.captured = 0);
        }
        WM_NCDESTROY => unsafe {
            let _ = RemoveWindowSubclass(hwnd, Some(hotkey_proc), SUBCLASS);
        },
        _ => {}
    }
    unsafe { DefSubclassProc(hwnd, msg, w, l) }
}

// --- the Add/Edit dialog ---

struct FormInit {
    list: List,
    texts: [String; 3],
    dpi: u32,
}

struct Form {
    hwnd: HWND,
    list: List,
    dpi: u32,
    font: HFONT,
    label: [HWND; 3],
    field: [HWND; 3],
    error: [HWND; 3],
    browse: HWND,
    /// Only for apps.
    start: HWND,
    hint: HWND,
    ok: HWND,
    cancel: HWND,
    errors: [Option<String>; 3],
    done: bool,
    outcome: Option<Made>,
}

enum Made {
    App(App),
    Site(Site),
}

fn dialog(owner: HWND, list: List, texts: Option<[String; 3]>, editing: bool) -> Option<Made> {
    unsafe {
        let module = GetModuleHandleW(None).ok()?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(form_proc),
            hInstance: module.into(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hbrBackground: GetSysColorBrush(COLOR_BTNFACE),
            lpszClassName: FORM_CLASS,
            ..Default::default()
        };
        RegisterClassW(&class);

        let dpi = GetDpiForWindow(owner).max(96);
        let mut rc = RECT {
            left: 0,
            top: 0,
            right: px(FORM_W, dpi),
            bottom: form_height(dpi, list == List::Sites, &Default::default()),
        };
        let _ = AdjustWindowRectExForDpi(&mut rc, FORM_STYLE, false, FORM_EX, dpi);
        let (w, h) = (rc.right - rc.left, rc.bottom - rc.top);
        let mut own = RECT::default();
        let _ = GetWindowRect(owner, &mut own);
        let x = own.left + ((own.right - own.left) - w) / 2;
        let y = own.top + ((own.bottom - own.top) - h) / 2;

        let title = wide(list.title(editing));
        FORM_INIT.with(|c| {
            *c.borrow_mut() = Some(FormInit {
                list,
                texts: texts.unwrap_or_default(),
                dpi,
            })
        });
        let created = CreateWindowExW(
            FORM_EX,
            FORM_CLASS,
            PCWSTR(title.as_ptr()),
            FORM_STYLE,
            x,
            y,
            w,
            h,
            Some(owner),
            None,
            Some(module.into()),
            None,
        );
        let Ok(hwnd) = created else {
            FORM_INIT.with(|c| *c.borrow_mut() = None);
            return None;
        };

        use_dark_title(hwnd);
        let _ = EnableWindow(owner, false);
        let _ = ShowWindow(hwnd, SW_SHOW);
        if let Some(first) = with_form(|f| f.field[0]) {
            let _ = SetFocus(Some(first));
        }
        let mut msg = MSG::default();
        while IsWindow(Some(hwnd)).as_bool() {
            // The owner can only die from a message this loop pumps; notice it
            // here so the loop never blocks on a window that no longer exists.
            if !IsWindow(Some(owner)).as_bool() {
                break;
            }
            if with_form(|f| f.done || f.outcome.is_some()).unwrap_or(true) {
                break;
            }
            let got = GetMessageW(&mut msg, None, 0, 0);
            if got.0 <= 0 {
                // The app is quitting: let the outer loop see WM_QUIT again.
                if msg.message == WM_QUIT {
                    PostQuitMessage(msg.wParam.0 as i32);
                }
                break;
            }
            // Enter and Esc are ours, so they work even though this is a plain
            // window class and not a dialog resource.
            if msg.message == WM_KEYDOWN {
                let vk = msg.wParam.0 as u32;
                if vk == VK_RETURN.0 as u32 {
                    // Enter on a focused button presses that button.
                    match with_form(|f| (f.browse, f.start, f.cancel)) {
                        Some((browse, ..)) if msg.hwnd == browse => form_browse(),
                        Some((_, start, _)) if msg.hwnd == start => form_start(),
                        Some((.., cancel)) if msg.hwnd == cancel => {
                            with_form(|f| f.done = true);
                        }
                        _ => form_ok(),
                    }
                    continue;
                }
                if vk == VK_ESCAPE.0 as u32 {
                    with_form(|f| f.done = true);
                    continue;
                }
            }
            if !IsDialogMessageW(hwnd, &msg).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        let made = with_form(|f| f.outcome.take()).flatten();
        // The owner is usable again before this window goes away.
        if IsWindow(Some(owner)).as_bool() {
            let _ = EnableWindow(owner, true);
            let _ = SetFocus(Some(owner));
        }
        if IsWindow(Some(hwnd)).as_bool() {
            let _ = DestroyWindow(hwnd);
        }
        made
    }
}

fn form_height(dpi: u32, site: bool, errors: &[Option<String>; 3]) -> i32 {
    let shown = errors.iter().filter(|e| e.is_some()).count() as i32;
    let logical = MARGIN * 2 + ROW * 3 + BTN_H + ERR_H * shown + if site { ERR_H } else { 0 };
    px(logical, dpi)
}

fn form_size(dpi: u32, site: bool, errors: &[Option<String>; 3]) -> (i32, i32) {
    let mut rc = RECT {
        left: 0,
        top: 0,
        right: px(FORM_W, dpi),
        bottom: form_height(dpi, site, errors),
    };
    unsafe {
        let _ = AdjustWindowRectExForDpi(&mut rc, FORM_STYLE, false, FORM_EX, dpi);
    }
    (rc.right - rc.left, rc.bottom - rc.top)
}

fn form_setup(hwnd: HWND) -> bool {
    let Some(init) = FORM_INIT.with(|c| c.borrow_mut().take()) else {
        return false;
    };
    let site = init.list == List::Sites;
    let font = message_font(init.dpi);
    let names = init.list.names();
    let text_ctl = |text: &str| unsafe {
        make(
            hwnd,
            w!("STATIC"),
            text,
            WS_CHILD | WS_VISIBLE | SS_CENTER,
            0,
        )
    };
    let err_ctl = |id: u16| unsafe {
        make(
            hwnd,
            w!("STATIC"),
            "",
            WS_CHILD | WS_VISIBLE | SS_ELLIPSIS | SS_NOPREFIX_BIT,
            id,
        )
    };
    let edit_ctl = || unsafe { make(hwnd, w!("EDIT"), "", edit_style(false), 0) };

    let mut label = [HWND::default(); 3];
    let mut field = [HWND::default(); 3];
    let mut error = [HWND::default(); 3];
    // Created after its field, so Tab reaches it right there.
    let mut browse = HWND::default();
    for i in 0..3 {
        label[i] = text_ctl(names[i]);
        field[i] = edit_ctl();
        if i == init.list.browse_row() {
            browse = unsafe {
                make(
                    hwnd,
                    w!("BUTTON"),
                    "&Browse...",
                    button_style(false),
                    ID_BROWSE,
                )
            };
        }
        error[i] = err_ctl(0);
    }
    let hint = if site {
        text_ctl("Leave empty for the default browser.")
    } else {
        HWND::default()
    };
    let start = if site {
        HWND::default()
    } else {
        unsafe {
            make(
                hwnd,
                w!("BUTTON"),
                "&Start menu...",
                button_style(false),
                ID_START,
            )
        }
    };
    let ok = unsafe { make(hwnd, w!("BUTTON"), "OK", button_style(true), IDOK.0 as u16) };
    let cancel = unsafe {
        make(
            hwnd,
            w!("BUTTON"),
            "Cancel",
            button_style(false),
            IDCANCEL.0 as u16,
        )
    };

    let form = Form {
        hwnd,
        list: init.list,
        dpi: init.dpi,
        font,
        label,
        field,
        error,
        browse,
        start,
        hint,
        ok,
        cancel,
        errors: Default::default(),
        done: false,
        outcome: None,
    };
    let mut all = Vec::new();
    all.extend(label);
    all.extend(field);
    all.extend(error);
    all.push(browse);
    // Only a site has the hint.
    all.extend(site.then_some(hint));
    all.extend((!site).then_some(start));
    all.push(ok);
    all.push(cancel);
    if all.iter().any(|h| h.is_invalid()) {
        unsafe {
            if !font.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(font.0));
            }
        }
        for h in all {
            unsafe {
                let _ = DestroyWindow(h);
            }
        }
        return false;
    }

    FORM.with(|slot| *slot.borrow_mut() = Some(form));
    for h in all {
        send(h, WM_SETFONT, font.0 as usize, 1);
    }
    for (i, (field, text)) in field.iter().zip(init.texts.iter()).enumerate() {
        let limit = if i == 0 { NAME_MAX } else { TEXT_MAX };
        send(*field, EM_SETLIMITTEXT, limit, 0);
        set_text(*field, text);
    }
    true
}

fn form_arrange(f: &Form, width: i32, height: i32) -> Vec<(HWND, i32, i32, i32, i32)> {
    let g = Geometry::new(f.dpi);
    let xc = g.margin + g.label;
    let cw = (width - xc - g.margin).max(px(60, f.dpi));
    let browse_row = f.list.browse_row();
    let mut out = Vec::with_capacity(16);
    let mut y = g.margin;
    for i in 0..3 {
        out.push((f.label[i], g.margin, y, g.label, g.ctl));
        let w = if i == browse_row {
            cw - g.btn_w - g.gap
        } else {
            cw
        };
        out.push((f.field[i], xc, y, w, g.ctl));
        if i == browse_row {
            out.push((f.browse, xc + cw - g.btn_w, y - 2, g.btn_w, g.btn_h));
        }
        let shown = f.errors[i].is_some();
        out.push((
            f.error[i],
            xc,
            y + g.ctl + 2,
            cw,
            if shown { g.err } else { 0 },
        ));
        y += g.row + if shown { g.err } else { 0 };
    }
    if !f.hint.is_invalid() {
        out.push((f.hint, xc, y - px(8, f.dpi), cw, g.err));
        y += g.err;
    }
    let by = (height - g.margin - g.btn_h).max(y);
    let cancel_x = width - g.margin - px(FORM_BTN_W, f.dpi);
    let ok_x = cancel_x - g.gap - px(FORM_BTN_W, f.dpi);
    if !f.start.is_invalid() {
        out.push((f.start, g.margin, by, px(110, f.dpi), g.btn_h));
    }
    out.push((f.ok, ok_x, by, px(FORM_BTN_W, f.dpi), g.btn_h));
    out.push((f.cancel, cancel_x, by, px(FORM_BTN_W, f.dpi), g.btn_h));
    out
}

fn form_layout() {
    let Some(hwnd) = with_form(|f| f.hwnd) else {
        return;
    };
    let mut rc = RECT::default();
    if unsafe { GetClientRect(hwnd, &mut rc) }.is_err() {
        return;
    }
    let plan = with_form(|f| form_arrange(f, rc.right, rc.bottom)).unwrap_or_default();
    for (h, x, y, w, h2) in plan {
        unsafe {
            let _ = MoveWindow(h, x, y, w, h2, true);
        }
    }
    repaint(hwnd);
}

fn form_errors() {
    let Some(errors) = with_form(|f| f.errors.clone()) else {
        return;
    };
    let Some(fields) = with_form(|f| f.error) else {
        return;
    };
    for (h, text) in fields.iter().zip(errors.iter()) {
        let line = text
            .as_ref()
            .map_or_else(String::new, |t| format!("Error: {t}"));
        set_text(*h, &line);
    }
}

fn form_resize() {
    let Some((hwnd, dpi, site, errors)) =
        with_form(|f| (f.hwnd, f.dpi, f.list == List::Sites, f.errors.clone()))
    else {
        return;
    };
    let (w, h) = form_size(dpi, site, &errors);
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            w,
            h,
            SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

fn form_ok() {
    let Some((list, fields)) = with_form(|f| (f.list, f.field)) else {
        return;
    };
    let texts = [text_of(fields[0]), text_of(fields[1]), text_of(fields[2])];
    // A second OK must not leave the previous round's errors on screen.
    with_form(|f| f.errors = Default::default());
    let checked = match list {
        List::MyApps | List::Apps => check_app(&texts[0], &texts[1], &texts[2]).map(Made::App),
        List::Sites => check_site(&texts[0], &texts[1], &texts[2]).map(Made::Site),
    };
    match checked {
        Ok(made) => {
            with_form(|f| f.outcome = Some(made));
        }
        Err((index, text)) => {
            with_form(|f| f.errors[index] = Some(text));
            form_errors();
            form_resize();
            form_layout();
            if let Some(field) = with_form(|f| f.field.get(index).copied()).flatten() {
                unsafe {
                    let _ = SetFocus(Some(field));
                }
            }
        }
    }
}

fn form_browse() {
    let Some((hwnd, list, fields)) = with_form(|f| (f.hwnd, f.list, f.field)) else {
        return;
    };
    let Some(path) = pick_file(hwnd, list) else {
        return;
    };
    let target = list.browse_row();
    set_text(fields[target], &path);
    if text_of(fields[0]).trim().is_empty()
        && let Some(stem) = Path::new(&path).file_stem()
    {
        set_text(fields[0], &stem.to_string_lossy());
    }
}

fn form_start() {
    let Some((hwnd, fields)) = with_form(|f| (f.hwnd, f.field)) else {
        return;
    };
    let Some(entry) = start_menu::pick(hwnd) else {
        return;
    };
    set_text(fields[1], &entry.path);
    if text_of(fields[0]).trim().is_empty() {
        let name: String = entry.name.chars().take(NAME_MAX).collect();
        set_text(fields[0], &name);
    }
}

fn pick_file(owner: HWND, list: List) -> Option<String> {
    let filter: Vec<u16> = match list {
        List::MyApps | List::Apps => "Programs\0*.exe;*.lnk\0\0",
        List::Sites => "Programs\0*.exe\0\0",
    }
    .encode_utf16()
    .collect();
    let title = wide(if list == List::Sites {
        "Choose a browser"
    } else {
        "Choose a program"
    });
    let mut buf = vec![0u16; 1024];
    let mut ofn = OPENFILENAMEW {
        lStructSize: size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: owner,
        lpstrFilter: PCWSTR(filter.as_ptr()),
        lpstrFile: PWSTR(buf.as_mut_ptr()),
        nMaxFile: buf.len() as u32,
        lpstrTitle: PCWSTR(title.as_ptr()),
        Flags: OFN_FILEMUSTEXIST
            | OFN_PATHMUSTEXIST
            | OFN_NOCHANGEDIR
            | OFN_NODEREFERENCELINKS
            | OFN_EXPLORER
            | OFN_HIDEREADONLY,
        ..Default::default()
    };
    if !unsafe { GetOpenFileNameW(&mut ofn) }.as_bool() {
        return None;
    }
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    (end > 0).then(|| String::from_utf16_lossy(&buf[..end]))
}

fn form_command(id: u16, code: u16) {
    if id == IDOK.0 as u16 && code == BN_CLICKED as u16 {
        form_ok();
    } else if id == IDCANCEL.0 as u16 {
        with_form(|f| f.done = true);
    } else if id == ID_BROWSE && code == BN_CLICKED as u16 {
        form_browse();
    } else if id == ID_START && code == BN_CLICKED as u16 {
        form_start();
    }
}

fn form_teardown() {
    let form = FORM.with(|c| c.try_borrow_mut().ok().and_then(|mut f| f.take()));
    if let Some(f) = form {
        unsafe {
            if !f.font.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(f.font.0));
            }
        }
    }
}

unsafe extern "system" fn form_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match msg {
        WM_CREATE => {
            if !form_setup(hwnd) {
                return LRESULT(-1);
            }
        }
        WM_SIZE => form_layout(),
        WM_COMMAND => form_command(loword(w.0 as u32), hiword(w.0 as u32)),
        WM_CLOSE => {
            with_form(|f| f.done = true);
        }
        WM_DPICHANGED => {
            let dpi = (w.0 & 0xFFFF) as u32;
            let r = unsafe { &*(l.0 as *const RECT) };
            unsafe {
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    r.left,
                    r.top,
                    r.right - r.left,
                    r.bottom - r.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
            let font = message_font(dpi);
            if !font.is_invalid() {
                let old = with_form(|f| {
                    f.dpi = dpi;
                    replace(&mut f.font, font)
                })
                .unwrap_or_default();
                if let Some((fields, labels, errors, buttons)) = with_form(|f| {
                    (
                        f.field,
                        f.label,
                        f.error,
                        [f.browse, f.start, f.ok, f.cancel, f.hint],
                    )
                }) {
                    for h in fields
                        .into_iter()
                        .chain(labels)
                        .chain(errors)
                        .chain(buttons)
                    {
                        send(h, WM_SETFONT, font.0 as usize, 1);
                    }
                }
                if !old.is_invalid() {
                    unsafe {
                        let _ = DeleteObject(HGDIOBJ(old.0));
                    }
                }
            }
            form_resize();
        }
        WM_NCDESTROY => form_teardown(),
        _ => {}
    }
    unsafe { DefWindowProcW(hwnd, msg, w, l) }
}

// --- validators ---

fn strip_quotes(text: &str) -> String {
    let text = text.trim();
    let inner = text
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .unwrap_or(text);
    inner.to_string()
}

pub fn check_app(name: &str, path: &str, args: &str) -> Result<App, (usize, String)> {
    let name = name.trim();
    let path = strip_quotes(path);
    let args = args.trim();
    if name.contains('\0') || path.contains('\0') || args.contains('\0') {
        let field = if name.contains('\0') {
            0
        } else if path.contains('\0') {
            1
        } else {
            2
        };
        return Err((field, "Invalid character.".to_string()));
    }
    if name.is_empty() {
        return Err((0, "Enter a name.".to_string()));
    }
    actions::program(&path, true).map_err(|e| (1, e))?;
    Ok(App {
        label: name.to_string(),
        path,
        args: args.to_string(),
    })
}

pub fn check_site(name: &str, url: &str, browser: &str) -> Result<Site, (usize, String)> {
    let name = name.trim();
    let url = url.trim();
    let browser = strip_quotes(browser);
    if name.contains('\0') || url.contains('\0') || browser.contains('\0') {
        let field = if name.contains('\0') {
            0
        } else if url.contains('\0') {
            1
        } else {
            2
        };
        return Err((field, "Invalid character.".to_string()));
    }
    if name.is_empty() {
        return Err((0, "Enter a name.".to_string()));
    }
    let url = if url.contains("://") {
        url.to_string()
    } else {
        format!("https://{url}")
    };
    if !config::is_web_url(&url) {
        return Err((1, "Enter an http or https address.".to_string()));
    }
    let browser = if browser.is_empty() {
        None
    } else {
        Some(browser)
    };
    if let Some(exe) = &browser {
        actions::program(exe, false).map_err(|e| (2, e))?;
    }
    Ok(Site {
        label: name.to_string(),
        url,
        browser,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trigger;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_SHIFT, MOD_WIN,
    };

    fn scratch(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!(
            "madar-settings-{name}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn file(dir: &Path, name: &str) -> String {
        let path = dir.join(name);
        fs::write(&path, b"MZ").unwrap();
        path.to_string_lossy().into_owned()
    }

    fn all_mods() -> Mods {
        Mods {
            ctrl: true,
            alt: true,
            shift: true,
            win: true,
        }
    }

    fn letter(vk: VIRTUAL_KEY) -> u32 {
        vk.0 as u32
    }

    fn accepted_keys() -> Vec<u32> {
        let mut keys: Vec<u32> = (0x41..=0x5A).collect();
        keys.extend(0x30..=0x39);
        keys.extend(VK_F1.0 as u32..=VK_F24.0 as u32);
        keys.extend(NAMED.iter().map(|(k, _)| k.0 as u32));
        keys
    }

    #[test]
    fn hotkey_text_round_trips_through_parse() {
        let mods = all_mods();
        let all = HOT_KEY_MODIFIERS(MOD_CONTROL.0 | MOD_ALT.0 | MOD_SHIFT.0 | MOD_WIN.0);
        for vk in accepted_keys() {
            let text = hotkey_text(mods, vk).unwrap_or_else(|| panic!("{vk:#x} refused"));
            assert_eq!(
                trigger::parse_hotkey(&text),
                Some((all, vk)),
                "{text} does not parse back"
            );
        }
        // Mouse order also holds for the single-modifier forms.
        assert_eq!(
            hotkey_text(
                Mods {
                    ctrl: true,
                    ..Default::default()
                },
                letter(VK_SPACE)
            )
            .as_deref(),
            Some("Ctrl+Space")
        );
        assert_eq!(
            hotkey_text(
                Mods {
                    win: true,
                    ..Default::default()
                },
                letter(VK_F13)
            )
            .as_deref(),
            Some("Win+F13")
        );
        assert_eq!(
            hotkey_text(Mods::default(), letter(VK_F13)).as_deref(),
            Some("F13")
        );
    }

    #[test]
    fn hotkey_text_order_and_modifiers() {
        assert_eq!(
            hotkey_text(
                Mods {
                    shift: true,
                    ctrl: true,
                    win: true,
                    alt: true
                },
                0x41
            )
            .as_deref(),
            Some("Ctrl+Alt+Shift+Win+A")
        );
        assert_eq!(
            hotkey_text(
                Mods {
                    win: true,
                    ctrl: true,
                    ..Default::default()
                },
                letter(VK_SPACE)
            )
            .as_deref(),
            Some("Ctrl+Win+Space")
        );
        assert_eq!(
            hotkey_text(
                Mods {
                    alt: true,
                    ctrl: true,
                    ..Default::default()
                },
                0x39
            )
            .as_deref(),
            Some("Ctrl+Alt+9")
        );
    }

    #[test]
    fn hotkey_text_refuses_what_windows_uses() {
        let none = Mods::default();
        assert_eq!(hotkey_text(none, 0x41), None);
        assert_eq!(
            hotkey_text(
                Mods {
                    shift: true,
                    alt: true,
                    ..none
                },
                0x41
            ),
            None
        );
        assert_eq!(hotkey_text(none, letter(VK_F12)), None);
        // Ctrl alone with a clipboard key belongs to Windows.
        for vk in [VK_A, VK_C, VK_V, VK_X, VK_Y, VK_Z, VK_S] {
            assert_eq!(
                hotkey_text(Mods { ctrl: true, ..none }, letter(vk)),
                None,
                "{vk:?}"
            );
        }
        // Any other modifier makes it ours again.
        assert_eq!(
            hotkey_text(
                Mods {
                    ctrl: true,
                    shift: true,
                    ..none
                },
                letter(VK_C)
            )
            .as_deref(),
            Some("Ctrl+Shift+C")
        );
        // Keys the parser has no name for.
        assert_eq!(hotkey_text(Mods { ctrl: true, ..none }, 0x1B), None);
        assert_eq!(hotkey_text(Mods { ctrl: true, ..none }, 0x88), None);
        assert_eq!(hotkey_text(Mods { ctrl: true, ..none }, 0x0A), None);
    }

    #[test]
    fn refusal_names_the_real_key() {
        let ctrl = Mods {
            ctrl: true,
            ..Default::default()
        };
        assert_eq!(refusal(ctrl, letter(VK_C)), "Windows uses Ctrl+C.");
        assert_eq!(refusal(ctrl, letter(VK_S)), "Windows uses Ctrl+S.");
        assert_eq!(refusal(ctrl, 0x1B), "This key cannot be used.");
        let both = Mods {
            ctrl: true,
            shift: true,
            ..Default::default()
        };
        assert_eq!(refusal(both, letter(VK_C)), "This key cannot be used.");
    }

    #[test]
    fn mouse_mapping_both_ways() {
        for (i, &mouse) in MOUSE_ITEMS.iter().enumerate() {
            assert_eq!(mouse_at(i), mouse);
            assert_eq!(mouse_index(mouse), i);
        }
        assert_eq!(mouse_at(9), Mouse::None);
        assert_eq!(
            MOUSE_ITEMS.map(mouse_label),
            ["None", "Back", "Forward", "Ctrl + Middle"]
        );
    }

    #[test]
    fn theme_index_matches_config() {
        for (i, theme) in THEMES.iter().enumerate() {
            assert_eq!(theme_index(theme), i);
        }
        assert_eq!(theme_index("Neon"), 1);
        assert_eq!(THEMES[theme_index("Space Gray")], "Space Gray");
    }

    #[test]
    fn list_edges() {
        assert_eq!(up_index(None), None);
        assert_eq!(up_index(Some(0)), None);
        assert_eq!(up_index(Some(1)), Some(0));
        assert_eq!(down_index(None, 8), None);
        assert_eq!(down_index(Some(0), 0), None);
        assert_eq!(down_index(Some(0), 1), None);
        assert_eq!(down_index(Some(0), 8), Some(1));
        assert_eq!(down_index(Some(6), 8), Some(7));
        assert_eq!(down_index(Some(7), 8), None);
    }

    #[test]
    fn list_full_at_eight() {
        assert_eq!(MAX_ITEMS, 8);
        for len in 0..MAX_ITEMS {
            assert!(len < MAX_ITEMS, "{len}");
        }
        // Moving at the edges of a full list still works except at the ends.
        assert_eq!(up_index(Some(7)), Some(6));
        assert_eq!(down_index(Some(7), 8), None);
    }

    #[test]
    fn selection_after_remove() {
        assert_eq!(after_remove(0, 0), None);
        assert_eq!(after_remove(0, 1), None);
        assert_eq!(after_remove(0, 2), Some(0));
        assert_eq!(after_remove(1, 2), Some(0));
        assert_eq!(after_remove(1, 3), Some(1));
        assert_eq!(after_remove(2, 3), Some(1));
        assert_eq!(after_remove(7, 8), Some(6));
    }

    #[test]
    fn app_rules() {
        let dir = scratch("app");
        let path = file(&dir, "My App.exe");
        let quoted = format!("\"{path}\"");
        let app = check_app("  Name  ", &quoted, "  --open  ").unwrap();
        assert_eq!(
            app,
            App {
                label: "Name".to_string(),
                path: path.clone(),
                args: "--open".to_string(),
            }
        );
        assert_eq!(
            check_app("   ", &path, "").unwrap_err(),
            (0, "Enter a name.".to_string())
        );
        assert_eq!(check_app("a\0b", &path, "").unwrap_err().0, 0);
        assert_eq!(
            check_app("a", "relative.exe", "").unwrap_err(),
            (1, "Full path needed".to_string())
        );
        assert_eq!(check_app("a", "", "").unwrap_err().0, 1);
        assert_eq!(check_app("a", &path, "x\0y").unwrap_err().0, 2);
        assert_eq!(
            check_app("تحميل المستندات", &path, "").unwrap().label,
            "تحميل المستندات"
        );
        let link = file(&dir, "Browser.lnk");
        assert!(check_app("a", &link, "").is_ok());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn site_rules() {
        let dir = scratch("site");
        let browser = file(&dir, "browser.exe");
        let quoted = format!("\"{browser}\"");
        let site = check_site("  بحث  ", "  example.com  ", "").unwrap();
        assert_eq!(
            site,
            Site {
                label: "بحث".to_string(),
                url: "https://example.com".to_string(),
                browser: None,
            }
        );
        let site = check_site("a", "HTTPS://X.COM/p?q=1", &quoted).unwrap();
        assert_eq!(site.url, "HTTPS://X.COM/p?q=1");
        assert_eq!(site.browser.as_deref(), Some(browser.as_str()));
        assert_eq!(
            check_site("", "https://a.b", "").unwrap_err(),
            (0, "Enter a name.".to_string())
        );
        assert_eq!(
            check_site("a", "   ", "").unwrap_err(),
            (1, "Enter an http or https address.".to_string())
        );
        assert_eq!(check_site("a", "http://exa mple.com", "").unwrap_err().0, 1);
        assert_eq!(check_site("a", "https://a.b", "\0").unwrap_err().0, 2);
        let gone = dir.join("gone.exe").to_string_lossy().into_owned();
        assert_eq!(
            check_site("a", "https://a.b", &gone).unwrap_err(),
            (2, "File not found".to_string())
        );
        let lnk = file(&dir, "b.lnk");
        assert_eq!(
            check_site("a", "https://a.b", &lnk).unwrap_err(),
            (2, "Not a program".to_string())
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    fn handle(n: usize) -> HWND {
        HWND(core::ptr::without_provenance_mut::<std::ffi::c_void>(n))
    }

    fn dummy_apply(_: Config) -> Result<(), Problem> {
        Ok(())
    }

    /// Only the numbers matter here: the controls are made-up handles.
    fn dummy_state(errors: [Option<String>; 6]) -> State {
        let mut w = Widgets::default();
        for (i, field) in [
            &mut w.hotkey,
            &mut w.mouse,
            &mut w.theme,
            &mut w.autostart,
            &mut w.names,
            &mut w.items,
        ]
        .into_iter()
        .enumerate()
        {
            *field = handle(1 + i);
        }
        w.buttons = std::array::from_fn(|i| handle(10 + i));
        w.tabs = std::array::from_fn(|i| handle(20 + i));
        w.label = std::array::from_fn(|i| handle(30 + i));
        w.error = std::array::from_fn(|i| handle(40 + i));
        State {
            hwnd: handle(99),
            host: Host {
                get: Config::default,
                apply: dummy_apply,
            },
            list: List::MyApps,
            dpi: 96,
            font: HFONT::default(),
            big: HICON::default(),
            small: HICON::default(),
            owned: false,
            w,
            errors,
            busy: 0,
            captured: 0,
        }
    }

    fn rect(plan: &[(HWND, i32, i32, i32, i32)], h: HWND) -> (i32, i32, i32, i32) {
        let r = plan.iter().find(|r| r.0 == h).expect("control planned");
        (r.1, r.2, r.3, r.4)
    }

    #[test]
    fn layout_puts_rows_in_order_and_gives_the_rest_to_the_lists() {
        let s = dummy_state(Default::default());
        let plan = arrange(&s, 480, 560);
        for (h, x, y, w, h2) in &plan {
            assert!(x >= &0 && y >= &0 && w > &0 && h2 >= &0, "{h:?}");
            assert!(x + w <= 480 && y + h2 <= 560, "{h:?}");
        }
        // 16 margin, then one 36 row per field.
        assert_eq!(rect(&plan, handle(1)).1, 16);
        assert_eq!(rect(&plan, handle(2)).1, 52);
        assert_eq!(rect(&plan, handle(3)).1, 88);
        assert_eq!(rect(&plan, handle(4)).1, 124);
        assert_eq!(rect(&plan, handle(5)).1, 160);
        // The switch, then the list takes the rest.
        assert_eq!(rect(&plan, handle(20)).1, 196);
        assert_eq!(rect(&plan, handle(6)), (126, 232, 222, 312));
        let end = |plan: &Vec<_>, h| rect(plan, h).1 + rect(plan, h).3;
        assert!(end(&plan, handle(16)) <= 560 - 16);
        // Extra height goes to the list, extra width to the right column.
        let tall = arrange(&s, 600, 600);
        assert_eq!(rect(&tall, handle(6)).3, 352);
        assert_eq!(rect(&tall, handle(1)).1, 16);
        // An error under a field moves every row below it down by its height.
        let with_error = dummy_state([Some("x".to_string()), None, None, None, None, None]);
        let plan = arrange(&with_error, 480, 560);
        assert_eq!(rect(&plan, handle(2)).1, 72);
        assert_eq!(rect(&plan, handle(6)).1, 252);
        assert_eq!(rect(&plan, handle(40)).3, 20);
        // Every error at once still fits.
        let plan = arrange(
            &dummy_state(std::array::from_fn(|_| Some("x".into()))),
            480,
            560,
        );
        assert!(end(&plan, handle(16)) <= 560);
        assert!(end(&plan, handle(45)) <= 560);
    }

    #[test]
    fn items_move_between_the_lists() {
        let app = |n: &str| App {
            label: n.to_string(),
            path: "C:\\a.exe".to_string(),
            args: String::new(),
        };
        let mut c = Config {
            my_apps: vec![app("a"), app("b")],
            ..Config::default()
        };
        assert_eq!(
            enabled(List::MyApps, &c, None)[..4],
            [true, true, false, false]
        );
        assert!(enabled(List::MyApps, &c, Some(0))[MOVE]);
        let made = List::MyApps.take(&mut c, 0).unwrap();
        List::Apps.put(&mut c, 0, made);
        assert_eq!(List::MyApps.labels(&c), ["b"]);
        assert_eq!(List::Apps.labels(&c), ["a"]);
        assert!(List::Apps.take(&mut c, 5).is_none());
        // A site never lands in a list of programs.
        List::Sites.put(&mut c, 0, Made::App(app("x")));
        assert!(c.sites.is_empty());
        c.apps = vec![app("z"); MAX_ITEMS];
        assert!(!enabled(List::MyApps, &c, Some(0))[MOVE]);
        assert!(!enabled(List::Apps, &c, Some(0))[ADD]);
        assert!(!enabled(List::Sites, &c, None)[START]);
        List::Apps.swap(&mut c, 0, 99);
        assert_eq!(List::Apps.other(), Some(List::MyApps));
    }
}
