//! Settings file (`config.json`): parse, validate, load and save.
//!
//! JSON shape: a single object with `hotkey`, `mouse`, `theme`, `size`, `autostart`,
//! `show_names`, `my_apps`, `apps`, `tools` and `sites`. Unknown keys are ignored; missing keys take the default.

use serde_json::{Map, Value};
use std::io::{Read, Write};
use std::path::Path;

/// Theme names from DESIGN.md.
pub const THEMES: [&str; 8] = [
    "Space Gray",
    "Midnight",
    "Ember",
    "Grape",
    "Solar",
    "Meadow",
    "Aqua",
    "Porcelain",
];

const DEFAULT_THEME: &str = "Midnight";
const DEFAULT_HOTKEY: &str = "Ctrl+Space";
const MAX_ITEMS: usize = 8;
const MAX_FILE_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub struct App {
    pub label: String,
    pub path: String,
    pub args: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Site {
    pub label: String,
    pub url: String,
    /// Browser exe path; `None` means the system default browser.
    pub browser: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mouse {
    None,
    Back,
    Forward,
    CtrlMiddle,
}

/// How large the wheel is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Size {
    Large,
    Medium,
    Small,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub hotkey: String,
    pub mouse: Mouse,
    pub theme: String,
    pub size: Size,
    pub autostart: bool,
    /// False: the wheel shows icons only.
    pub show_names: bool,
    pub my_apps: Vec<App>,
    pub apps: Vec<App>,
    pub tools: Vec<App>,
    pub sites: Vec<Site>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hotkey: DEFAULT_HOTKEY.to_string(),
            mouse: Mouse::Back,
            theme: DEFAULT_THEME.to_string(),
            size: Size::Large,
            autostart: false,
            show_names: true,
            my_apps: Vec::new(),
            apps: Vec::new(),
            tools: Vec::new(),
            sites: Vec::new(),
        }
    }
}

impl Mouse {
    fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Back => "back",
            Self::Forward => "forward",
            Self::CtrlMiddle => "ctrl_middle",
        }
    }
}

impl Size {
    /// In the order the Settings window shows them.
    pub const ALL: [Self; 3] = [Self::Large, Self::Medium, Self::Small];

    fn as_str(self) -> &'static str {
        match self {
            Self::Large => "large",
            Self::Medium => "medium",
            Self::Small => "small",
        }
    }

    pub fn factor(self) -> f32 {
        match self {
            Self::Large => 1.0,
            Self::Medium => 0.85,
            Self::Small => 0.75,
        }
    }
}

/// True for an `http`/`https` URL with a non-empty host and no whitespace or
/// control characters. Scheme case is ignored.
pub fn is_web_url(url: &str) -> bool {
    // A quote could break out of the argument when a chosen browser is started.
    if url
        .chars()
        .any(|c| c.is_whitespace() || c.is_control() || c == '"')
    {
        return false;
    }
    // Lowercasing only touches ASCII, so byte offsets and char boundaries hold.
    let lower = url.to_ascii_lowercase();
    let rest = if let Some(r) = lower.strip_prefix("https://") {
        r
    } else if let Some(r) = lower.strip_prefix("http://") {
        r
    } else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    !host.is_empty() && !host.starts_with(':')
}

pub fn parse(text: &str) -> Result<Config, String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let root: Value = serde_json::from_str(text).map_err(|e| format!("invalid JSON: {e}"))?;
    let obj = root
        .as_object()
        .ok_or_else(|| "JSON root must be an object".to_string())?;

    let mut c = Config::default();

    if let Some(v) = obj.get("hotkey") {
        c.hotkey = as_str(v, "hotkey")?.to_string();
    }
    if let Some(v) = obj.get("mouse") {
        c.mouse = parse_mouse(v)?;
    }
    if let Some(v) = obj.get("theme") {
        let name = as_str(v, "theme")?;
        c.theme = if THEMES.contains(&name) {
            name.to_string()
        } else {
            DEFAULT_THEME.to_string()
        };
    }
    if let Some(v) = obj.get("size") {
        let name = as_str(v, "size")?;
        // Like an unknown theme, an unknown size is the default one.
        let known = Size::ALL.into_iter().find(|s| s.as_str() == name);
        c.size = known.unwrap_or(Size::Large);
    }
    if let Some(v) = obj.get("autostart") {
        c.autostart = v
            .as_bool()
            .ok_or_else(|| "autostart must be a boolean".to_string())?;
    }
    if let Some(v) = obj.get("show_names") {
        c.show_names = v
            .as_bool()
            .ok_or_else(|| "show_names must be a boolean".to_string())?;
    }
    if let Some(v) = obj.get("my_apps") {
        c.my_apps = parse_apps(v, "my_apps")?;
    }
    if let Some(v) = obj.get("apps") {
        c.apps = parse_apps(v, "apps")?;
    }
    if let Some(v) = obj.get("tools") {
        c.tools = parse_apps(v, "tools")?;
    }
    if let Some(v) = obj.get("sites") {
        c.sites = parse_sites(v)?;
    }

    Ok(c)
}

fn parse_mouse(v: &Value) -> Result<Mouse, String> {
    match v.as_str() {
        Some("none") => Ok(Mouse::None),
        Some("back") => Ok(Mouse::Back),
        Some("forward") => Ok(Mouse::Forward),
        Some("ctrl_middle") => Ok(Mouse::CtrlMiddle),
        Some(other) => Err(format!("unknown mouse value: \"{other}\"")),
        None => Err("mouse must be a string".to_string()),
    }
}

fn parse_apps(v: &Value, field: &str) -> Result<Vec<App>, String> {
    let arr = v
        .as_array()
        .ok_or_else(|| format!("{field} must be an array"))?;
    if arr.len() > MAX_ITEMS {
        return Err(format!("{field}: at most {MAX_ITEMS} are allowed"));
    }
    let mut out = Vec::with_capacity(arr.len());
    for (i, item) in arr.iter().enumerate() {
        let o = item
            .as_object()
            .ok_or_else(|| format!("{field}[{i}] must be an object"))?;
        let label = match o.get("label") {
            Some(v) => as_str(v, &format!("{field}[{i}].label"))?.to_string(),
            None => String::new(),
        };
        let path = match o.get("path") {
            Some(v) => as_str(v, &format!("{field}[{i}].path"))?.to_string(),
            None => String::new(),
        };
        let args = match o.get("args") {
            Some(v) => as_str(v, &format!("{field}[{i}].args"))?.to_string(),
            None => String::new(),
        };
        if label.is_empty() {
            return Err(format!("{field}[{i}].label is empty"));
        }
        if path.is_empty() {
            return Err(format!("{field}[{i}].path is empty"));
        }
        out.push(App { label, path, args });
    }
    Ok(out)
}

fn parse_sites(v: &Value) -> Result<Vec<Site>, String> {
    let arr = v
        .as_array()
        .ok_or_else(|| "sites must be an array".to_string())?;
    if arr.len() > MAX_ITEMS {
        return Err(format!("sites: at most {MAX_ITEMS} are allowed"));
    }
    let mut out = Vec::with_capacity(arr.len());
    for (i, item) in arr.iter().enumerate() {
        let o = item
            .as_object()
            .ok_or_else(|| format!("sites[{i}] must be an object"))?;
        let label = match o.get("label") {
            Some(v) => as_str(v, &format!("sites[{i}].label"))?.to_string(),
            None => String::new(),
        };
        let url = match o.get("url") {
            Some(v) => as_str(v, &format!("sites[{i}].url"))?.to_string(),
            None => String::new(),
        };
        let browser = match o.get("browser") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(s.clone()),
            Some(_) => return Err(format!("sites[{i}].browser must be a string or null")),
        };
        if label.is_empty() {
            return Err(format!("sites[{i}].label is empty"));
        }
        if !is_web_url(&url) {
            return Err(format!("site \"{label}\": invalid URL \"{url}\""));
        }
        out.push(Site {
            label,
            url,
            browser,
        });
    }
    Ok(out)
}

fn as_str<'a>(v: &'a Value, field: &str) -> Result<&'a str, String> {
    v.as_str()
        .ok_or_else(|| format!("{field} must be a string"))
}

fn to_value(c: &Config) -> Value {
    let apps = |apps: &[App]| -> Vec<Value> {
        let item = |a: &App| {
            let mut m = Map::new();
            m.insert("label".into(), Value::String(a.label.clone()));
            m.insert("path".into(), Value::String(a.path.clone()));
            m.insert("args".into(), Value::String(a.args.clone()));
            Value::Object(m)
        };
        apps.iter().map(item).collect()
    };
    let sites = c
        .sites
        .iter()
        .map(|s| {
            let mut m = Map::new();
            m.insert("label".into(), Value::String(s.label.clone()));
            m.insert("url".into(), Value::String(s.url.clone()));
            m.insert(
                "browser".into(),
                match &s.browser {
                    Some(b) => Value::String(b.clone()),
                    None => Value::Null,
                },
            );
            Value::Object(m)
        })
        .collect();

    let mut root = Map::new();
    root.insert("hotkey".into(), Value::String(c.hotkey.clone()));
    root.insert("mouse".into(), Value::String(c.mouse.as_str().into()));
    root.insert("theme".into(), Value::String(c.theme.clone()));
    root.insert("size".into(), Value::String(c.size.as_str().into()));
    root.insert("autostart".into(), Value::Bool(c.autostart));
    root.insert("show_names".into(), Value::Bool(c.show_names));
    root.insert("my_apps".into(), Value::Array(apps(&c.my_apps)));
    root.insert("apps".into(), Value::Array(apps(&c.apps)));
    root.insert("tools".into(), Value::Array(apps(&c.tools)));
    root.insert("sites".into(), Value::Array(sites));
    Value::Object(root)
}

fn render(v: &Value) -> Result<String, String> {
    serde_json::to_string_pretty(v).map_err(|e| format!("cannot serialize settings: {e}"))
}

pub fn load(path: &Path) -> Result<Config, String> {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
        Err(e) => return Err(format!("cannot open {}: {e}", path.display())),
    };
    // Read one byte past the limit so an oversized file is rejected without
    // slurping all of it.
    let mut buf = Vec::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut buf)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    if buf.len() as u64 > MAX_FILE_BYTES {
        return Err(format!(
            "{} is larger than {MAX_FILE_BYTES} bytes",
            path.display()
        ));
    }
    let text =
        String::from_utf8(buf).map_err(|_| format!("{} is not valid UTF-8", path.display()))?;
    parse(&text)
}

fn write_tmp(tmp: &Path, data: &str) -> std::io::Result<()> {
    let mut f = std::fs::File::create(tmp)?;
    f.write_all(data.as_bytes())?;
    f.flush()?;
    f.sync_all()
}

pub fn save(path: &Path, c: &Config) -> std::io::Result<()> {
    let data = render(&to_value(c)).map_err(std::io::Error::other)?;
    // Never write a file that load() would reject.
    if data.len() as u64 > MAX_FILE_BYTES {
        return Err(std::io::Error::other("settings are too large"));
    }
    parse(&data).map_err(std::io::Error::other)?;
    let dir = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => std::path::PathBuf::from("."),
    };
    std::fs::create_dir_all(&dir)?;

    let name = path.file_name().map_or_else(
        || "config".to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));

    // Temp file first, then rename: the target is never missing or partial.
    if let Err(e) = write_tmp(&tmp, &data) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    /// Unique folder per test, under the OS temp dir.
    fn scratch(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!(
            "madar-config-{name}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn tmp_leftovers(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        v.sort();
        v
    }

    fn full_config() -> Config {
        Config {
            hotkey: "Ctrl+Alt+M".to_string(),
            mouse: Mouse::CtrlMiddle,
            theme: "Ember".to_string(),
            size: Size::Small,
            autostart: true,
            show_names: false,
            my_apps: vec![App {
                label: "مدار".to_string(),
                path: r"C:\Madar\madar.exe".to_string(),
                args: String::new(),
            }],
            apps: vec![
                App {
                    label: "تحميل \"المستندات\"".to_string(),
                    path: r"C:\Program Files\App\app.exe".to_string(),
                    args: "--open \"a b\"".to_string(),
                },
                App {
                    label: "Settings".to_string(),
                    path: "C:/Windows/notepad.exe".to_string(),
                    args: String::new(),
                },
            ],
            tools: vec![App {
                label: "أداة".to_string(),
                path: r"shell:AppsFolder\Some.Tool_x!App".to_string(),
                args: String::new(),
            }],
            sites: vec![
                Site {
                    label: "بحث".to_string(),
                    url: "https://example.com/a?b=1".to_string(),
                    browser: Some(r"C:\Program Files\Firefox\firefox.exe".to_string()),
                },
                Site {
                    label: "Mail".to_string(),
                    url: "HTTP://MAIL.EXAMPLE.COM".to_string(),
                    browser: None,
                },
            ],
        }
    }

    #[test]
    fn default_matches_spec() {
        let c = Config::default();
        assert_eq!(c.hotkey, "Ctrl+Space");
        assert_eq!(c.mouse, Mouse::Back);
        assert_eq!(c.theme, "Midnight");
        assert!(!c.autostart);
        assert!(c.my_apps.is_empty() && c.apps.is_empty());
        assert!(c.tools.is_empty() && c.sites.is_empty());
        assert!(c.show_names);
        assert_eq!(THEMES.len(), 8);
        assert_eq!(THEMES[1], "Midnight");
    }

    #[test]
    fn round_trip_save_load() {
        let dir = scratch("round-trip");
        let path = dir.join("config.json");
        let c = full_config();
        save(&path, &c).unwrap();
        assert_eq!(load(&path).unwrap(), c);
        assert!(tmp_leftovers(&dir).is_empty());

        let via_text = parse(&render(&to_value(&c)).unwrap()).unwrap();
        assert_eq!(via_text, c);
        assert!(
            render(&to_value(&c)).unwrap().contains("\n  \""),
            "output must be pretty"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn empty_object_gives_defaults() {
        assert_eq!(parse("{}").unwrap(), Config::default());
        assert_eq!(parse("  {}  \n").unwrap(), Config::default());
    }

    #[test]
    fn missing_fields_use_defaults() {
        let c = parse(r#"{"theme":"Aqua"}"#).unwrap();
        assert_eq!(c.theme, "Aqua");
        assert_eq!(c.hotkey, "Ctrl+Space");
        assert_eq!(c.mouse, Mouse::Back);
        assert!(!c.autostart);
    }

    #[test]
    fn invalid_json_is_err() {
        assert!(parse("{").is_err());
        assert!(parse("").is_err());
        assert!(parse("null").is_err());
        assert!(parse("[]").is_err());
        assert!(parse("\"text\"").is_err());
        assert!(parse("42").is_err());
    }

    #[test]
    fn wrong_type_names_the_field() {
        let e = parse(r#"{"hotkey":5}"#).unwrap_err();
        assert!(e.contains("hotkey"), "{e}");
        let e = parse(r#"{"autostart":"yes"}"#).unwrap_err();
        assert!(e.contains("autostart"), "{e}");
        let e = parse(r#"{"mouse":true}"#).unwrap_err();
        assert!(e.contains("mouse"), "{e}");
        let e = parse(r#"{"apps":{}}"#).unwrap_err();
        assert!(e.contains("apps"), "{e}");
        let e = parse(r#"{"sites":[{"label":"a","url":5}]}"#).unwrap_err();
        assert!(e.contains("sites[0].url"), "{e}");
        let e = parse(r#"{"apps":[{"label":"a","path":1}]}"#).unwrap_err();
        assert!(e.contains("apps[0].path"), "{e}");
    }

    #[test]
    fn unknown_theme_falls_back_to_midnight() {
        assert_eq!(parse(r#"{"theme":"Neon"}"#).unwrap().theme, "Midnight");
        assert_eq!(parse(r#"{"theme":""}"#).unwrap().theme, "Midnight");
        assert_eq!(
            parse(r#"{"theme":"Space Gray"}"#).unwrap().theme,
            "Space Gray"
        );
    }

    #[test]
    fn size_values() {
        assert_eq!(parse("{}").unwrap().size, Size::Large);
        for (text, size, factor) in [
            ("large", Size::Large, 1.0),
            ("medium", Size::Medium, 0.85),
            ("small", Size::Small, 0.75),
            ("huge", Size::Large, 1.0),
            ("", Size::Large, 1.0),
        ] {
            let c = parse(&format!(r#"{{"size":"{text}"}}"#)).unwrap();
            assert_eq!((c.size, c.size.factor()), (size, factor), "{text}");
            assert_eq!(parse(&render(&to_value(&c)).unwrap()).unwrap().size, size);
        }
        let e = parse(r#"{"size":2}"#).unwrap_err();
        assert!(e.contains("size"), "{e}");
    }

    #[test]
    fn mouse_values() {
        assert_eq!(parse(r#"{"mouse":"none"}"#).unwrap().mouse, Mouse::None);
        assert_eq!(parse(r#"{"mouse":"back"}"#).unwrap().mouse, Mouse::Back);
        assert_eq!(
            parse(r#"{"mouse":"forward"}"#).unwrap().mouse,
            Mouse::Forward
        );
        assert_eq!(
            parse(r#"{"mouse":"ctrl_middle"}"#).unwrap().mouse,
            Mouse::CtrlMiddle
        );
        let e = parse(r#"{"mouse":"CtrlMiddle"}"#).unwrap_err();
        assert!(e.contains("mouse"), "{e}");
        assert_eq!(Mouse::Forward.as_str(), "forward");
    }

    #[test]
    fn web_url_rules() {
        assert!(is_web_url("http://example.com"));
        assert!(is_web_url("https://example.com/path?q=1#f"));
        assert!(is_web_url("HTTPS://EXAMPLE.COM"));
        assert!(is_web_url("http://localhost:8080"));
        assert!(!is_web_url(""));
        assert!(!is_web_url("example.com"));
        assert!(!is_web_url("ftp://example.com"));
        assert!(!is_web_url("http://"));
        assert!(!is_web_url("https:///path"));
        assert!(!is_web_url("http://:80/x"));
        assert!(!is_web_url("http://exa mple.com"));
        assert!(!is_web_url("http://example.com/\ta"));
        assert!(!is_web_url("http://example.com/\u{0}"));
        assert!(!is_web_url("https://a.b/\"--flag"));
        assert!(!is_web_url("تحميل"));
        assert!(is_web_url("https://مثال.com"));
    }

    #[test]
    fn bad_site_url_names_label() {
        let e = parse(r#"{"sites":[{"label":"Quran","url":"example.com"}]}"#).unwrap_err();
        assert!(e.contains("Quran"), "{e}");
        let e = parse(r#"{"sites":[{"label":"Quran"}]}"#).unwrap_err();
        assert!(e.contains("Quran"), "{e}");
        let e = parse(r#"{"sites":[{"label":"X","url":"https://a.b","browser":7}]}"#).unwrap_err();
        assert!(e.contains("sites[0].browser"), "{e}");
    }

    #[test]
    fn empty_label_or_app_path_is_err() {
        assert!(parse(r#"{"apps":[{"label":"a","path":""}]}"#).is_err());
        assert!(parse(r#"{"apps":[{"label":"","path":"C:/a.exe"}]}"#).is_err());
        assert!(parse(r#"{"apps":[{"label":"a"}]}"#).is_err());
        assert!(parse(r#"{"sites":[{"label":"","url":"https://a.b"}]}"#).is_err());
        assert!(parse(r#"{"apps":[7]}"#).is_err());
        assert!(
            parse(r#"{"my_apps":[7]}"#)
                .unwrap_err()
                .contains("my_apps[0]")
        );
        assert!(parse(r#"{"show_names":1}"#).is_err());
        assert!(parse(r#"{"sites":[7]}"#).is_err());
    }

    #[test]
    fn too_many_items_is_err() {
        let app = |i: usize| format!(r#"{{"label":"a{i}","path":"C:/a{i}.exe"}}"#);
        let site = |i: usize| format!(r#"{{"label":"s{i}","url":"https://s{i}.example"}}"#);
        let eight_apps: Vec<String> = (0..8).map(app).collect();
        let nine_apps: Vec<String> = (0..9).map(app).collect();
        let eight_sites: Vec<String> = (0..8).map(site).collect();
        let nine_sites: Vec<String> = (0..9).map(site).collect();
        assert!(parse(&format!(r#"{{"apps":[{}]}}"#, eight_apps.join(","))).is_ok());
        let e = parse(&format!(r#"{{"apps":[{}]}}"#, nine_apps.join(","))).unwrap_err();
        assert!(e.contains("apps"), "{e}");
        assert!(parse(&format!(r#"{{"sites":[{}]}}"#, eight_sites.join(","))).is_ok());
        let e = parse(&format!(r#"{{"sites":[{}]}}"#, nine_sites.join(","))).unwrap_err();
        assert!(e.contains("sites"), "{e}");
    }

    #[test]
    fn unknown_fields_ignored_and_bom_accepted() {
        let with_bom = "\u{feff}{\"theme\":\"Grape\",\"future\":{\"x\":1}}";
        assert_eq!(parse(with_bom).unwrap().theme, "Grape");
        assert_eq!(parse("\u{feff}{}").unwrap(), Config::default());
    }

    #[test]
    fn load_missing_file_is_default() {
        let dir = scratch("missing");
        assert_eq!(load(&dir.join("nope.json")).unwrap(), Config::default());
        assert_eq!(
            load(&dir.join("no").join("such").join("dir.json")).unwrap(),
            Config::default()
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn corrupt_file_is_err_and_untouched() {
        let dir = scratch("corrupt");
        let path = dir.join("config.json");
        let junk = b"{ \"theme\": \"Ember\", oops";
        fs::write(&path, junk).unwrap();
        assert!(load(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), junk);
        assert!(tmp_leftovers(&dir).is_empty());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn oversized_file_is_err() {
        let dir = scratch("oversize");
        let path = dir.join("config.json");
        fs::write(&path, vec![b' '; MAX_FILE_BYTES as usize + 1]).unwrap();
        let e = load(&path).unwrap_err();
        assert!(e.contains("larger"), "{e}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn save_creates_parent_and_replaces() {
        let dir = scratch("replace");
        let path = dir.join("nested").join("deeper").join("config.json");
        let first = Config::default();
        save(&path, &first).unwrap();
        assert_eq!(load(&path).unwrap(), first);

        let mut second = full_config();
        second.theme = "Meadow".to_string();
        save(&path, &second).unwrap();
        assert_eq!(load(&path).unwrap(), second);
        let parent = path.parent().unwrap();
        assert!(tmp_leftovers(parent).is_empty());
        assert_eq!(fs::read_dir(parent).unwrap().count(), 1);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn failed_save_leaves_target_and_removes_temp() {
        let dir = scratch("fail");
        let good = dir.join("config.json");
        save(&good, &Config::default()).unwrap();
        let before = fs::read(&good).unwrap();

        // Rename onto a directory fails: the target survives, the temp is gone.
        let blocked = dir.join("blocked");
        fs::create_dir_all(blocked.join("inner")).unwrap();
        assert!(save(&blocked, &full_config()).is_err());
        assert_eq!(fs::read(&good).unwrap(), before);
        assert!(tmp_leftovers(&dir).is_empty());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn save_refuses_what_load_would_reject() {
        let dir = scratch("refuse");
        let path = dir.join("config.json");
        save(&path, &Config::default()).unwrap();
        let before = fs::read(&path).unwrap();

        let mut bad = full_config();
        bad.sites[0].url = "file:///C:/x".to_string();
        assert!(save(&path, &bad).is_err());
        let mut many = full_config();
        many.apps = vec![many.apps[0].clone(); MAX_ITEMS + 1];
        assert!(save(&path, &many).is_err());
        let mut huge = full_config();
        huge.apps[0].args = "x".repeat(MAX_FILE_BYTES as usize);
        assert!(save(&path, &huge).is_err());

        assert_eq!(fs::read(&path).unwrap(), before);
        assert!(tmp_leftovers(&dir).is_empty());
        fs::remove_dir_all(&dir).unwrap();
    }
}
