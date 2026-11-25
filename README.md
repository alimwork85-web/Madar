# Madar

Madar is a personal tool for Windows 11. It shows a wheel of eight places at the pointer.
Developer: Ali Alhumedi.

## Opening the wheel

- Press Ctrl+Space. This is the default; Settings can change it.
- Press the mouse button chosen in Settings: Back (the default), Forward or Ctrl + Middle. None turns it off.
- Click the tray icon. A right click shows its menu: Open wheel, Settings, Pause, Exit.
- Start madar.exe again. A copy that is already running shows its wheel and the new one exits.

## Using it

- Click a slot to open it. Click outside the wheel to close it.
- Hold the summon button and drag, then release on a slot.
- A digit opens the slot with that number. The number pad works too.
- Arrows, Tab and Shift+Tab move between slots. On the Sound page Up and Down change the volume instead.
- Enter opens the highlighted slot. With none highlighted it works like Backspace.
- Backspace goes back from a folder, and closes the wheel at the top level.
- Esc closes the wheel. Clicking the hub works like Backspace.

The hub shows Close at the top level and Back inside a folder. A highlighted slot shows its name there, or Not ready.
On the Sound page the hub shows the level as well, under the slot's name.

## Places

Every page has eight places. At the top level, clockwise from the top:

| Place | What it holds |
|---|---|
| My Apps | The programs you made, up to 8 |
| Apps | Your other programs, up to 8 |
| Sites | Your web links, up to 8 |
| Google | Not ready |
| System | Run, Task Manager and Maintenance |
| Settings | Opens Settings in Windows |
| Sound | Volume, mute and the output devices |
| Bluetooth | Not ready |

Google, Bluetooth and System > Maintenance are not ready yet. They show Not ready when picked.
While a list is not full, its next free place reads Add app or Add site and opens Settings at that list.
All other free places stay empty and the keyboard skips them.

## Sound

Sound opens a page of its own. Clockwise from the top: Volume up, Mute (Unmute while muted), Volume down,
then the connected output devices by name, the one in use with a check mark.
Only connected devices are listed: the list is read when the page opens and after choosing a device.
A Bluetooth device that is paired but not connected is not listed.
With more than 5 devices the page shows 4, the one in use among them, and More, which opens the sound page of Windows Settings.

The mouse wheel changes the volume by 2% a notch anywhere on the Sound page, and over the Sound slot at the top level.
The wheel stays open after a change. Raising the volume unmutes.
On the Sound page Up and Down change the volume; Left, Right, Tab and Shift+Tab move between slots.

The hub shows the level as a percentage, or Muted, or No output device, under the name of the highlighted slot.
A highlighted device shows its name instead.

Choosing a device makes it the Windows default output. The default for calls is not changed.
Windows documents no call for this, so Madar uses an undocumented one in a short child process, reads the default back,
and opens the sound page of Windows Settings when the change did not happen.
The interface layout comes from com-policy-config by sidit77, under the MIT licence.

## Settings

The Settings window changes the shortcut, the mouse button, the theme, starting with Windows, Show names and the lists.

- A theme is picked by name from 8 themes. Midnight is the default.
- Show names off draws each slot as its icon alone, without label or digit. The digits still work.
- One list shows at a time: My Apps, Apps or Sites.
- Add asks for a name, a program path and optional arguments, or for a site a name, an http or https
  address and an optional browser. Browse picks the file; the Start menu button,
  for programs only, fills the path. Both fill an empty name.
- Add from Start shows the Start menu program list with a search box, Store apps included.
- Buttons: Add, Add from Start, Edit, Remove, Move up, Move down, and Move to Apps or Move to My Apps.
- Add from Start and Move to are off for Sites. Move to is off while the other program list is full.
- Each list holds at most 8 items. Changes save at once; a failing control shows its error underneath.

## The settings file

Settings live in `%APPDATA%\Madar\config.json`. A key that is missing keeps its default, so a file
from an earlier version still loads.
If the file cannot be read, Madar starts with defaults and shows a warning.
The first change then asks before replacing it, and keeps a copy next to it.
A save that fails leaves the previous file in place.
The `MADAR_DIR` environment variable points Madar at another folder, for tests.

## Building

Rust stable with the MSVC toolchain, then:

    cargo build --release

The result is `target\release\madar.exe`. There are two direct dependencies: `serde_json` and `windows`.
The program makes no network calls, needs no account and has no updater.

## Copyright

© 2026 Ali Alhumedi. All rights reserved.
