"""Draws assets/madar.svg and assets/madar.ico. Run: python tools/make_icon.py"""
import math
from pathlib import Path
from PIL import Image, ImageDraw

OUT = Path(__file__).resolve().parent.parent / "assets"
SIZES = [16, 20, 24, 32, 48, 64, 128, 256]
BLUE, WHITE, MINT = "#1F3FBF", "#FFFFFF", "#9BE8C9"
OUTER, HUB, RAISED = 25, 10, 27.5
N = 8
START, STEP = -90 - 180 / N, 360 / N


def edge(deg, r):
    a = math.radians(deg)
    return 32 + r * math.cos(a), 32 + r * math.sin(a)


def draw(size):
    n, s = size * 8, size * 8 / 64
    im = Image.new("RGBA", (n, n))
    d = ImageDraw.Draw(im)
    box = lambda r: [(32 - r) * s] * 2 + [(32 + r) * s] * 2
    d.rounded_rectangle((0, 0, n - 1, n - 1), 15 * s, fill=BLUE)
    d.ellipse(box(OUTER), fill=WHITE)
    d.pieslice(box(RAISED), START, START + STEP, fill=MINT)
    gap = 4.5 if size < 32 else 4  # wider gaps survive at tray size
    for k in range(N):
        x, y = edge(START + k * STEP, 30)
        d.line([32 * s, 32 * s, x * s, y * s], fill=BLUE, width=round(gap * s))
    d.ellipse(box(HUB), fill=BLUE)
    return im.resize((size, size), Image.Resampling.LANCZOS)


def svg():
    (ax, ay), (bx, by) = edge(START, RAISED), edge(START + STEP, RAISED)
    spokes = "".join("M32 32L%.2f %.2f" % edge(START + k * STEP, 30) for k in range(N))
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><rect width="64" height="64" rx="15" '
            f'fill="{BLUE}"/><circle cx="32" cy="32" r="{OUTER}" fill="{WHITE}"/>'
            f'<path d="M32 32L{ax:.2f} {ay:.2f}A{RAISED} {RAISED} 0 0 1 {bx:.2f} {by:.2f}Z" fill="{MINT}"/>'
            f'<path d="{spokes}" stroke="{BLUE}" stroke-width="4"/><circle cx="32" cy="32" r="{HUB}" fill="{BLUE}"/></svg>\n')


if __name__ == "__main__":
    (OUT / "madar.svg").write_text(svg(), encoding="utf-8")
    ims = [draw(n) for n in SIZES]
    ims[-1].save(OUT / "madar.ico", sizes=[(n, n) for n in SIZES], append_images=ims[:-1])
    ico = Image.open(OUT / "madar.ico")
    assert ico.info["sizes"] == {(n, n) for n in SIZES}
    assert ico.ico.getimage((16, 16)).convert("RGBA").tobytes() == ims[0].tobytes()
