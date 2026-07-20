#!/usr/bin/env python3
"""Assemble the self-contained NEONWIRE OS dossier artifact.

Subsets JetBrains Mono (regular + bold) to woff, inlines the panel screenshots
as data URIs, and injects everything into dossier.template.html. Output is a
single fully self-contained HTML file (Artifact CSP blocks external assets).
"""
from __future__ import annotations

import base64
import io
import sys
from pathlib import Path

from fontTools.subset import Subsetter, Options
from fontTools.ttLib import TTFont
from PIL import Image

HERE = Path(__file__).resolve().parent
SP = Path(sys.argv[1]) if len(sys.argv) > 1 else HERE
FONT_DIR = Path("/usr/share/fonts/TTF")
OUT = SP / "neonwire-dossier.html"

CHARS = list(range(0x20, 0x7F)) + [
    0x00A0, 0x00B7, 0x00D7, 0x2011, 0x2013, 0x2014, 0x2018, 0x2019, 0x201C,
    0x201D, 0x2022, 0x2026, 0x2039, 0x203A, 0x25B8, 0x202F, 0x2192, 0x2588,
    0x2591, 0x2592, 0x2593, 0x2713, 0x2714,
]

IMAGES = {
    "%%IMG_SPLASH%%":   SP / "neonos-splash2.png",
    "%%IMG_DASH%%":     SP / "shot2.png",
    "%%IMG_LAUNCHER%%": SP / "act-final.png",
    "%%IMG_KERNLOG%%":  SP / "ui-3.png",
    "%%IMG_NET%%":      SP / "ui-4.png",
    "%%IMG_CONFIRM%%":  SP / "act-confirm.png",
}


def subset_woff(path: Path) -> str:
    opts = Options()
    opts.flavor = "woff"           # zlib, no brotli needed
    opts.desubroutinize = True
    opts.notdef_outline = True
    opts.recalc_bounds = True
    font = TTFont(str(path))
    ss = Subsetter(options=opts)
    ss.populate(unicodes=CHARS)
    ss.subset(font)
    buf = io.BytesIO()
    font.save(buf)
    b = buf.getvalue()
    return "data:font/woff;base64," + base64.b64encode(b).decode(), len(b)


def img_datauri(path: Path) -> tuple[str, int]:
    im = Image.open(path).convert("RGB")
    if im.width > 1024:
        im = im.resize((1024, round(im.height * 1024 / im.width)), Image.LANCZOS)
    buf = io.BytesIO()
    im.save(buf, "PNG", optimize=True)
    data = buf.getvalue()
    if len(data) > 95_000:                       # quantize heavy frames (dense text)
        q = im.quantize(colors=256, method=Image.MEDIANCUT, dither=Image.NONE)
        buf = io.BytesIO(); q.save(buf, "PNG", optimize=True); data = buf.getvalue()
    return "data:image/png;base64," + base64.b64encode(data).decode(), len(data)


def main() -> None:
    html = (HERE / "dossier.template.html").read_text()

    reg, rn = subset_woff(FONT_DIR / "JetBrainsMonoNerdFontMono-Regular.ttf")
    bold, bn = subset_woff(FONT_DIR / "JetBrainsMonoNerdFontMono-Bold.ttf")
    html = html.replace("%%FONT_REG%%", reg).replace("%%FONT_BOLD%%", bold)
    print(f"font  regular={rn//1024}KB  bold={bn//1024}KB")

    for token, path in IMAGES.items():
        if not path.is_file():
            raise SystemExit(f"missing image {path}")
        uri, n = img_datauri(path)
        html = html.replace(token, uri)
        print(f"img   {path.name:20s} {n//1024:4d}KB")

    left = [t for t in list(IMAGES) + ["%%FONT_REG%%", "%%FONT_BOLD%%"] if t in html]
    if left:
        raise SystemExit(f"unfilled tokens: {left}")

    OUT.write_text(html)
    print(f"\nwrote {OUT}  ({len(html.encode())//1024} KB total)")


if __name__ == "__main__":
    main()
