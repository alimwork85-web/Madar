# Madar

Madar is a personal tool for Windows 11. It shows a wheel of seven places at the pointer.
Developer: Ali Alhumedi.

## Opening the wheel

- Press Ctrl+Space. This is the default; Settings can change it.
- Press the mouse button chosen in Settings: Back, Forward, Ctrl+Middle or None.
- Click the tray icon and choose Open wheel.
- Start madar.exe again. A copy that is already running shows its wheel and the new one exits.

## Using it

- Click a slot to open it. Click outside the wheel to close it.
- Hold the summon button and drag, then release on a slot.
- A digit opens the slot with that number. The number pad works too.
- Arrows, Tab and Shift+Tab move between slots.
- Enter opens the highlighted slot.
- Backspace goes back from a folder, and closes the wheel at the top level.
- Esc closes the wheel. Clicking the hub goes back.

The hub shows Close at the top level and Back inside a folder.

## Places

| Place | What it holds |
|---|---|
| System | Run, Task Manager and Maintenance |
| Settings | Opens Settings in Windows |
| Sound | Not ready |
| Bluetooth | Not ready |
| Apps | Your programs, up to 8 |
| Sites | Your web links, up to 8 |
| Google | Not ready |

Sound, Bluetooth, Google and System > Maintenance are not ready yet. They show Not ready when picked.
While a list is not full, its folder keeps one free place that opens the Settings window.

## Settings

The Settings window changes the shortcut, the mouse button, the theme, starting with Windows and
both lists.

- A theme is picked by name from 8 themes. Midnight is the default.
- Add app from Start shows the Start menu program list with a search box, Store apps included.
- Add app asks for a name, a program path and optional arguments. Browse picks the file.
- Add site asks for a name, an http or https address and an optional browser.
- Each list holds at most 8 items and has Edit, Remove, Move up and Move down.
- Changes are saved as soon as they are made. A control that fails shows its error underneath.

## The settings file

Settings live in `%APPDATA%\Madar\config.json`.
If the file cannot be read, Madar starts with defaults and shows a warning.
A save that fails leaves the previous file in place.
The `MADAR_DIR` environment variable points Madar at another folder, for tests.

## Building

Rust stable with the MSVC toolchain, then:

    cargo build --release

The result is `target\release\madar.exe`. There are two direct dependencies: `serde_json` and `windows`.
The program makes no network calls, needs no account and has no updater.

## Copyright

© 2026 Ali Alhumedi. All rights reserved.
