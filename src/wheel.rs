//! The wheel: pages, slots, hit testing and placement. Drawing is in `paint`.

pub mod paint;

use crate::config::Config;
use windows::Win32::Foundation::{POINT, RECT};

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
    Apps,
    Sites,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    Run,
    TaskManager,
    Settings,
    App(usize),
    Site(usize),
    /// The slot of an empty folder.
    AddItems,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    Open(Page),
    Do(Action),
    Disabled,
}

#[derive(Clone, Debug)]
pub struct Slot {
    pub label: String,
    /// Segoe Fluent Icons code point.
    pub icon: char,
    pub target: Target,
}

fn slot(label: &str, icon: char, target: Target) -> Slot {
    Slot {
        label: label.to_string(),
        icon,
        target,
    }
}

pub fn slots(page: Page, c: &Config) -> Vec<Slot> {
    use Action::*;
    use Target::*;
    let add = || vec![slot("Add in Settings", '\u{E710}', Do(AddItems))];
    match page {
        Page::Root => vec![
            slot("System", '\u{E770}', Open(Page::System)),
            slot("Settings", '\u{E713}', Do(Settings)),
            slot("Sound", '\u{E767}', Disabled),
            slot("Bluetooth", '\u{E702}', Disabled),
            slot("Apps", '\u{E71D}', Open(Page::Apps)),
            slot("Sites", '\u{E774}', Open(Page::Sites)),
            slot("Google", '\u{E721}', Disabled),
        ],
        Page::System => vec![
            slot("Run", '\u{E756}', Do(Run)),
            slot("Task Manager", '\u{E9D9}', Do(TaskManager)),
            slot("Maintenance", '\u{E90F}', Disabled),
        ],
        Page::Apps if c.apps.is_empty() => add(),
        Page::Apps => (c.apps.iter().enumerate())
            .map(|(i, a)| slot(&a.label, '\u{ECAA}', Do(App(i))))
            .collect(),
        Page::Sites if c.sites.is_empty() => add(),
        Page::Sites => (c.sites.iter().enumerate())
            .map(|(i, s)| slot(&s.label, '\u{E774}', Do(Site(i))))
            .collect(),
    }
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
    fn pages() {
        let mut c = Config::default();
        assert_eq!(slots(Page::Root, &c).len(), 7);
        assert_eq!(
            slots(Page::Apps, &c)[0].target,
            Target::Do(Action::AddItems)
        );
        c.sites.push(crate::config::Site {
            label: "قوقل".into(),
            url: "https://google.com".into(),
            browser: None,
        });
        let sites = slots(Page::Sites, &c);
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].target, Target::Do(Action::Site(0)));
    }
}
