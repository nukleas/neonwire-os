#!/usr/bin/env python3
"""Unpack MTK RAW10 + debayer a captured SP2509 frame to PNG (host-side).

The SP2509 outputs 1600x1200 BGGR Bayer, 10-bit. camgrab dumps whatever the ISP
IMGO DMA wrote; two packings are common, so both are supported:

  --packing mipi   MIPI CSI-2 RAW10: 4 px in 5 bytes (4 high bytes + 1 low-bits byte)
  --packing u16    already unpacked to 16-bit little-endian per pixel (ISP FMT_UFEO etc)

Stride padding (bytes per row, if the DMA rounds up) via --stride.
No 3A / white balance — expect a green, dark image. That IS success for a first grab.

  python3 debayer.py frame.raw --width 1600 --height 1200 --packing mipi -o frame.png
"""
from __future__ import annotations

import argparse
import numpy as np


def unpack_mipi_raw10(buf: bytes, width: int, height: int, stride: int) -> np.ndarray:
    """MIPI RAW10: every 5 bytes = 4 pixels. byte4 packs the 4 LSB-pairs."""
    row_px_bytes = width * 5 // 4
    if stride == 0:
        stride = row_px_bytes
    out = np.empty((height, width), dtype=np.uint16)
    data = np.frombuffer(buf, dtype=np.uint8)
    for y in range(height):
        row = data[y * stride : y * stride + row_px_bytes]
        g = row[: row_px_bytes // 5 * 5].reshape(-1, 5).astype(np.uint16)
        hi = g[:, 0:4] << 2
        lo = g[:, 4]
        px = np.empty((g.shape[0], 4), dtype=np.uint16)
        px[:, 0] = hi[:, 0] | (lo & 0x3)
        px[:, 1] = hi[:, 1] | ((lo >> 2) & 0x3)
        px[:, 2] = hi[:, 2] | ((lo >> 4) & 0x3)
        px[:, 3] = hi[:, 3] | ((lo >> 6) & 0x3)
        out[y, : px.size] = px.reshape(-1)[:width]
    return out


def unpack_u16(buf: bytes, width: int, height: int, stride: int) -> np.ndarray:
    if stride == 0:
        stride = width * 2
    data = np.frombuffer(buf, dtype=np.uint8)
    out = np.empty((height, width), dtype=np.uint16)
    for y in range(height):
        row = data[y * stride : y * stride + width * 2]
        out[y] = row.view("<u2")[:width]
    return out


def debayer_bggr(raw: np.ndarray) -> np.ndarray:
    """Minimal bilinear debayer, BGGR order. raw is 10-bit (0..1023)."""
    h, w = raw.shape
    r = np.zeros((h, w), np.float32)
    g = np.zeros((h, w), np.float32)
    b = np.zeros((h, w), np.float32)
    f = raw.astype(np.float32)
    # BGGR: (0,0)=B (0,1)=G (1,0)=G (1,1)=R
    b[0::2, 0::2] = f[0::2, 0::2]
    g[0::2, 1::2] = f[0::2, 1::2]
    g[1::2, 0::2] = f[1::2, 0::2]
    r[1::2, 1::2] = f[1::2, 1::2]

    def fill(ch):  # cheap 3x3 mean over known samples
        from scipy.ndimage import uniform_filter  # optional; fallback below

        mask = (ch > 0).astype(np.float32)
        num = uniform_filter(ch, 3, mode="nearest")
        den = uniform_filter(mask, 3, mode="nearest")
        out = np.where(ch > 0, ch, num / np.maximum(den, 1e-6))
        return out

    try:
        r, g, b = fill(r), fill(g), fill(b)
    except ImportError:
        # scipy-free fallback: shift-and-average (coarser but no deps)
        def sh(a, dy, dx):
            return np.roll(np.roll(a, dy, 0), dx, 1)

        for ch in ("r", "b"):
            a = {"r": r, "b": b}[ch]
            a[:] = np.maximum(a, (sh(a, 0, 1) + sh(a, 0, -1) + sh(a, 1, 0) + sh(a, -1, 0)) / 4)
        g[:] = np.maximum(g, (sh(g, 0, 1) + sh(g, 0, -1) + sh(g, 1, 0) + sh(g, -1, 0)) / 4)

    rgb = np.stack([r, g, b], axis=-1)
    rgb = np.clip(rgb / 1023.0 * 255.0, 0, 255).astype(np.uint8)
    return rgb


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("raw")
    ap.add_argument("--width", type=int, default=1600)
    ap.add_argument("--height", type=int, default=1200)
    ap.add_argument("--stride", type=int, default=0, help="bytes/row (0=tight)")
    ap.add_argument("--packing", choices=["mipi", "u16"], default="mipi")
    ap.add_argument("-o", "--out", default="frame.png")
    args = ap.parse_args()

    buf = open(args.raw, "rb").read()
    if args.packing == "mipi":
        raw = unpack_mipi_raw10(buf, args.width, args.height, args.stride)
    else:
        raw = unpack_u16(buf, args.width, args.height, args.stride)
    print(f"raw {raw.shape} min={raw.min()} max={raw.max()} mean={raw.mean():.1f}")
    rgb = debayer_bggr(raw)
    from PIL import Image

    Image.fromarray(rgb, "RGB").save(args.out)
    print(f"wrote {args.out}")


if __name__ == "__main__":
    main()
