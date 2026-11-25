"""Draws the README art: banner, section titles, chart, theme gallery, social preview.

    python tools/make_readme_art.py <folder with the wheel screenshots>

Needs fontTools, Pillow, Playwright and Edge. Letters become outlines, so every reader sees the same fonts.
"""

import math
import sys
from pathlib import Path

from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.ttLib import TTFont
from PIL import Image, ImageDraw, ImageFont

ASSETS = Path(__file__).parent.parent / "assets"
FONTS = Path("C:/Windows/Fonts")
# name: plate, slot, accent, font file
THEMES = {
    "Space Gray": ("1B1B1E", "2F2F33", "D6D6DB", "bahnschrift.ttf"),
    "Midnight": ("0F151C", "1E2C38", "7CC4F0", "SegUIVar.ttf"),
    "Ember": ("1B100B", "40211A", "FF9A6B", "georgiab.ttf"),
    "Grape": ("150F22", "33234F", "C7A6FF", "trebucbi.ttf"),
    "Solar": ("1B140A", "47341A", "F4C04E", "constanb.ttf"),
    "Meadow": ("0B1A14", "1D4034", "7FDDB0", "Candarab.ttf"),
    "Aqua": ("FAFCFF", "E6EAF0", "0A66C2", "corbelb.ttf"),
    "Porcelain": ("EEF2F5", "CFD9E2", "2F5D7C", "georgiaz.ttf"),
}
# The order of the README; one theme and one font per title.
TITLES = [
    ("themes", "Eight themes", "Grape"),
    ("download", "Download", "Meadow"),
    ("opening", "Opening the wheel", "Midnight"),
    ("using", "Using it", "Ember"),
    ("places", "Places", "Solar"),
    ("system", "System", "Space Gray"),
    ("google", "Google", "Aqua"),
    ("sound", "Sound & Light", "Grape"),
    ("settings", "Settings", "Porcelain"),
    ("file", "The settings file", "Ember"),
    ("measured", "Measured", "Midnight"),
    ("building", "Building", "Solar"),
    ("license", "License", "Meadow"),
]
# Medians of 15 runs of the 0.2.0 release build, in milliseconds.
START, OPEN = 35, 6
_fonts = {}


def font(file):
    if file not in _fonts:
        f, at = TTFont(FONTS / file), {}
        if "fvar" in f:
            # Bold, at the largest optical size.
            at = {"wght": 700}
            at |= {a.axisTag: a.maxValue for a in f["fvar"].axes if a.axisTag == "opsz"}
        _fonts[file] = f, f.getGlyphSet(location=at)
    return _fonts[file]


def text(s, x, y, size, file, colors):
    """`colors` is one color, or a list with one per letter."""
    f, glyphs = font(file)
    cmap, k = f.getBestCmap(), size / f["head"].unitsPerEm
    out = []
    for i, ch in enumerate(s):
        pen = SVGPathPen(glyphs, lambda v: f"{v:.0f}")
        g = glyphs[cmap[ord(ch)]]
        g.draw(pen)
        color = colors if isinstance(colors, str) else colors[i % len(colors)]
        if pen.getCommands():
            out.append(
                f'<path transform="translate({x:.1f} {y:.1f}) scale({k:.4f} {-k:.4f})" '
                f'fill="#{color}" d="{pen.getCommands()}"/>'
            )
        x += g.width * k
    return "".join(out), x


def wedge(cx, cy, inner, outer, i, gap):
    """One of eight slots with parallel gaps, as in the app."""
    a = math.radians(45 * i - 90)
    pts = []
    for r, side in ((outer, -1), (outer, 1), (inner, 1), (inner, -1)):
        t = a + side * (math.radians(22.5) - math.asin(gap / 2 / r))
        pts.append(f"{cx + r * math.cos(t):.2f} {cy + r * math.sin(t):.2f}")
    return f"M{pts[0]}A{outer:.2f} {outer:.2f} 0 0 1 {pts[1]}L{pts[2]}A{inner:.2f} {inner:.2f} 0 0 0 {pts[3]}Z"


def wheel(cx, cy, r):
    """Each slot wears a theme: its slot color, and its accent on the rim."""
    s = f'<circle cx="{cx}" cy="{cy}" r="{r}" fill="#0B0F14"/>'
    for i, (_, slot, accent, file) in enumerate(THEMES.values()):
        s += f'<path d="{wedge(cx, cy, r * .39, r * .86, i, r * .047)}" fill="#{slot}"/>'
        s += f'<path d="{wedge(cx, cy, r * .88, r * .975, i, r * .047)}" fill="#{accent}"/>'
        a = math.radians(45 * i - 90)
        x, y = cx + r * 0.63 * math.cos(a), cy + r * 0.63 * math.sin(a) + r * 0.07
        _, w = text(str(i + 1), 0, 0, r * 0.2, file, accent)
        s += text(str(i + 1), x - w / 2, y, r * 0.2, file, accent)[0]
    return s + f'<circle cx="{cx}" cy="{cy}" r="{r * .34:.1f}" fill="#7CC4F0"/>'


def svg(w, h, body, label):
    label = label.replace("&", "&amp;")
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}" '
        f'role="img" aria-label="{label}"><title>{label}</title>{body}</svg>\n'
    )


def hero(w, h, r):
    letters = [THEMES[t][2] for t in ("Midnight", "Ember", "Grape", "Solar", "Meadow")]
    left, mid = w * 0.06, h / 2
    x = left + r * 2 + w * 0.06
    k = w / 1280
    s = f'<rect width="{w}" height="{h}" fill="#0F151C"/>' + wheel(left + r, mid, r)
    s += text("Madar", x, mid + 10 * k, 170 * k, "SegUIVar.ttf", letters)[0]
    s += text("Eight places at your pointer", x + 6, mid + 74 * k, 40 * k, "georgiab.ttf", "FBEDE6")[0]
    s += text("Windows 11. Developer: Ali Alhumedi", x + 6, mid + 122 * k, 25 * k, "bahnschrift.ttf", "D6D6DB")[0]
    return svg(w, h, s, "Madar: eight places at your pointer. Developer: Ali Alhumedi")


def title(label, theme):
    plate, slot, accent, file = THEMES[theme]
    body, _ = text(label, 64, 38, 29, file, accent)
    s = f'<rect x=".5" y=".5" width="859" height="55" rx="8" fill="#{plate}" stroke="#{slot}"/>'
    # The slot of this title's theme is the lit one.
    for i, name in enumerate(THEMES):
        s += f'<path d="{wedge(32, 28, 7, 19, i, 2)}" fill="#{accent if name == theme else slot}"/>'
    return svg(860, 56, s + body, label)


def chart():
    s = '<rect width="860" height="210" rx="8" fill="#0F151C"/>'
    s += text("Time until Madar is ready, in milliseconds", 32, 44, 21, "SegUIVar.ttf", "EAF2F8")[0]
    for y, label, v in ((72, "Start the program", START), (122, "Show the wheel", OPEN)):
        end = 250 + v * 14
        s += text(label, 32, y + 19, 17, "SegUIVar.ttf", "EAF2F8")[0]
        s += f'<path d="M250 {y}H{end - 4}a4 4 0 0 1 4 4v18a4 4 0 0 1-4 4H250Z" fill="#7CC4F0"/>'
        s += text(f"{v} ms", end + 12, y + 19, 18, "SegUIVar.ttf", "EAF2F8")[0]
    s += '<path d="M249.5 62V158" stroke="#4A5A68"/>'
    note = "Medians of 15 runs, version 0.2.0, Ryzen 7 250, Windows 11"
    s += text(note, 32, 188, 14, "bahnschrift.ttf", "B9C7D3")[0]
    return svg(860, 210, s, f"Start the program: {START} ms. Show the wheel: {OPEN} ms.")


def gallery(shots):
    side, pad, row = 300, 16, 356
    out = Image.new("RGBA", (4 * (side + pad) + pad, 2 * row), (0, 0, 0, 0))
    big = side * 4
    mask = Image.new("L", (big, big))
    ImageDraw.Draw(mask).ellipse((10, 10, big - 10, big - 10), fill=255)
    mask = mask.resize((side, side), Image.LANCZOS)
    draw = ImageDraw.Draw(out)
    for i, (name, (plate, _, accent, file)) in enumerate(THEMES.items()):
        x, y = pad + i % 4 * (side + pad), i // 4 * row
        shot = Image.open(shots / f"wheel-{name.replace(' ', '-')}.png").convert("RGB")
        out.paste(shot.resize((side, side), Image.LANCZOS), (x, y + 2), mask)
        box = (x + 60, y + side + 8, x + side - 60, y + side + 44)
        draw.rounded_rectangle(box, 8, fill=f"#{plate}", outline=f"#{accent}")
        face = ImageFont.truetype(str(FONTS / file), 22)
        draw.text((x + side / 2, y + side + 26), name, f"#{accent}", face, anchor="mm")
    out.save(ASSETS / "themes.png", optimize=True)


def main():
    def save(name, s):
        (ASSETS / name).write_text(s, encoding="utf8", newline="\n")

    (ASSETS / "titles").mkdir(exist_ok=True)
    save("banner.svg", hero(1280, 400, 170))
    save("performance.svg", chart())
    for file, label, theme in TITLES:
        save(f"titles/{file}.svg", title(label, theme))
    gallery(Path(sys.argv[1]))
    from playwright.sync_api import sync_playwright

    with sync_playwright() as p:
        page = p.chromium.launch(channel="msedge").new_page(viewport={"width": 1280, "height": 640})
        page.set_content(f'<body style="margin:0">{hero(1280, 640, 215)}')
        page.screenshot(path=str(ASSETS / "social-preview.png"))


main()
