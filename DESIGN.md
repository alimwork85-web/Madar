# Madar design

Status: approved 2026-11-25. Interface language is English, digits are 0-9.

## Purpose and feel
A tool opened many times a day for under two seconds. Precise and quiet: one wheel, seven fixed places, no decoration.
Signature: the seven-slot wheel and the seven-slot app icon are the same shape.

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

Order: System, Settings, Sound, Bluetooth, Apps, Sites, Google. A folder replaces the slots in place, at most 8.
Hub text: `Close` at the root, `Back` inside a folder, the slot name while hovering. Labels wrap to two lines, then end with an ellipsis.

## Slot states
| State | Drawing |
|---|---|
| Normal | slot color, text color |
| Hover or keyboard focus | accent color, on-accent text. No motion |
| Pressed | accent color, 2 px inner outline in on-accent |
| Disabled | hub color, text at 60%, label `Not ready` in the hub on hover |
| Empty folder | one slot: `Add in Settings` |
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
Order: Shortcut, Mouse button, Theme, Start with Windows, Apps list, Sites list. Lists have Add, Edit, Remove, Move up, Move down.
Changes save at once. An error appears under its control. Every control is reachable with Tab and has an access key.

## Tray menu
Open wheel, Settings, Pause (Resume when paused), separator, Exit.
