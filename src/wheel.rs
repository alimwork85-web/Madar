//! The wheel: pages, slots, hit testing and placement. Drawing is in `paint`.

pub mod paint;

use crate::audio::output::Device;
use crate::config::Config;
use crate::icons::Icon;
use std::rc::Rc;
use windows::Win32::Foundation::{POINT, RECT};

/// Places of every wheel.
const PLACES: usize = 8;
/// Where the output devices start on the Sound page.
pub const FIRST_OUTPUT: usize = 4;

// Logical pixels, from DESIGN.md.
pub const PLATE_R: f32 = 170.0;
pub const OUTER_R: f32 = 166.0;
pub const INNER_R: f32 = 66.0;
pub const HUB_R: f32 = 58.0;
pub const ICON_R: f32 = 117.0;
pub const GAP: f32 = 8.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Page {
    Root,
    System,
    MyApps,
    Apps,
    Tools,
    Sites,
    Sound,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    Run,
    TaskManager,
    RestartExplorer,
    CleanDns,
    Settings,
    MyApp(usize),
    App(usize),
    Tool(usize),
    Site(usize),
    /// The first free place of a folder.
    AddItems,
    VolumeUp,
    VolumeDown,
    Mute,
    /// An output device.
    Output(usize),
    SoundSettings,
    Bluetooth,
    BluetoothSettings,
    Google,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    Open(Page),
    Do(Action),
    Disabled,
    /// A free place: keeps the wheel whole and does nothing.
    Empty,
}

#[derive(Clone, Debug)]
pub struct Slot {
    pub label: String,
    /// Segoe Fluent Icons code point.
    pub icon: char,
    /// Drawn instead of `icon` once it is loaded.
    pub image: Option<Rc<Icon>>,
    pub target: Target,
}

fn slot(label: &str, icon: char, target: Target) -> Slot {
    Slot {
        label: label.to_string(),
        icon,
        image: None,
        target,
    }
}

pub fn slots(page: Page, c: &Config) -> Vec<Slot> {
    use Action::*;
    use Target::*;
    let fill = |mut items: Vec<Slot>| {
        while items.len() < PLACES {
            items.push(slot("", ' ', Empty));
        }
        items
    };
    let folder = |mut items: Vec<Slot>, add: &str| {
        if items.len() < PLACES {
            items.push(slot(add, '\u{E710}', Do(AddItems)));
        }
        fill(items)
    };
    match page {
        Page::Root => vec![
            slot("My Apps", '\u{E734}', Open(Page::MyApps)),
            slot("Apps", '\u{E71D}', Open(Page::Apps)),
            slot("Sites", '\u{E774}', Open(Page::Sites)),
            slot("Google", '\u{E721}', Do(Google)),
            slot("System", '\u{E770}', Open(Page::System)),
            slot("Settings", '\u{E713}', Do(Settings)),
            slot("Sound", '\u{E767}', Open(Page::Sound)),
            slot("Tools", '\u{EC7A}', Open(Page::Tools)),
        ],
        Page::System => fill(vec![
            slot("Run", '\u{E756}', Do(Run)),
            slot("Task Manager", '\u{E9D9}', Do(TaskManager)),
            slot("Restart Explorer", '\u{E72C}', Do(RestartExplorer)),
            slot("Clean DNS Cache", '\u{EA99}', Do(CleanDns)),
        ]),
        Page::MyApps => folder(
            (c.my_apps.iter().enumerate())
                .map(|(i, a)| slot(&a.label, '\u{ECAA}', Do(MyApp(i))))
                .collect(),
            "Add app",
        ),
        Page::Apps => folder(
            (c.apps.iter().enumerate())
                .map(|(i, a)| slot(&a.label, '\u{ECAA}', Do(App(i))))
                .collect(),
            "Add app",
        ),
        Page::Tools => folder(
            (c.tools.iter().enumerate())
                .map(|(i, a)| slot(&a.label, '\u{ECAA}', Do(Tool(i))))
                .collect(),
            "Add app",
        ),
        Page::Sites => folder(
            (c.sites.iter().enumerate())
                .map(|(i, s)| slot(&s.label, '\u{E774}', Do(Site(i))))
                .collect(),
            "Add site",
        ),
        Page::Sound => fill(vec![
            slot("Volume up", '\u{E995}', Do(VolumeUp)),
            slot("Mute", '\u{E74F}', Do(Mute)),
            slot("Volume down", '\u{E993}', Do(VolumeDown)),
            slot("Bluetooth", '\u{E702}', Do(Bluetooth)),
        ]),
    }
}

/// The Sound page's places after the volume. Windows' own page shows the rest.
pub fn outputs(devices: &[Device]) -> Vec<Slot> {
    let places = PLACES - FIRST_OUTPUT;
    let mut shown: Vec<usize> = (0..devices.len()).collect();
    let more = devices.len() > places;
    if more {
        shown.truncate(places - 1);
        // The one in use is never hidden.
        if let Some(i) = devices.iter().position(|d| d.current)
            && !shown.contains(&i)
        {
            shown[places - 2] = i;
        }
    }
    let mut all: Vec<Slot> = (shown.into_iter())
        .map(|i| {
            let icon = if devices[i].current {
                '\u{E73E}'
            } else {
                '\u{E7F5}'
            };
            slot(&devices[i].name, icon, Target::Do(Action::Output(i)))
        })
        .collect();
    if more {
        all.push(slot("More", '\u{E712}', Target::Do(Action::SoundSettings)));
    }
    while all.len() < places {
        all.push(slot("", ' ', Target::Empty));
    }
    all
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hit {
    Hub,
    Slot(usize),
    Outside,
}

/// Slot under an offset from the center, by angle only.
pub fn slot_at_angle(dx: f32, dy: f32, n: usize) -> usize {
    let step = std::f32::consts::TAU / n as f32;
    let from_top = dx.atan2(-dy);
    (from_top / step).round().rem_euclid(n as f32) as usize
}

/// What lies under an offset from the center, in logical pixels.
pub fn hit(dx: f32, dy: f32, n: usize) -> Hit {
    match dx.hypot(dy) {
        r if r < (HUB_R + INNER_R) / 2.0 => Hit::Hub,
        r if r <= PLATE_R => Hit::Slot(slot_at_angle(dx, dy, n)),
        _ => Hit::Outside,
    }
}

/// Center closest to `p` that keeps a wheel of radius `r` inside `area`.
pub fn place(p: POINT, r: i32, area: RECT) -> POINT {
    let fit = |v: i32, lo: i32, hi: i32| {
        if hi - lo < 2 * r {
            (lo + hi) / 2
        } else {
            v.clamp(lo + r, hi - r)
        }
    };
    POINT {
        x: fit(p.x, area.left, area.right),
        y: fit(p.y, area.top, area.bottom),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_slot_is_at_twelve_and_order_is_clockwise() {
        for n in 1..=8 {
            for i in 0..n {
                let a = std::f32::consts::TAU * i as f32 / n as f32;
                let (dx, dy) = (ICON_R * a.sin(), -ICON_R * a.cos());
                assert_eq!(hit(dx, dy, n), Hit::Slot(i), "{i} of {n}");
            }
        }
    }

    #[test]
    fn slot_borders() {
        // Seven slots: the first one ends 25.71 degrees from the top.
        let at = |deg: f32| slot_at_angle(deg.to_radians().sin(), -deg.to_radians().cos(), 7);
        assert_eq!(at(25.0), 0);
        assert_eq!(at(26.5), 1);
        assert_eq!(at(-25.0), 0);
        assert_eq!(at(-26.5), 6);
        assert_eq!(at(180.5), 4);
        assert_eq!(at(179.5), 3);
    }

    #[test]
    fn radii() {
        assert_eq!(hit(0.0, 0.0, 7), Hit::Hub);
        assert_eq!(hit(0.0, -61.0, 7), Hit::Hub);
        assert_eq!(hit(0.0, -63.0, 7), Hit::Slot(0));
        assert_eq!(hit(0.0, -170.5, 7), Hit::Outside);
    }

    #[test]
    fn placement_stays_on_screen() {
        let area = RECT {
            left: -1920,
            top: 0,
            right: 0,
            bottom: 1040,
        };
        let at = |x, y| {
            let c = place(POINT { x, y }, 170, area);
            (c.x, c.y)
        };
        assert_eq!(at(-900, 500), (-900, 500));
        assert_eq!(at(-1920, 0), (-1750, 170));
        assert_eq!(at(-1, 1079), (-170, 870));
        let small = RECT {
            left: 0,
            top: 0,
            right: 200,
            bottom: 1000,
        };
        let c = place(POINT { x: 5, y: 5 }, 170, small);
        assert_eq!((c.x, c.y), (100, 170));
    }

    #[test]
    fn output_devices() {
        let some = |n: usize, current: usize| -> Vec<Device> {
            (0..n)
                .map(|i| Device {
                    id: i.to_string(),
                    name: format!("Device {i}"),
                    current: i == current,
                })
                .collect()
        };
        let shown = |n, current| -> Vec<Target> {
            outputs(&some(n, current))
                .iter()
                .map(|s| s.target)
                .collect()
        };
        let out = |i| Target::Do(Action::Output(i));
        let more = Target::Do(Action::SoundSettings);
        assert_eq!(shown(0, 0), [Target::Empty; 4]);
        assert_eq!(shown(2, 0)[..3], [out(0), out(1), Target::Empty]);
        assert_eq!(shown(4, 3), [out(0), out(1), out(2), out(3)]);
        assert_eq!(shown(5, 1), [out(0), out(1), out(2), more]);
        assert_eq!(shown(9, 7), [out(0), out(1), out(7), more]);
        let icons: Vec<char> = outputs(&some(2, 1)).iter().map(|s| s.icon).collect();
        assert_eq!(icons[..2], ['\u{E7F5}', '\u{E73E}']);
    }

    #[test]
    fn pages() {
        let mut c = Config::default();
        assert_eq!(slots(Page::Root, &c).len(), 8);
        let system = slots(Page::System, &c);
        assert_eq!(system.len(), 8);
        assert_eq!(system[2].target, Target::Do(Action::RestartExplorer));
        assert_eq!(system[3].target, Target::Do(Action::CleanDns));
        assert_eq!(slots(Page::Sound, &c).len(), 8);
        assert_eq!(
            slots(Page::Apps, &c)[0].target,
            Target::Do(Action::AddItems)
        );
        c.my_apps.push(crate::config::App {
            label: "مدار".into(),
            path: "C:\\madar.exe".into(),
            args: String::new(),
        });
        let mine = slots(Page::MyApps, &c);
        assert_eq!(mine[0].target, Target::Do(Action::MyApp(0)));
        assert_eq!(
            slots(Page::Apps, &c)[0].target,
            Target::Do(Action::AddItems)
        );
        let sound = slots(Page::Sound, &c);
        assert_eq!(sound[3].target, Target::Do(Action::Bluetooth));
        let root = slots(Page::Root, &c);
        assert_eq!(root[3].target, Target::Do(Action::Google));
        assert_eq!(root[7].target, Target::Open(Page::Tools));
        c.tools = vec![c.my_apps[0].clone(); 8];
        let tools = slots(Page::Tools, &c);
        assert_eq!(tools.len(), 8);
        assert_eq!(tools[7].target, Target::Do(Action::Tool(7)));
        c.sites.push(crate::config::Site {
            label: "قوقل".into(),
            url: "https://google.com".into(),
            browser: None,
        });
        let sites = slots(Page::Sites, &c);
        let targets: Vec<_> = sites.iter().map(|s| s.target).collect();
        let mut expected = [Target::Empty; 8];
        expected[0] = Target::Do(Action::Site(0));
        expected[1] = Target::Do(Action::AddItems);
        assert_eq!(targets, expected);
        for (items, can_add) in [(6, true), (7, true), (8, false)] {
            c.sites = vec![c.sites[0].clone(); items];
            let all = slots(Page::Sites, &c);
            assert_eq!(all.len(), 8);
            let add = Target::Do(Action::AddItems);
            assert_eq!(all.iter().any(|s| s.target == add), can_add);
            let empty = all.iter().filter(|s| s.target == Target::Empty).count();
            assert_eq!(empty, 8 - items - can_add as usize);
        }
    }
}
