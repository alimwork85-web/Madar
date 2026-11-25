"""Draws assets/madar.svg and assets/madar.ico. Run: python tools/make_icon.py"""
import math
from pathlib import Path
from PIL import Image, ImageDraw

OUT = Path(__file__).resolve().parent.parent / "assets"
SIZES = [16, 20, 24, 32, 48, 64, 128, 256]
BLUE, WHITE, MINT = "#1F3FBF", "#FFFFFF", "#9BE8C9"
# Below 32 px the slots are thinner and pushed outward so the gaps survive.
SMALL = dict(stroke=7, inner=13.5, outer=19, tip=24.5)
LARGE = dict(stroke=8, inner=12, outer=18, tip=22)


def slots(g):
    for k in range(7):
        a = k * 2 * math.pi / 7 - math.pi / 2
        end = g["tip"] if k == 0 else g["outer"]
        yield (MINT if k == 0 else WHITE,
               *[(32 + r * math.cos(a), 32 + r * math.sin(a)) for r in (g["inner"], end)])


def draw(size):
    g, s = (SMALL if size < 32 else LARGE), size * 8 / 64
    im = Image.new("RGBA", (size * 8,) * 2)
    d = ImageDraw.Draw(im)
    d.rounded_rectangle((0, 0, size * 8 - 1, size * 8 - 1), 15 * s, fill=BLUE)
    r = g["stroke"] / 2
    for color, p, q in slots(g):
        d.line([c * s for c in (*p, *q)], fill=color, width=round(g["stroke"] * s))
        for x, y in (p, q):
            d.ellipse([c * s for c in (x - r, y - r, x + r, y + r)], fill=color)
    return im.resize((size, size), Image.Resampling.LANCZOS)


def svg():
    lines = "".join(
        f'<path d="M{p[0]:.2f} {p[1]:.2f}L{q[0]:.2f} {q[1]:.2f}" stroke="{c}"/>' for c, p, q in slots(LARGE))
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><rect width="64" height="64" rx="15" '
            f'fill="{BLUE}"/><g stroke-width="8" stroke-linecap="round">{lines}</g></svg>\n')


if __name__ == "__main__":
    (OUT / "madar.svg").write_text(svg(), encoding="utf-8")
    ims = [draw(n) for n in SIZES]
    ims[-1].save(OUT / "madar.ico", sizes=[(n, n) for n in SIZES], append_images=ims[:-1])
    ico = Image.open(OUT / "madar.ico")
    assert ico.info["sizes"] == {(n, n) for n in SIZES}
    assert ico.ico.getimage((16, 16)).convert("RGBA").tobytes() == ims[0].tobytes()
