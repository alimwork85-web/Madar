//! What the wheel's slots do: turn an `Action` into a launch plan, then run it.
//!
//! No interpreter is ever started and no command line is ever joined by hand:
//! documents and folders go through `ShellExecuteExW`, a chosen browser is
//! started with exactly one argument.

use crate::config::{Config, is_web_url};
use crate::wheel::Action;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use windows::Win32::Foundation::{ERROR_CANCELLED, HINSTANCE, HWND};
use windows::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize,
};
use windows::Win32::System::SystemInformation::GetSystemDirectoryW;
use windows::Win32::UI::Shell::{
    SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW, ShellExecuteExW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HRESULT, PCWSTR};

/// The shell folder behind the Run dialog.
const RUN: &str = "shell:::{2559a1f3-21d7-11d4-bdaf-00c04f60b9f0}";
/// The folder behind the Start menu's list; an app id follows.
pub const START_APPS: &str = "shell:AppsFolder\\";
const SETTINGS: &str = "ms-settings:";
const BLUETOOTH_SETTINGS: &str = "ms-settings:bluetooth";
const SOUND_SETTINGS: &str = "ms-settings:sound";

#[derive(Debug, PartialEq)]
enum Plan {
    /// ShellExecuteExW: file, raw parameters, working folder.
    Shell {
        file: String,
        args: String,
        dir: Option<PathBuf>,
    },
    /// std::process::Command with exactly one argument.
    Spawn { exe: PathBuf, arg: String },
}

fn plan(action: Action, c: &Config) -> Result<Plan, String> {
    use Plan::*;
    match action {
        Action::Run => Ok(Shell {
            file: RUN.to_string(),
            args: String::new(),
            dir: None,
        }),
        Action::Settings => Ok(Shell {
            file: SETTINGS.to_string(),
            args: String::new(),
            dir: None,
        }),
        Action::TaskManager => Ok(Shell {
            file: format!("{}\\taskmgr.exe", system_dir()?),
            args: String::new(),
            dir: None,
        }),
        Action::MyApp(i) | Action::App(i) | Action::Tool(i) => {
            let apps = match action {
                Action::MyApp(_) => &c.my_apps,
                Action::Tool(_) => &c.tools,
                _ => &c.apps,
            };
            let app = apps.get(i).ok_or("Item not found")?;
            if app.args.contains('\0') {
                return Err("Invalid arguments".to_string());
            }
            let exe = program(&app.path, true)?;
            Ok(Shell {
                file: app.path.clone(),
                args: app.args.clone(),
                dir: exe.parent().map(Path::to_path_buf),
            })
        }
        Action::Site(i) => {
            let site = c.sites.get(i).ok_or("Item not found")?;
            if !is_web_url(&site.url) {
                return Err("Invalid address".to_string());
            }
            match &site.browser {
                None => Ok(Shell {
                    file: site.url.clone(),
                    args: String::new(),
                    dir: None,
                }),
                Some(browser) => Ok(Spawn {
                    exe: program(browser, false)?,
                    arg: site.url.clone(),
                }),
            }
        }
        Action::BluetoothSettings => Ok(Shell {
            file: BLUETOOTH_SETTINGS.to_string(),
            args: String::new(),
            dir: None,
        }),
        Action::SoundSettings => Ok(Shell {
            file: SOUND_SETTINGS.to_string(),
            args: String::new(),
            dir: None,
        }),
        // The caller handles these.
        Action::AddItems
        | Action::Bluetooth
        | Action::Google
        | Action::VolumeUp
        | Action::VolumeDown
        | Action::Mute
        | Action::Output(_) => Err("Not ready".to_string()),
    }
}

/// True for a Start menu program. Windows checks that it exists on launch.
pub fn is_start_app(path: &str) -> bool {
    (path.strip_prefix(START_APPS)).is_some_and(|id| {
        !id.trim().is_empty() && !id.contains(char::is_control) && !id.contains("..")
    })
}

/// Checks an absolute, existing program path. `shortcut` also allows `.lnk`
/// and Start menu programs, which have no folder: their path comes back empty.
pub fn program(path: &str, shortcut: bool) -> Result<PathBuf, String> {
    if path.is_empty() || path.contains('\0') {
        return Err("Invalid path".to_string());
    }
    if shortcut && is_start_app(path) {
        return Ok(PathBuf::new());
    }
    let p = Path::new(path);
    if !p.is_absolute() {
        return Err("Full path needed".to_string());
    }
    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
    let allowed = ext.eq_ignore_ascii_case("exe") || (shortcut && ext.eq_ignore_ascii_case("lnk"));
    if !allowed {
        return Err("Not a program".to_string());
    }
    if !p.is_file() {
        return Err("File not found".to_string());
    }
    Ok(p.to_path_buf())
}

fn system_dir() -> Result<String, String> {
    let mut buf = vec![0u16; 260];
    loop {
        let n = unsafe { GetSystemDirectoryW(Some(&mut buf)) } as usize;
        if n == 0 {
            return Err("Could not open".to_string());
        }
        if n < buf.len() {
            return Ok(String::from_utf16_lossy(&buf[..n]));
        }
        buf.resize(n + 1, 0);
    }
}

pub fn run(action: Action, c: &Config) -> Result<(), String> {
    match plan(action, c)? {
        Plan::Shell { file, args, dir } => shell(&file, &args, dir.as_deref()),
        Plan::Spawn { exe, arg } => spawn(&exe, &arg),
    }
}

pub fn open_url(url: &str) -> Result<(), String> {
    if !is_web_url(url) {
        return Err("Invalid address".to_string());
    }
    shell(url, "", None)
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn shell(file: &str, args: &str, dir: Option<&Path>) -> Result<(), String> {
    let file = wide(file);
    let args = wide(args);
    let dir: Option<Vec<u16>> = dir.map(|p| p.as_os_str().encode_wide().chain([0]).collect());

    // The buffers above outlive the call; nothing here may move before it ends.
    unsafe {
        let initialized =
            CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE).is_ok();
        let mut info = SHELLEXECUTEINFOW {
            cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI,
            hwnd: HWND::default(),
            lpVerb: PCWSTR::null(),
            lpFile: PCWSTR(file.as_ptr()),
            lpParameters: PCWSTR(args.as_ptr()),
            lpDirectory: dir.as_ref().map_or(PCWSTR::null(), |d| PCWSTR(d.as_ptr())),
            nShow: SW_SHOWNORMAL.0,
            hInstApp: HINSTANCE::default(),
            ..Default::default()
        };
        let result = match ShellExecuteExW(&mut info) {
            Ok(()) => Ok(()),
            // A declined elevation prompt is the user's answer, not an error.
            Err(e) if e.code() == HRESULT::from_win32(ERROR_CANCELLED.0) => Ok(()),
            Err(_) => Err("Could not open".to_string()),
        };
        if initialized {
            CoUninitialize();
        }
        result
    }
}

fn spawn(exe: &Path, arg: &str) -> Result<(), String> {
    let mut cmd = std::process::Command::new(exe);
    cmd.arg(arg)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    if let Some(dir) = exe.parent() {
        cmd.current_dir(dir);
    }
    // The child is dropped, never waited for.
    match cmd.spawn() {
        Ok(_) => Ok(()),
        Err(_) => Err("Could not start".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{App, Site};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// Unique folder per test, under the OS temp dir.
    fn scratch(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!(
            "madar-actions-{name}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn file(path: &Path) {
        fs::write(path, b"MZ").unwrap();
    }

    fn config(apps: Vec<App>, sites: Vec<Site>) -> Config {
        Config {
            apps,
            sites,
            ..Config::default()
        }
    }

    fn app(path: &str, args: &str) -> App {
        App {
            label: "a".to_string(),
            path: path.to_string(),
            args: args.to_string(),
        }
    }

    fn site(url: &str, browser: Option<&Path>) -> Site {
        Site {
            label: "s".to_string(),
            url: url.to_string(),
            browser: browser.map(|b| b.to_string_lossy().into_owned()),
        }
    }

    /// The error text, checked to stay short and free of paths.
    fn err(action: Action, c: &Config) -> String {
        let e = plan(action, c).unwrap_err();
        assert!(e.len() < 40, "{e}");
        assert!(!e.contains('\u{2014}'), "{e}");
        assert!(!e.contains(['\\', '/', ':', '.']), "{e}");
        e
    }

    #[test]
    fn system_actions() {
        let c = Config::default();
        assert_eq!(
            plan(Action::Run, &c).unwrap(),
            Plan::Shell {
                file: "shell:::{2559a1f3-21d7-11d4-bdaf-00c04f60b9f0}".to_string(),
                args: String::new(),
                dir: None,
            }
        );
        assert_eq!(
            plan(Action::Settings, &c).unwrap(),
            Plan::Shell {
                file: "ms-settings:".to_string(),
                args: String::new(),
                dir: None,
            }
        );
        assert_eq!(
            plan(Action::SoundSettings, &c).unwrap(),
            Plan::Shell {
                file: "ms-settings:sound".to_string(),
                args: String::new(),
                dir: None,
            }
        );
        let Plan::Shell { file, .. } = plan(Action::BluetoothSettings, &c).unwrap() else {
            panic!("settings go through the shell");
        };
        assert_eq!(file, "ms-settings:bluetooth");
        let Plan::Shell { file, args, dir } = plan(Action::TaskManager, &c).unwrap() else {
            panic!("Task Manager must go through the shell");
        };
        assert!(Path::new(&file).is_absolute(), "{file}");
        assert!(file.to_lowercase().ends_with("\\taskmgr.exe"), "{file}");
        assert!(args.is_empty() && dir.is_none());
    }

    #[test]
    fn app_keeps_file_args_and_folder() {
        let dir = scratch("app");
        let exe = dir.join("Madar App.exe");
        file(&exe);
        let path = exe.to_string_lossy().into_owned();
        let args = "--open \"a b\" &| %PATH%";
        let c = config(vec![app(&path, args)], vec![]);
        assert_eq!(
            plan(Action::App(0), &c).unwrap(),
            Plan::Shell {
                file: path,
                args: args.to_string(),
                dir: Some(dir.clone()),
            }
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn my_apps_is_its_own_list() {
        let mut c = config(vec![app("shell:AppsFolder\\other", "")], vec![]);
        assert_eq!(err(Action::MyApp(0), &c), "Item not found");
        c.my_apps = vec![app("shell:AppsFolder\\mine", "")];
        let Plan::Shell { file, .. } = plan(Action::MyApp(0), &c).unwrap() else {
            panic!("a program goes through the shell");
        };
        assert!(file.ends_with("mine"));
        assert_eq!(err(Action::Tool(0), &c), "Item not found");
        c.tools = vec![app("shell:AppsFolder\\tool", "")];
        let Plan::Shell { file, .. } = plan(Action::Tool(0), &c).unwrap() else {
            panic!("a program goes through the shell");
        };
        assert!(file.ends_with("tool"));
    }

    #[test]
    fn start_menu_apps() {
        let path = "shell:AppsFolder\\Some.App_x!App";
        let c = config(vec![app(path, "")], vec![site("https://a.b", None)]);
        assert_eq!(
            plan(Action::App(0), &c).unwrap(),
            Plan::Shell {
                file: path.to_string(),
                args: String::new(),
                dir: None,
            }
        );
        assert!(program(path, false).is_err());
        for bad in [
            "shell:AppsFolder\\",
            "shell:Other\\x",
            "shell:AppsFolder\\..\\x",
        ] {
            assert!(program(bad, true).is_err(), "{bad}");
        }
    }

    #[test]
    fn app_accepts_shortcuts_and_uppercase_extension() {
        let dir = scratch("lnk");
        let lnk = dir.join("Browser.LNK");
        file(&lnk);
        let exe = dir.join("TOOL.EXE");
        file(&exe);
        let c = config(
            vec![
                app(&lnk.to_string_lossy(), ""),
                app(&exe.to_string_lossy(), ""),
            ],
            vec![],
        );
        assert!(matches!(
            plan(Action::App(0), &c).unwrap(),
            Plan::Shell { .. }
        ));
        assert!(matches!(
            plan(Action::App(1), &c).unwrap(),
            Plan::Shell { .. }
        ));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn arabic_path_with_spaces() {
        let root = scratch("arabic");
        let dir = root.join("مجلد التطبيقات");
        fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("برنامج التحميل.exe");
        file(&exe);
        let path = exe.to_string_lossy().into_owned();
        let c = config(vec![app(&path, "")], vec![]);
        assert_eq!(
            plan(Action::App(0), &c).unwrap(),
            Plan::Shell {
                file: path,
                args: String::new(),
                dir: Some(dir),
            }
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn app_rejects_everything_but_programs() {
        let dir = scratch("reject");
        let folder = dir.join("folder.exe");
        fs::create_dir_all(&folder).unwrap();
        let bat = dir.join("run.bat");
        let cmd = dir.join("run.cmd");
        let ps1 = dir.join("run.ps1");
        let gone = dir.join("gone.exe");
        for p in [&bat, &cmd, &ps1] {
            file(p);
        }
        let cases = [
            (bat.to_string_lossy().into_owned(), "Not a program"),
            (cmd.to_string_lossy().into_owned(), "Not a program"),
            (ps1.to_string_lossy().into_owned(), "Not a program"),
            (folder.to_string_lossy().into_owned(), "File not found"),
            (gone.to_string_lossy().into_owned(), "File not found"),
            ("run.exe".to_string(), "Full path needed"),
            (r"%APPDATA%\x.exe".to_string(), "Full path needed"),
            (format!("{}\\nul\0.exe", dir.display()), "Invalid path"),
        ];
        for (path, want) in cases {
            let c = config(vec![app(&path, "")], vec![]);
            assert_eq!(err(Action::App(0), &c), want, "{path}");
        }
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn nul_in_args_is_rejected() {
        let dir = scratch("nul-args");
        let exe = dir.join("a.exe");
        file(&exe);
        let c = config(vec![app(&exe.to_string_lossy(), "x\0y")], vec![]);
        assert!(err(Action::App(0), &c).contains("Invalid"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn site_without_browser_goes_through_the_shell() {
        let url = "https://example.com/a?b=1&c=\\";
        let c = config(vec![], vec![site(url, None)]);
        assert_eq!(
            plan(Action::Site(0), &c).unwrap(),
            Plan::Shell {
                file: url.to_string(),
                args: String::new(),
                dir: None,
            }
        );
    }

    #[test]
    fn site_with_browser_passes_one_argument() {
        let dir = scratch("browser");
        let exe = dir.join("browser.exe");
        file(&exe);
        let url = "https://example.com/a?b=1&c=\\";
        let c = config(vec![], vec![site(url, Some(&exe))]);
        assert_eq!(
            plan(Action::Site(0), &c).unwrap(),
            Plan::Spawn {
                exe: exe.clone(),
                arg: url.to_string(),
            }
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn site_browser_must_be_a_real_exe() {
        let dir = scratch("browser-bad");
        let lnk = dir.join("browser.lnk");
        let bat = dir.join("browser.bat");
        for p in [&lnk, &bat] {
            file(p);
        }
        let missing = dir.join("gone.exe");
        let cases = [
            (lnk.clone(), "Not a program"),
            (bat.clone(), "Not a program"),
            (missing, "File not found"),
            (PathBuf::from("browser.exe"), "Full path needed"),
        ];
        for (browser, want) in cases {
            let c = config(vec![], vec![site("https://example.com", Some(&browser))]);
            assert_eq!(err(Action::Site(0), &c), want, "{}", browser.display());
        }
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn site_rejects_bad_addresses() {
        for url in [
            "",
            "example.com",
            "ftp://example.com",
            "http://",
            "http://exa mple.com",
            r#"https://a.b/"--flag"#,
            "http://a.b/\u{0}",
        ] {
            let c = config(vec![], vec![site(url, None)]);
            assert_eq!(err(Action::Site(0), &c), "Invalid address", "{url}");
        }
    }

    #[test]
    fn missing_items_and_add_slot_are_errors() {
        let c = Config::default();
        assert_eq!(err(Action::App(0), &c), "Item not found");
        assert_eq!(err(Action::Site(3), &c), "Item not found");
        assert!(plan(Action::AddItems, &c).is_err());
    }
}
