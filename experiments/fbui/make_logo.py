#!/usr/bin/env python3
"""Render a NEONWIRE boot splash and rebuild the MTK logo.bin for the DL7006.

The DIGILAND boot logo lives in the `logo` partition (@0x4400000) as an MTK
logo.bin: a 512-byte MTK header + { u32 nums, u32 blocksize, u32 offset[nums] }
then zlib-compressed 1024x600 32bpp BGRA images. logo[0] and logo[38] are the
identical power-on splash; we replace those two and keep every other blob
byte-identical (battery/charging frames).

  ./make_logo.py --preview logo_preview.png          # just render the splash
  ./make_logo.py --build out/logo-neonos.bin         # render + rebuild logo.bin
"""
from __future__ import annotations

import argparse
import struct
import zlib
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter, ImageFont

ROOT = Path(__file__).resolve().parents[2]
LOGO_SRC = ROOT / "reference/dumps/session-20260718/images/logo.bin"
FONT = "/usr/share/fonts/TTF/JetBrainsMonoNerdFontMono-Bold.ttf"
W, H = 1024, 600
REPLACE = (0, 38)

# cyberdesign tokens
BG, BG2 = (5, 6, 10), (7, 10, 18)
CYAN, CYANHI = (71, 246, 255), (189, 255, 255)
MAGENTA = (255, 43, 214)
GREEN = (82, 255, 159)
TEXT2 = (167, 183, 214)
TEXTDIM = (106, 98, 92)
GRID = (71, 246, 255)


def font(sz):
    return ImageFont.truetype(FONT, sz)


def ctext(d, cx, y, s, f, fill):
    w = d.textbbox((0, 0), s, font=f)[2]
    d.text((cx - w / 2, y), s, font=f, fill=fill)
    return w


def render() -> Image.Image:
    img = Image.new("RGB", (W, H), BG)
    px = img.load()
    # vertical gradient
    for y in range(H):
        t = y / H
        c = tuple(int(BG[i] + (BG2[i] - BG[i]) * t) for i in range(3))
        for x in range(W):
            px[x, y] = c
    d = ImageDraw.Draw(img)

    # NB: no pixel-grid / scanline texture here — high-frequency noise wrecks zlib
    # and the rebuilt logo.bin must stay <= the original partition size. The live
    # neui UI carries the grid+scanlines; the ~2s boot splash stays clean.

    # glow layer (blurred), then crisp text on top
    glow = Image.new("RGB", (W, H), (0, 0, 0))
    gd = ImageDraw.Draw(glow)
    fbig, fmid, fsub, fsmall = font(120), font(40), font(26), font(22)
    cx = W // 2
    ctext(gd, cx, 150, "NEONWIRE", fbig, CYAN)
    ctext(gd, cx, 300, "// DL-7006 //", fmid, MAGENTA)
    glow = glow.filter(ImageFilter.GaussianBlur(10))
    img = Image.blend(img, Image.composite(glow, img, glow.convert("L").point(lambda v: min(255, v * 2))), 1.0)
    d = ImageDraw.Draw(img)

    # crisp text
    ctext(d, cx, 150, "NEONWIRE", fbig, CYANHI)
    ctext(d, cx, 300, "// DL-7006 //", fmid, MAGENTA)
    ctext(d, cx, 360, "cyberpunk shell   ::   self-built linux", fsub, TEXT2)

    # neon rule
    d.line([(cx - 300, 405), (cx + 300, 405)], fill=CYAN, width=2)
    ctext(d, cx, 470, "MEDIATEK MT8127   ::   KERNEL 3.18.35", fsmall, TEXTDIM)
    ctext(d, cx, 505, "> booting neon subsystems ...", fsmall, GREEN)

    # outer frame + corner brackets
    m, L = 20, 34
    d.rectangle([m, m, W - m - 1, H - m - 1], outline=CYAN, width=1)
    for (ox, oy, dx, dy) in [(m, m, 1, 1), (W - m - 1, m, -1, 1),
                             (m, H - m - 1, 1, -1), (W - m - 1, H - m - 1, -1, -1)]:
        d.line([(ox, oy), (ox + dx * L, oy)], fill=CYANHI, width=3)
        d.line([(ox, oy), (ox, oy + dy * L)], fill=CYANHI, width=3)

    return img


def rebuild(new_img: Image.Image, out_path: Path) -> None:
    bgra = new_img.convert("RGBA").tobytes("raw", "BGRA")
    assert len(bgra) == W * H * 4, len(bgra)
    new_blob = zlib.compress(bgra, 9)

    data = LOGO_SRC.read_bytes()
    hdr512, body = data[:512], data[512:]
    nums, blocksize = struct.unpack_from("<II", body, 0)
    offs = list(struct.unpack_from("<%dI" % nums, body, 8))

    blobs = []
    for i in range(nums):
        end = offs[i + 1] if i + 1 < nums else blocksize
        blobs.append(new_blob if i in REPLACE else body[offs[i]:end])

    head = 8 + 4 * nums
    new_offs, run = [], head
    for b in blobs:
        new_offs.append(run)
        run += len(b)
    new_blocksize = run
    new_body = struct.pack("<II", nums, new_blocksize) + struct.pack("<%dI" % nums, *new_offs) + b"".join(blobs)
    new_hdr = bytearray(hdr512)
    struct.pack_into("<I", new_hdr, 4, new_blocksize)   # MTK header size field

    out = bytes(new_hdr) + new_body
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_bytes(out)
    print(f"wrote {out_path} ({len(out)} bytes; orig {len(data)})")
    print(f"  replaced logos {REPLACE}: new blob {len(new_blob)}B (orig 73709B)")
    print(f"  new_blocksize={new_blocksize}  fits_partition={len(out) <= len(data)}")

    # self-verify: re-parse and decompress the replaced blob
    b2 = out[512:]
    n2, bs2 = struct.unpack_from("<II", b2, 0)
    o2 = list(struct.unpack_from("<%dI" % n2, b2, 8))
    chk = zlib.decompress(b2[o2[0]:o2[1]])
    assert len(chk) == W * H * 4 and n2 == nums, "verify failed"
    print("  self-verify OK (re-parsed, decompressed logo[0])")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--preview", type=Path)
    ap.add_argument("--build", type=Path)
    args = ap.parse_args()
    img = render()
    if args.preview:
        img.save(args.preview)
        print(f"preview -> {args.preview}")
    if args.build:
        rebuild(img, args.build)
    if not args.preview and not args.build:
        ap.error("give --preview and/or --build")


if __name__ == "__main__":
    main()
