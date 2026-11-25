#![windows_subsystem = "windows"]

mod config;
mod trigger;
mod wheel;

use config::{Config, Mouse};
use std::cell::RefCell;
use std::path::PathBuf;
use std::time::Instant;
use trigger::{MouseHook, unpack};
use wheel::paint::{Painter, Scene, palette};
use wheel::{Action, Hit, Page, Slot, Target};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{BOOL, HSTRING, PCWSTR, w};

const CLASS: PCWSTR = w!("MadarWheel");
/// A second launch asks the first one to open the wheel.
const WM_SHOW_WHEEL: u32 = WM_APP + 10;
const SUMMON_KEY: i32 = 1;
const FIRST_WHEEL_KEY: i32 = 100;
const ANIMATION: usize = 1;
const MESSAGE: usize = 2;
const ANIMATION_MS: f32 = 120.0;
/// Pointer travel, in logical pixels, that turns a press into a drag.
const DRAG: f32 = 20.0;

/// Keys the wheel takes from other programs while it is visible.
const WHEEL_KEYS: [(HOT_KEY_MODIFIERS, VIRTUAL_KEY); 25] = {
    let none = HOT_KEY_MODIFIERS(0);
    let mut keys = [(none, VK_ESCAPE); 25];
    let named = [
        VK_RETURN, VK_BACK, VK_TAB, VK_LEFT, VK_UP, VK_RIGHT, VK_DOWN,
    ];
    let mut i = 0;
    while i < named.len() {
        keys[1 + i].1 = named[i];
        i += 1;
    }
    let mut d = 0;
    while d < 8 {
        keys[8 + d].1 = VIRTUAL_KEY(VK_1.0 + d as u16);
        keys[16 + d].1 = VIRTUAL_KEY(VK_NUMPAD1.0 + d as u16);
        d += 1;
    }
    keys[24] = (MOD_SHIFT, VK_TAB);
    keys
};

struct App {
    hwnd: HWND,
    config: Config,
    painter: Painter,
    hook: MouseHook,
    visible: bool,
    center: POINT,
    scale: f32,
    page: Page,
    slots: Vec<Slot>,
    hot: Option<usize>,
    hub_hot: bool,
    pressed: Option<Hit>,
    /// Where the summon button went down, while it is held.
    summon: Option<POINT>,
    dragged: bool,
    message: Option<String>,
    animation: Option<Instant>,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// Runs `f` unless the app is already busy in an outer call or not started.
fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|a| a.try_borrow_mut().ok()?.as_mut().map(f))
}

fn cursor() -> POINT {
    let mut p = POINT::default();
    let _ = unsafe { GetCursorPos(&mut p) };
    p
}

fn animations_on() -> bool {
    let mut on = BOOL(1);
    let _ = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some((&raw mut on).cast()),
            Default::default(),
        )
    };
    on.as_bool()
}

impl App {
    fn open(&mut self, animate: bool) {
        let p = cursor();
        let (mut dpi, mut unused) = (96, 0);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        unsafe {
            let monitor = MonitorFromPoint(p, MONITOR_DEFAULTTONEAREST);
            let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut unused);
            let _ = GetMonitorInfoW(monitor, &mut info);
        }
        self.scale = dpi as f32 / 96.0;
        let r = (wheel::PLATE_R * self.scale).ceil() as i32;
        self.center = wheel::place(p, r, info.rcWork);
        self.message = None;
        self.pressed = None;
        self.summon = None;
        self.animation = (animate && animations_on()).then(Instant::now);
        self.go(Page::Root);
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
            if self.animation.is_some() {
                SetTimer(Some(self.hwnd), ANIMATION, 10, None);
            }
            if !self.visible {
                for (id, (mods, key)) in (FIRST_WHEEL_KEY..).zip(WHEEL_KEYS) {
                    // A key some other program owns simply stays with it.
                    let _ = RegisterHotKey(Some(self.hwnd), id, mods, key.0 as u32);
                }
            }
        }
        self.visible = true;
        // Without the hook only closing by an outside click is lost.
        let _ = self.hook.update(self.config.mouse, true);
    }

    fn close(&mut self) {
        if !self.visible {
            return;
        }
        self.visible = false;
        self.animation = None;
        self.pressed = None;
        self.summon = None;
        unsafe {
            let _ = ReleaseCapture();
            let _ = KillTimer(Some(self.hwnd), ANIMATION);
            let _ = KillTimer(Some(self.hwnd), MESSAGE);
            for (id, _) in (FIRST_WHEEL_KEY..).zip(WHEEL_KEYS) {
                let _ = UnregisterHotKey(Some(self.hwnd), id);
            }
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
        let _ = self.hook.update(self.config.mouse, false);
    }

    fn go(&mut self, page: Page) {
        self.page = page;
        self.slots = wheel::slots(page, &self.config);
        self.hot = None;
        self.hub_hot = false;
        self.draw();
    }

    fn draw(&mut self) {
        let hot_slot = self.hot.and_then(|i| self.slots.get(i));
        let hub = match (&self.message, hot_slot) {
            (Some(m), _) => m.as_str(),
            (None, Some(s)) if s.target == Target::Disabled => "Not ready",
            (None, Some(s)) => s.label.as_str(),
            (None, None) if self.page == Page::Root => "Close",
            (None, None) => "Back",
        };
        let progress = match self.animation {
            Some(start) => {
                let t = (start.elapsed().as_secs_f32() * 1000.0 / ANIMATION_MS).min(1.0);
                1.0 - (1.0 - t).powi(3)
            }
            None => 1.0,
        };
        let scene = Scene {
            slots: &self.slots,
            hot: self.hot,
            pressed: match self.pressed {
                Some(Hit::Slot(i)) => Some(i),
                _ => None,
            },
            hub,
            hub_hot: self.hub_hot && self.message.is_none(),
            palette: palette(&self.config.theme),
            progress,
        };
        // A failed frame leaves the previous one; the next event draws again.
        let _ = self
            .painter
            .present(self.hwnd, self.center, self.scale, &scene);
    }

    fn offset(&self, p: POINT) -> (f32, f32) {
        let d = |v: i32, c: i32| (v - c) as f32 / self.scale;
        (d(p.x, self.center.x), d(p.y, self.center.y))
    }

    fn hit(&self, p: POINT) -> Hit {
        let (dx, dy) = self.offset(p);
        wheel::hit(dx, dy, self.slots.len())
    }

    fn set_hot(&mut self, h: Hit) {
        let hot = match h {
            Hit::Slot(i) => Some(i),
            _ => None,
        };
        let hub_hot = h == Hit::Hub;
        if (hot, hub_hot) != (self.hot, self.hub_hot) {
            (self.hot, self.hub_hot) = (hot, hub_hot);
            self.draw();
        }
    }

    fn fail(&mut self, text: &str) {
        self.message = Some(text.to_string());
        unsafe { SetTimer(Some(self.hwnd), MESSAGE, 4000, None) };
        self.draw();
    }

    fn back(&mut self) {
        if self.page == Page::Root {
            self.close();
        } else {
            self.message = None;
            self.go(Page::Root);
        }
    }

    fn choose(&mut self, i: usize) {
        let Some(slot) = self.slots.get(i) else {
            return;
        };
        self.message = None;
        match slot.target {
            Target::Open(page) => self.go(page),
            Target::Disabled => {
                self.hot = Some(i);
                self.hub_hot = false;
                self.fail("Not ready");
            }
            Target::Do(action) => match run(action) {
                Ok(()) => self.close(),
                Err(e) => self.fail(&e),
            },
        }
    }

    fn activate(&mut self, h: Hit) {
        match h {
            Hit::Hub => self.back(),
            Hit::Slot(i) => self.choose(i),
            Hit::Outside => {}
        }
    }

    fn step(&mut self, by: isize) {
        let n = self.slots.len() as isize;
        let next = match self.hot {
            Some(i) => (i as isize + by).rem_euclid(n),
            None if by > 0 => 0,
            None => n - 1,
        };
        self.set_hot(Hit::Slot(next as usize));
    }

    fn key(&mut self, id: i32) {
        let Some(&(mods, key)) = WHEEL_KEYS.get((id - FIRST_WHEEL_KEY) as usize) else {
            return;
        };
        match key {
            VK_ESCAPE => self.close(),
            VK_BACK => self.back(),
            VK_RETURN => match self.hot {
                Some(i) => self.choose(i),
                None => self.back(),
            },
            VK_TAB if mods == MOD_SHIFT => self.step(-1),
            VK_TAB | VK_RIGHT | VK_DOWN => self.step(1),
            VK_LEFT | VK_UP => self.step(-1),
            VIRTUAL_KEY(k) if k >= VK_NUMPAD1.0 => self.choose((k - VK_NUMPAD1.0) as usize),
            VIRTUAL_KEY(k) => self.choose((k - VK_1.0) as usize),
        }
    }

    fn summon_down(&mut self, p: POINT) {
        if self.visible {
            self.close();
        } else {
            self.open(true);
            self.summon = Some(p);
            self.dragged = false;
        }
    }

    /// While the summon button is held, direction alone picks a slot.
    fn drag_hit(&self, p: POINT) -> Hit {
        match self.hit(p) {
            Hit::Outside => {
                let (dx, dy) = self.offset(p);
                Hit::Slot(wheel::slot_at_angle(dx, dy, self.slots.len()))
            }
            h => h,
        }
    }

    fn summon_move(&mut self, p: POINT) {
        let Some(start) = self.summon else { return };
        let (dx, dy) = ((p.x - start.x) as f32, (p.y - start.y) as f32);
        self.dragged |= dx.hypot(dy) > DRAG * self.scale;
        if self.dragged {
            self.set_hot(self.drag_hit(p));
        }
    }

    fn summon_up(&mut self, p: POINT) {
        self.summon_move(p);
        if self.summon.take().is_some() && self.dragged {
            // Letting go in the middle only ends the drag.
            if let Hit::Slot(i) = self.drag_hit(p) {
                self.choose(i);
            }
        }
        // Lets a hook kept alive by the held button stop when it is not needed.
        let _ = self.hook.update(self.config.mouse, self.visible);
    }

    fn left_up(&mut self, p: POINT) {
        let Some(down) = self.pressed.take() else {
            return;
        };
        let _ = unsafe { ReleaseCapture() };
        let up = self.hit(p);
        let from_hub = down == Hit::Hub && matches!(up, Hit::Slot(_));
        if up == down || from_hub {
            self.activate(up);
        }
        if self.visible {
            self.draw();
        }
    }
}

/// Filled in by the `actions` module.
fn run(_action: Action) -> Result<(), String> {
    Err("Not ready".to_string())
}

fn settings_path() -> PathBuf {
    // MADAR_DIR points tests at a scratch folder.
    let dir = match std::env::var_os("MADAR_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default()).join("Madar"),
    };
    dir.join("config.json")
}

fn warn(text: &str) {
    unsafe {
        MessageBoxW(
            None,
            &HSTRING::from(text),
            w!("Madar"),
            MB_OK | MB_ICONWARNING,
        )
    };
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    let client = || {
        let mut p = POINT {
            x: l.0 as i16 as i32,
            y: (l.0 >> 16) as i16 as i32,
        };
        let _ = unsafe { ClientToScreen(hwnd, &mut p) };
        p
    };
    let handled = match msg {
        WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
        WM_HOTKEY => with_app(|a| match w.0 as i32 {
            SUMMON_KEY if a.visible => a.close(),
            SUMMON_KEY => a.open(false),
            id if a.visible => a.key(id),
            _ => {}
        }),
        WM_SHOW_WHEEL => with_app(|a| a.open(false)),
        trigger::WM_SUMMON_DOWN => with_app(|a| a.summon_down(unpack(l))),
        trigger::WM_SUMMON_MOVE => with_app(|a| a.summon_move(unpack(l))),
        trigger::WM_SUMMON_UP => with_app(|a| a.summon_up(unpack(l))),
        trigger::WM_PRESS => with_app(|a| {
            if a.visible && a.hit(unpack(l)) == Hit::Outside {
                a.close();
            }
        }),
        WM_MOUSEMOVE => with_app(|a| {
            if !a.visible || a.summon.is_some() {
                return;
            }
            let mut leave = TRACKMOUSEEVENT {
                cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: hwnd,
                dwHoverTime: 0,
            };
            let _ = unsafe { TrackMouseEvent(&mut leave) };
            a.set_hot(a.hit(client()));
        }),
        WM_MOUSELEAVE => with_app(|a| {
            if a.visible && a.summon.is_none() {
                a.set_hot(Hit::Outside);
            }
        }),
        WM_LBUTTONDOWN => with_app(|a| {
            a.pressed = Some(a.hit(client()));
            unsafe { SetCapture(hwnd) };
            a.draw();
        }),
        WM_LBUTTONUP => with_app(|a| a.left_up(client())),
        WM_TIMER => with_app(|a| {
            let running = |s: Instant| s.elapsed().as_secs_f32() * 1000.0 < ANIMATION_MS;
            if w.0 == MESSAGE {
                a.message = None;
                let _ = unsafe { KillTimer(Some(hwnd), MESSAGE) };
            } else if !a.animation.is_some_and(running) {
                a.animation = None;
                let _ = unsafe { KillTimer(Some(hwnd), ANIMATION) };
            }
            if a.visible {
                a.draw();
            }
        }),
        WM_DISPLAYCHANGE => with_app(App::close),
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            Some(())
        }
        _ => None,
    };
    match handled {
        Some(()) => LRESULT(0),
        None => unsafe { DefWindowProcW(hwnd, msg, w, l) },
    }
}

fn create_window() -> windows::core::Result<HWND> {
    unsafe {
        let module = GetModuleHandleW(None)?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: module.into(),
            lpszClassName: CLASS,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            ..Default::default()
        };
        RegisterClassW(&class);
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_NOACTIVATE | WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            CLASS,
            w!("Madar"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(module.into()),
            None,
        )
    }
}

fn start(config: Config) -> windows::core::Result<HWND> {
    let hwnd = create_window()?;
    let mut app = App {
        hwnd,
        config,
        painter: Painter::new()?,
        hook: MouseHook::new(hwnd),
        visible: false,
        center: POINT::default(),
        scale: 1.0,
        page: Page::Root,
        slots: Vec::new(),
        hot: None,
        hub_hot: false,
        pressed: None,
        summon: None,
        dragged: false,
        message: None,
        animation: None,
    };
    // Draws once while hidden, so the first real frame is fast.
    app.go(Page::Root);
    APP.with(|a| *a.borrow_mut() = Some(app));
    Ok(hwnd)
}

fn main() {
    unsafe {
        // Held until the process ends.
        let mutex = CreateMutexW(None, false, w!("Local\\Madar.SingleInstance"));
        if mutex.is_ok() && GetLastError() == ERROR_ALREADY_EXISTS {
            if let Ok(first) = FindWindowW(CLASS, None) {
                let _ = PostMessageW(Some(first), WM_SHOW_WHEEL, WPARAM(0), LPARAM(0));
            }
            return;
        }
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let mut problems = Vec::new();
    let config = config::load(&settings_path()).unwrap_or_else(|e| {
        problems.push(format!(
            "Settings could not be read, so defaults are in use.\n{e}"
        ));
        Config::default()
    });
    let hotkey = config.hotkey.clone();
    let mouse: Mouse = config.mouse;
    let hwnd = match start(config) {
        Ok(hwnd) => hwnd,
        Err(e) => return warn(&format!("Madar could not start: {e}")),
    };
    let registered = trigger::parse_hotkey(&hotkey).is_some_and(|(mods, key)| unsafe {
        RegisterHotKey(Some(hwnd), SUMMON_KEY, mods | MOD_NOREPEAT, key).is_ok()
    });
    if !registered {
        problems.push(format!(
            "The shortcut {hotkey} is invalid or used by another program."
        ));
    }
    if let Some(Err(e)) = with_app(|a| a.hook.update(mouse, false)) {
        problems.push(e);
    }
    if !problems.is_empty() {
        warn(&problems.join("\n\n"));
    }
    unsafe {
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    // Stops the hook thread before the process ends.
    APP.with(|a| a.take());
}
