#![windows_subsystem = "windows"]

mod actions;
mod audio;
mod autostart;
mod bluetooth;
mod brightness;
mod config;
mod icons;
mod maintenance;
mod search;
mod settings;
mod tray;
mod trigger;
mod wheel;

use audio::output::Device;
use audio::{Audio, Order, Volume};
use brightness::Light;
use config::{Config, Mouse};
use settings::{Field, Problem};
use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::Instant;
use tray::Tray;
use trigger::{MouseHook, unpack};
use wheel::paint::{Painter, Scene, palette};
use wheel::{Action, Hit, Page, Slot, Target};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::DataExchange::AddClipboardFormatListener;
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
/// An action finished; its result waits in `App::running`.
const WM_ACTION_DONE: u32 = WM_APP + 11;
/// Opens the settings window once the app is no longer borrowed.
const WM_OPEN_SETTINGS: u32 = WM_APP + 12;
const SUMMON_KEY: i32 = 1;
const FIRST_WHEEL_KEY: i32 = 100;
const ANIMATION: usize = 1;
const MESSAGE: usize = 2;
/// Reads the sound again while its page shows, so a device that joins appears.
const DEVICES: usize = 3;
const DEVICES_MS: u32 = 1000;
/// Turns the screen off. The wait keeps the press itself from waking it.
const SCREEN: usize = 4;
const SCREEN_MS: u32 = 1000;
/// `SC_MONITORPOWER`: off.
const POWER_OFF: isize = 2;
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
    /// The settings file could not be read; it is never replaced without asking.
    unreadable: bool,
    painter: Painter,
    icons: icons::Icons,
    hook: MouseHook,
    tray: Tray,
    /// Message Explorer broadcasts after it restarts.
    taskbar_created: u32,
    /// Shortcut and mouse button are off.
    paused: bool,
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
    /// Result of the action running on a worker thread.
    running: Option<Receiver<Result<(), String>>>,
    /// The last press on Restart Explorer.
    restart_pressed: Option<Instant>,
    /// Open while the wheel uses the sound.
    audio: Option<Audio>,
    /// The last answer; cleared when the wheel closes.
    volume: Option<Result<Volume, String>>,
    devices: Vec<Device>,
    /// Open while the wheel uses the brightness.
    light: Option<Light>,
    /// The last answer; cleared when the wheel closes.
    brightness: Option<brightness::Answer>,
    /// Scrolling not yet worth one percent.
    scroll_rest: i32,
    /// Reading or switching the Bluetooth radio. Kept when the wheel closes,
    /// so a second switch never starts during the first.
    radio: Option<Receiver<bluetooth::Answer>>,
    radio_switching: bool,
    /// The last answer; cleared when the wheel closes.
    radio_state: Option<bluetooth::Answer>,
    /// Until then a press on Bluetooth does nothing.
    radio_lock: Option<Instant>,
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
        self.icons.request(&self.config, self.scale);
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
        let _ = self.hook.update(self.button(), true);
    }

    fn close(&mut self) {
        if !self.visible {
            return;
        }
        self.visible = false;
        // A result that arrives after closing belongs to nobody.
        self.running = None;
        self.audio = None;
        self.volume = None;
        self.devices.clear();
        self.light = None;
        self.brightness = None;
        self.scroll_rest = 0;
        self.radio_state = None;
        self.animation = None;
        self.pressed = None;
        self.summon = None;
        unsafe {
            let _ = ReleaseCapture();
            let _ = KillTimer(Some(self.hwnd), ANIMATION);
            let _ = KillTimer(Some(self.hwnd), MESSAGE);
            let _ = KillTimer(Some(self.hwnd), DEVICES);
            for (id, _) in (FIRST_WHEEL_KEY..).zip(WHEEL_KEYS) {
                let _ = UnregisterHotKey(Some(self.hwnd), id);
            }
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
        let _ = self.hook.update(self.button(), false);
    }

    fn go(&mut self, page: Page) {
        self.page = page;
        self.slots = wheel::slots(page, &self.config);
        self.hot = None;
        self.hub_hot = false;
        if matches!(page, Page::MyApps | Page::Apps | Page::Tools) {
            self.icons.request(&self.config, self.scale);
            self.show_icons();
        }
        if page == Page::Sound {
            if !self.radio_switching {
                self.radio = Some(bluetooth::start(self.hwnd, false));
            }
            self.sound(Order::Read);
            self.shine(brightness::Order::Read);
            self.show_sound();
            unsafe { SetTimer(Some(self.hwnd), DEVICES, DEVICES_MS, None) };
        } else {
            let _ = unsafe { KillTimer(Some(self.hwnd), DEVICES) };
        }
        self.draw();
    }

    fn switch_bluetooth(&mut self) {
        let now = Instant::now();
        if self.radio_switching || self.radio_lock.is_some_and(|until| now < until) {
            // A held key keeps coming; it counts as one press.
            self.radio_lock = Some(now + bluetooth::LOCK);
        } else {
            self.radio_switching = true;
            self.radio = Some(bluetooth::start(self.hwnd, true));
        }
        self.draw();
    }

    fn radio_answered(&mut self) {
        use bluetooth::Answer;
        let mut failed = false;
        while let Some(rx) = &self.radio {
            match rx.try_recv() {
                Ok(answer) => {
                    failed |= answer == Answer::Failed;
                    if self.visible {
                        self.radio_state = Some(answer);
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.radio = None;
                    if std::mem::take(&mut self.radio_switching) {
                        self.radio_lock = Some(Instant::now() + bluetooth::LOCK);
                    }
                }
            }
        }
        if !self.visible {
            return;
        }
        self.draw();
        if failed {
            self.launch(Action::BluetoothSettings);
        }
    }

    /// What the hub says about Bluetooth, after its name.
    fn radio_text(&self) -> Option<String> {
        use bluetooth::Answer;
        let slot = self.hot.and_then(|i| self.slots.get(i))?;
        if slot.target != Target::Do(Action::Bluetooth) {
            return None;
        }
        let value = match self.radio_state? {
            Answer::Working(true) => "Turning on...",
            Answer::Working(false) => "Turning off...",
            Answer::Is(true) => "On",
            Answer::Is(false) => "Off",
            Answer::Failed => return None,
        };
        Some(format!("{}\n{value}", slot.label))
    }

    fn sound(&mut self, order: Order) {
        let hwnd = self.hwnd;
        self.audio
            .get_or_insert_with(|| Audio::new(hwnd))
            .send(order);
    }

    fn shine(&mut self, order: brightness::Order) {
        let hwnd = self.hwnd;
        self.light
            .get_or_insert_with(|| Light::new(hwnd))
            .send(order);
    }

    fn light_answered(&mut self) {
        let answers = self.light.as_ref().map(Light::answers).unwrap_or_default();
        if let Some(&answer) = answers.last() {
            self.brightness = Some(answer);
            self.draw();
        }
    }

    fn on_brightness_slot(&self) -> bool {
        let slot = self.hot.and_then(|i| self.slots.get(i));
        slot.is_some_and(|s| {
            matches!(
                s.target,
                Target::Do(Action::BrightnessUp | Action::BrightnessDown)
            )
        })
    }

    /// What the hub says about the brightness.
    fn light_text(&self) -> Option<String> {
        if !self.on_brightness_slot() {
            return None;
        }
        let slot = self.hot.and_then(|i| self.slots.get(i))?;
        Some(match self.brightness? {
            Some(percent) => format!("{}\n{percent}%", slot.label),
            None => "Brightness not available".to_string(),
        })
    }

    fn sound_answered(&mut self) {
        let answers = self.audio.as_ref().map(Audio::answers).unwrap_or_default();
        let mut failed = false;
        for answer in answers {
            self.volume = Some(answer.volume);
            if let Some(devices) = answer.devices
                && devices != self.devices
            {
                self.devices = devices;
                // A half-made press must not choose what moved under it.
                if self.page == Page::Sound
                    && matches!(self.pressed, Some(Hit::Slot(i)) if i >= wheel::FIRST_OUTPUT)
                {
                    self.pressed = None;
                }
            }
            failed |= answer.failed;
        }
        self.show_sound();
        self.draw();
        if failed {
            self.launch(Action::SoundSettings);
        }
    }

    fn show_sound(&mut self) {
        if self.page == Page::Sound {
            self.slots.truncate(wheel::FIRST_OUTPUT);
            self.slots.extend(wheel::outputs(&self.devices));
            // A device may have left the place under the pointer.
            self.hot = self.hot.filter(|&i| !self.is_empty(i));
        }
        let muted = matches!(self.volume, Some(Ok(v)) if v.muted);
        for slot in &mut self.slots {
            if slot.target == Target::Do(Action::Mute) {
                let (label, icon) = if muted {
                    ("Unmute", '\u{E767}')
                } else {
                    ("Mute", '\u{E74F}')
                };
                (slot.label, slot.icon) = (label.to_string(), icon);
            }
        }
    }

    fn on_sound_slot(&self) -> bool {
        let slot = self.hot.and_then(|i| self.slots.get(i));
        slot.is_some_and(|s| s.target == Target::Open(Page::Sound))
    }

    fn scroll(&mut self, delta: i32) {
        if !self.visible || !(self.page == Page::Sound || self.on_sound_slot()) {
            return;
        }
        let by = audio::scrolled(&mut self.scroll_rest, delta);
        if by == 0 {
            return;
        }
        if self.on_brightness_slot() {
            self.shine(brightness::Order::Change(by));
        } else {
            self.sound(Order::Change(by));
        }
    }

    /// What the hub says about the sound, after the name of the hot slot.
    fn volume_text(&self) -> Option<String> {
        if self.on_brightness_slot() {
            return None;
        }
        let on_page = self.page == Page::Sound && !self.hub_hot;
        if !on_page && !self.on_sound_slot() {
            return None;
        }
        let value = match self.volume.as_ref()? {
            Ok(v) if v.muted => "Muted".to_string(),
            Ok(v) => format!("{}%", v.percent),
            Err(e) => e.clone(),
        };
        let slot = self.hot.and_then(|i| self.slots.get(i));
        Some(match slot {
            // Their name is the news.
            Some(s)
                if matches!(
                    s.target,
                    Target::Do(Action::Output(_) | Action::SoundSettings | Action::Bluetooth)
                ) =>
            {
                return None;
            }
            Some(s) => format!("{}\n{value}", s.label),
            None => value,
        })
    }

    fn show_icons(&mut self) {
        for slot in &mut self.slots {
            let app = match slot.target {
                Target::Do(Action::MyApp(i)) => self.config.my_apps.get(i),
                Target::Do(Action::App(i)) => self.config.apps.get(i),
                Target::Do(Action::Tool(i)) => self.config.tools.get(i),
                _ => None,
            };
            if app.is_some() {
                slot.image = app.and_then(|a| self.icons.get(&a.path));
            }
        }
    }

    fn icon_loaded(&mut self) {
        self.icons.collect();
        if self.visible && matches!(self.page, Page::MyApps | Page::Apps | Page::Tools) {
            self.show_icons();
            self.draw();
        }
    }

    fn draw(&mut self) {
        let hot_slot = self.hot.and_then(|i| self.slots.get(i));
        let status = (self.volume_text())
            .or_else(|| self.radio_text())
            .or_else(|| self.light_text());
        let hub = match (&self.message, hot_slot) {
            (Some(m), _) => m.as_str(),
            _ if status.is_some() => status.as_deref().unwrap_or_default(),
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
            names: self.config.show_names,
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

    /// The summon button in effect.
    fn button(&self) -> Mouse {
        if self.paused {
            Mouse::None
        } else {
            self.config.mouse
        }
    }

    /// Turns the shortcut and the mouse button on or off. Returns what failed.
    fn apply_triggers(&mut self) -> Vec<String> {
        let mut problems: Vec<String> = self.register_shortcut().into_iter().collect();
        problems.extend(self.hook.update(self.button(), self.visible).err());
        problems
    }

    fn register_shortcut(&mut self) -> Option<String> {
        let _ = unsafe { UnregisterHotKey(Some(self.hwnd), SUMMON_KEY) };
        let register = |(mods, key)| unsafe {
            RegisterHotKey(Some(self.hwnd), SUMMON_KEY, mods | MOD_NOREPEAT, key).is_ok()
        };
        let failed =
            !self.paused && !trigger::parse_hotkey(&self.config.hotkey).is_some_and(register);
        failed.then(|| {
            format!(
                "The shortcut {} is invalid or used by another program.",
                self.config.hotkey
            )
        })
    }

    fn toggle_pause(&mut self) -> Vec<String> {
        self.paused = !self.paused;
        self.tray.set_paused(self.paused);
        self.apply_triggers()
    }

    fn offset(&self, p: POINT) -> (f32, f32) {
        let d = |v: i32, c: i32| (v - c) as f32 / self.scale;
        (d(p.x, self.center.x), d(p.y, self.center.y))
    }

    fn hit(&self, p: POINT) -> Hit {
        let (dx, dy) = self.offset(p);
        wheel::hit(dx, dy, self.slots.len())
    }

    fn is_empty(&self, i: usize) -> bool {
        self.slots.get(i).is_none_or(|s| s.target == Target::Empty)
    }

    fn set_hot(&mut self, h: Hit) {
        let hot = match h {
            Hit::Slot(i) if !self.is_empty(i) => Some(i),
            _ => None,
        };
        let hub_hot = h == Hit::Hub;
        if (hot, hub_hot) != (self.hot, self.hub_hot) {
            (self.hot, self.hub_hot) = (hot, hub_hot);
            self.draw();
        }
    }

    fn tell(&mut self, text: &str) {
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
        let message = self.message.take();
        match slot.target {
            Target::Open(page) => self.go(page),
            Target::Empty => {}
            Target::Disabled => {
                self.hot = Some(i);
                self.hub_hot = false;
                self.tell("Not ready");
            }
            Target::Do(Action::AddItems) => {
                self.close();
                // Which list, counted from 1.
                let list = match self.page {
                    Page::MyApps => 1,
                    Page::Apps => 2,
                    Page::Tools => 3,
                    _ => 4,
                };
                let _ = unsafe {
                    PostMessageW(Some(self.hwnd), WM_OPEN_SETTINGS, WPARAM(list), LPARAM(0))
                };
            }
            Target::Do(Action::Bluetooth) => {
                // The hub shows the state, also when a digit was pressed.
                self.hot = Some(i);
                self.hub_hot = false;
                self.switch_bluetooth();
            }
            Target::Do(Action::RestartExplorer) if self.running.is_none() => {
                let now = Instant::now();
                let asked = message.as_deref() == Some(maintenance::CONFIRM);
                let last = self.restart_pressed.replace(now).filter(|_| asked);
                if maintenance::confirmed(last, now) {
                    self.launch(Action::RestartExplorer);
                    self.tell(maintenance::WORKING);
                    // Shown until the result comes.
                    let _ = unsafe { KillTimer(Some(self.hwnd), MESSAGE) };
                } else {
                    self.tell(maintenance::CONFIRM);
                }
            }
            // A press during the restart changes nothing.
            Target::Do(Action::RestartExplorer) => self.message = message,
            Target::Do(action @ (Action::BrightnessUp | Action::BrightnessDown)) => {
                // The hub shows the value, also when a digit was pressed.
                self.hot = Some(i);
                self.hub_hot = false;
                let by = match action {
                    Action::BrightnessUp => brightness::STEP,
                    _ => -brightness::STEP,
                };
                self.shine(brightness::Order::Change(by));
                self.draw();
            }
            Target::Do(Action::ScreenOff) => {
                self.close();
                unsafe { SetTimer(Some(self.hwnd), SCREEN, SCREEN_MS, None) };
            }
            Target::Do(Action::VolumeUp) => self.sound(Order::Step(true)),
            Target::Do(Action::VolumeDown) => self.sound(Order::Step(false)),
            Target::Do(Action::Mute) => self.sound(Order::ToggleMute),
            Target::Do(Action::Output(i)) => {
                if let Some(d) = self.devices.get(i).filter(|d| !d.current) {
                    let id = d.id.clone();
                    self.sound(Order::Choose(id));
                }
            }
            Target::Do(action) => self.launch(action),
        }
    }

    /// Starting a program can block, so it happens off this thread.
    fn launch(&mut self, action: Action) {
        if self.running.is_some() {
            return;
        }
        let (tx, rx) = channel();
        self.running = Some(rx);
        let (config, target) = (self.config.clone(), self.hwnd.0 as usize);
        std::thread::spawn(move || {
            let _ = tx.send(actions::run(action, &config));
            let hwnd = HWND(target as *mut _);
            let _ = unsafe { PostMessageW(Some(hwnd), WM_ACTION_DONE, WPARAM(0), LPARAM(0)) };
        });
    }

    fn launched(&mut self) {
        let Some(Ok(result)) = self.running.as_ref().map(Receiver::try_recv) else {
            return;
        };
        self.running = None;
        match result {
            Ok(()) => self.close(),
            Err(e) => self.tell(&e),
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
        let mut next = match self.hot {
            Some(i) => i as isize,
            None if by > 0 => -1,
            None => n,
        };
        for _ in 0..n {
            next = (next + by).rem_euclid(n);
            if !self.is_empty(next as usize) {
                break;
            }
        }
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
            VK_UP if self.page == Page::Sound => self.sound(Order::Change(audio::STEP)),
            VK_DOWN if self.page == Page::Sound => self.sound(Order::Change(-audio::STEP)),
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
        let _ = self.hook.update(self.button(), self.visible);
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

/// `list` is counted from 1; 0 keeps the one shown.
fn open_settings(list: usize) {
    let host = settings::Host {
        get: current_config,
        apply: apply_config,
    };
    if let Err(e) = settings::open(host, settings::LISTS.get(list.wrapping_sub(1)).copied()) {
        warn(&format!("The settings window could not open: {e}"));
    }
}

fn current_config() -> Config {
    let mut c = with_app(|a| a.config.clone()).unwrap_or_default();
    // Windows may have dropped the entry, so the registry decides.
    c.autostart = autostart::enabled().unwrap_or(c.autostart);
    c
}

fn changed(old: &Config, new: &Config) -> Field {
    if old.hotkey != new.hotkey {
        Field::Hotkey
    } else if old.mouse != new.mouse {
        Field::Mouse
    } else if old.theme != new.theme {
        Field::Theme
    } else if old.autostart != new.autostart {
        Field::Autostart
    } else if old.show_names != new.show_names {
        Field::Names
    } else if old.my_apps != new.my_apps {
        Field::MyApps
    } else if old.apps != new.apps {
        Field::Apps
    } else if old.tools != new.tools {
        Field::Tools
    } else {
        Field::Sites
    }
}

/// Saves and applies one change. On an error everything is as it was.
fn apply_config(new: Config) -> Result<(), Problem> {
    let old = current_config();
    if new == old {
        return Ok(());
    }
    let field = changed(&old, &new);
    let problem = |text: String| Problem { field, text };
    keep_unreadable_file().map_err(problem)?;
    let applied = match field {
        Field::Autostart => autostart::set(new.autostart),
        _ => with_app(|a| {
            a.config = new.clone();
            match field {
                Field::Hotkey => a.register_shortcut().map_or(Ok(()), Err),
                Field::Mouse => a.hook.update(a.button(), a.visible),
                _ => Ok(()),
            }
        })
        .unwrap_or_else(|| Err("Madar is busy.".into())),
    };
    let saved = applied.and_then(|()| {
        config::save(&settings_path(), &new).map_err(|e| format!("Could not save: {e}"))
    });
    if let Err(text) = saved {
        if field == Field::Autostart {
            let _ = autostart::set(old.autostart);
        }
        with_app(|a| {
            a.config = old;
            a.apply_triggers();
        });
        return Err(problem(text));
    }
    with_app(|a| {
        a.config = new;
        a.unreadable = false;
        if a.visible {
            a.go(a.page);
        }
    });
    Ok(())
}

/// Asks before the first save over a file that could not be read, and copies it.
fn keep_unreadable_file() -> Result<(), String> {
    let path = settings_path();
    if !with_app(|a| a.unreadable).unwrap_or(false) || !path.exists() {
        return Ok(());
    }
    let answer = unsafe {
        MessageBoxW(
            Some(GetActiveWindow()),
            w!(
                "The settings file could not be read. Replace it with the settings shown here?\n\nA copy of the old file is kept next to it."
            ),
            w!("Madar"),
            MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2,
        )
    };
    if answer != IDYES {
        return Err("Not saved.".into());
    }
    let copy = (1..100)
        .map(|n| path.with_file_name(format!("config.broken-{n}.json")))
        .find(|p| !p.exists())
        .ok_or("Could not copy the old file.")?;
    match std::fs::copy(&path, copy) {
        Ok(_) => Ok(()),
        Err(e) => Err(format!("Could not copy the old file: {e}")),
    }
}

/// The menu runs its own message loop, so the app must not be borrowed here.
fn tray_menu(hwnd: HWND, at: POINT) {
    let Some(paused) = with_app(|a| a.paused) else {
        return;
    };
    match tray::menu(hwnd, paused, at) {
        Some(tray::Command::OpenWheel) => {
            with_app(|a| a.open(false));
        }
        Some(tray::Command::Settings) => open_settings(0),
        Some(tray::Command::TogglePause) => {
            let problems = with_app(App::toggle_pause).unwrap_or_default();
            if !problems.is_empty() {
                warn(&problems.join(
                    "

",
                ));
            }
        }
        Some(tray::Command::Exit) => {
            let _ = unsafe { DestroyWindow(hwnd) };
        }
        None => {}
    }
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
        tray::WM_TRAY => match tray::event(w, l) {
            Some(tray::Event::Open) => with_app(|a| a.open(false)),
            Some(tray::Event::Menu(at)) => {
                tray_menu(hwnd, at);
                Some(())
            }
            None => Some(()),
        },
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
        WM_MOUSEWHEEL => with_app(|a| a.scroll((w.0 >> 16) as i16 as i32)),
        WM_LBUTTONDOWN => with_app(|a| {
            a.pressed = Some(a.hit(client()));
            unsafe { SetCapture(hwnd) };
            a.draw();
        }),
        WM_LBUTTONUP => with_app(|a| a.left_up(client())),
        WM_TIMER => with_app(|a| {
            let running = |s: Instant| s.elapsed().as_secs_f32() * 1000.0 < ANIMATION_MS;
            if w.0 == DEVICES {
                if a.visible && a.page == Page::Sound {
                    a.sound(Order::Read);
                }
                return;
            }
            if w.0 == SCREEN {
                // Posted: Windows acts on it once this borrow has ended.
                unsafe {
                    let _ = KillTimer(Some(hwnd), SCREEN);
                    let off = WPARAM(SC_MONITORPOWER as usize);
                    let _ = PostMessageW(Some(hwnd), WM_SYSCOMMAND, off, LPARAM(POWER_OFF));
                }
                return;
            }
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
        WM_OPEN_SETTINGS => {
            open_settings(w.0);
            Some(())
        }
        WM_ACTION_DONE => with_app(App::launched),
        bluetooth::WM_RADIO => with_app(App::radio_answered),
        brightness::WM_LIGHT => with_app(App::light_answered),
        audio::WM_AUDIO => with_app(App::sound_answered),
        icons::WM_ICON => with_app(App::icon_loaded),
        WM_CLIPBOARDUPDATE => {
            search::copied();
            Some(())
        }
        WM_DISPLAYCHANGE => with_app(App::close),
        WM_DESTROY => {
            settings::close();
            unsafe { PostQuitMessage(0) };
            Some(())
        }
        // Registered messages start at 0xC000.
        0xC000.. => with_app(|a| {
            if msg == a.taskbar_created {
                a.tray.add();
            }
        }),
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

fn start(config: Config, unreadable: bool) -> windows::core::Result<()> {
    let hwnd = create_window()?;
    let mut app = App {
        hwnd,
        config,
        unreadable,
        painter: Painter::new()?,
        icons: icons::Icons::new(hwnd),
        hook: MouseHook::new(hwnd),
        tray: Tray::new(hwnd),
        taskbar_created: tray::taskbar_created(),
        paused: false,
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
        running: None,
        restart_pressed: None,
        audio: None,
        volume: None,
        devices: Vec::new(),
        light: None,
        brightness: None,
        scroll_rest: 0,
        radio: None,
        radio_switching: false,
        radio_state: None,
        radio_lock: None,
    };
    // Without it copied text counts as old. Windows removes it with the window.
    let _ = unsafe { AddClipboardFormatListener(hwnd) };
    // Draws once while hidden, so the first real frame is fast.
    app.go(Page::Root);
    // At sign-in the taskbar may not exist yet; its broadcast adds the icon then.
    app.tray.add();
    APP.with(|a| *a.borrow_mut() = Some(app));
    Ok(())
}

fn main() {
    let mut args = std::env::args().skip(1);
    let first = args.next();
    if first.as_deref() == Some(settings::start_menu::CHILD) {
        // An empty answer tells the parent that the list could not be read.
        let _ = settings::start_menu::child();
        return;
    }
    if first.as_deref() == Some(settings::browse::CHILD) {
        // An empty answer tells the parent that the user cancelled.
        let _ = settings::browse::child(args);
        return;
    }
    if first.as_deref() == Some(audio::output::CHILD) {
        // The parent reads the device back, so the reason does not matter.
        std::process::exit(audio::output::child(args.next()).map_or_else(|e| e.code().0, |()| 0));
    }
    if first.as_deref() == Some(bluetooth::CHILD) {
        bluetooth::child(args.next());
        return;
    }
    if first.as_deref() == Some(icons::CHILD) {
        let side = args.next().and_then(|s| s.parse().ok());
        if let Some(side) = side.filter(|s| (1..=icons::MAX_SIDE).contains(s)) {
            // The parent may be gone; nobody is left to tell.
            let _ = icons::child(side, args);
        }
        return;
    }
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
    if let Err(e) = start(config, !problems.is_empty()) {
        return warn(&format!("Madar could not start: {e}"));
    }
    problems.extend(with_app(App::apply_triggers).unwrap_or_default());
    if !problems.is_empty() {
        warn(&problems.join("\n\n"));
    }
    unsafe {
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            if !settings::translate(&msg) {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
    // Stops the hook thread before the process ends.
    APP.with(|a| a.take());
    maintenance::wait();
}
