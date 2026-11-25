# Madar design

Status: approved 2026-11-25. Interface language is English, digits are 0-9.

## Purpose and feel
A tool opened many times a day for under two seconds. Precise and quiet: one wheel, eight fixed places, no decoration.
Signature: the eight-slot wheel and the eight-slot app icon are the same shape.

## App icon
`assets/madar.svg`, `assets/madar.ico`, drawn by `tools/make_icon.py`. Colors never follow the theme.

## Wheel (logical pixels at 96 DPI, scaled per monitor)
| Part | Value |
|---|---|
| Plate diameter | 340 |
| Slot outer / inner radius | 166 / 66 |
| Gap between slots | 8 at the outer edge, 8 at the inner edge (parallel sides) |
| Hub diameter | 116 |
| Icon | 22, Segoe Fluent Icons; icon and label centered as one block at radius 117, digit right of the icon |
| Label / digit | 13 / 12, Segoe UI Variable Text, regular |
| First slot | centered at 12 o'clock, then clockwise |

Order: My Apps, Apps, Sites, Google, System, Settings, Sound, Tools. A folder replaces the slots in place. Every page shows 8 places.
My Apps holds the programs the user made, Tools the user's tools, Apps all others. They work alike and a program can move between them.
With Show names off, every slot shows its icon alone, 32 and centered, without digit and label. The digits still work and the hub still names the slot.
Sound: Volume up, Mute (Unmute when muted), Volume down, then the output devices by name, the current one with a check mark. With more than 5 devices: 4, the current one among them, then `More`, which opens Windows sound settings. Changing the volume or the device keeps the wheel open. The calls device is never changed.
The mouse wheel changes the volume by 2% a notch anywhere on the Sound page and over the Sound slot of the first page; raising it unmutes. On the Sound page Up and Down change the volume; Left, Right and Tab move. The hub shows `40%` or `Muted` under the slot's name, the full name of a device, or `No output device`.
Hub text: `Close` at the root, `Back` inside a folder, the slot name while hovering. Labels wrap to two lines, then end with an ellipsis.

## Slot states
| State | Drawing |
|---|---|
| Normal | slot color, text color |
| Hover or keyboard focus | accent color, on-accent text. No motion |
| Pressed | accent color, 2 px inner outline in on-accent |
| Disabled | hub color, text at 60%, label `Not ready` in the hub on hover |
| First free place in My Apps, Apps, Tools or Sites | `Add app` or `Add site`, opens Settings at that list |
| Other free places | slot color at 35% over the plate, no text, no hover, skipped by the keyboard |
| Error | message in the hub for 4 s, wheel stays open |

## Themes
Hex without `#`. Default Midnight. Windows High Contrast overrides every theme.

| Theme | Hub and plate | Slot | Text | Accent | On accent |
|---|---|---|---|---|---|
| Space Gray | 1B1B1E | 2F2F33 | F2F2F4 | D6D6DB | 1B1B1E |
| Midnight | 0F151C | 1E2C38 | EAF2F8 | 7CC4F0 | 0B1A26 |
| Ember | 1B100B | 40211A | FBEDE6 | FF9A6B | 2A0F05 |
| Grape | 150F22 | 33234F | F2ECFB | C7A6FF | 1E0F38 |
| Solar | 1B140A | 47341A | FBF3E2 | F4C04E | 2A1C04 |
| Meadow | 0B1A14 | 1D4034 | E8F5EE | 7FDDB0 | 06251A |
| Aqua | FAFCFF | E6EAF0 | 17202E | 0A66C2 | FFFFFF |
| Porcelain | EEF2F5 | CFD9E2 | 1C2833 | 2F5D7C | FFFFFF |

Colored program icons get a neutral base behind them when their contrast with the slot is below 3:1.

## Motion
Keyboard: none. Mouse: opacity and 0.96 to 1 scale over 120 ms, ease-out. Closing: none. All off when Windows animations are off.

## Settings window
Native controls, 480 x 560, resizable, Windows light or dark title bar. One column, labels left, controls right, 16 px margin, 36 px rows.
Order: Shortcut, Mouse button, Theme, Start with Windows, Show names, then a switch (My Apps, Apps, Tools, Sites) over one list. Buttons: Add, Add from Start, Edit, Remove, Move up, Move down, Move to... The last opens a menu of the other two program lists, a full one grayed; it is off for Sites and when both are full.
Changes save at once. An error appears under its control. Every control is reachable with Tab and has an access key.

## Tray menu
Open wheel, Settings, Pause (Resume when paused), separator, Exit.
