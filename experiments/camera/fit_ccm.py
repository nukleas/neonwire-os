#!/usr/bin/env python3
"""Fit an SP2509 -> sRGB color-correction matrix from a ColorChecker shot.

Companion to ccm-calibration-plan.md. Replicates camgrab.c's write_preview
pipeline exactly (black level, BGGR 2x2 cell average, 70/30 daylight+gray-world
WB in integer math) so the fitted matrix drops straight into the device code.

Modes:
  render   frame.raw -o chart.png          # gamma preview for corner picking
  pick     frame.raw                       # click 4 corners (TL TR BR BL of the
                                           # 24-patch grid, TL = dark-skin end),
                                           # writes corners.json
  solve    frame.raw [frame2.raw ...] --corners corners.json
                                           # fit CCM, print Q10 C initializer,
                                           # write diag swatch PNG
  measure  frame.raw --corners corners.json [--ccm a,b,c,...(9 Q10 ints)]
                                           # report deltaE only (validation)

Frames are full-still RAW10 dumps: camgrab /tmp/frame.raw 14 0
Default geometry 1592x1194 stride 1992 (camgrab prints the real one on stderr).
"""
from __future__ import annotations

import argparse
import json
import sys

import numpy as np

# ---------------------------------------------------------------- constants
BLACK_LVL = 16  # 10-bit units, must match camgrab.c
WB_DAYLIGHT = (358, 256, 326)  # Q8, must match camgrab.c

# ColorChecker Classic, sRGB 8-bit, patch 1..24 row-major (6 cols x 4 rows)
CHECKER_SRGB = np.array([
    [115, 82, 68], [194, 150, 130], [98, 122, 157], [87, 108, 67],
    [133, 128, 177], [103, 189, 170],
    [214, 126, 44], [80, 91, 166], [193, 90, 99], [94, 60, 108],
    [157, 188, 64], [224, 163, 46],
    [56, 61, 150], [70, 148, 73], [175, 54, 60], [231, 199, 31],
    [187, 86, 149], [8, 133, 161],
    [243, 243, 242], [200, 200, 200], [160, 160, 160], [122, 122, 121],
    [85, 85, 85], [52, 52, 52],
], dtype=np.float64)
PATCH_NAMES = [
    "dark skin", "light skin", "blue sky", "foliage", "blue flower",
    "bluish green", "orange", "purplish blue", "moderate red", "purple",
    "yellow green", "orange yellow", "blue", "green", "red", "yellow",
    "magenta", "cyan", "white", "neutral 8", "neutral 6.5", "neutral 5",
    "neutral 3.5", "black",
]
WHITE_IDX = 18  # patch 19


# ---------------------------------------------------------------- raw pipeline
def unpack_mipi_raw10(buf: bytes, width: int, height: int, stride: int) -> np.ndarray:
    """MIPI RAW10: every 5 bytes = 4 pixels (4 high bytes + LSB-pairs byte)."""
    groups = (width + 3) // 4
    data = np.frombuffer(buf, dtype=np.uint8)
    rows = data[: height * stride].reshape(height, stride)
    g = rows[:, : groups * 5].reshape(height, groups, 5).astype(np.uint16)
    px = np.empty((height, groups, 4), dtype=np.uint16)
    lo = g[:, :, 4]
    for i in range(4):
        px[:, :, i] = (g[:, :, i] << 2) | ((lo >> (2 * i)) & 3)
    return px.reshape(height, groups * 4)[:, :width]


def cell_demosaic(raw: np.ndarray) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
    """BGGR 2x2 cell average with black-level subtract, matching camgrab.
    Returns half-res R,G,B int32 arrays in 10-bit range."""
    h, w = raw.shape
    h &= ~1
    w &= ~1
    f = raw[:h, :w].astype(np.int32)
    sub = lambda a: np.maximum(a - BLACK_LVL, 0)
    B = sub(f[0::2, 0::2])
    G = (sub(f[0::2, 1::2]) + sub(f[1::2, 0::2])) // 2
    R = sub(f[1::2, 1::2])
    return R, G, B


def device_wb(R: np.ndarray, G: np.ndarray, B: np.ndarray):
    """The exact integer WB from write_preview: 70% daylight, 30% gray-world."""
    mr = max(int(R.mean()), 1)
    mg = max(int(G.mean()), 1)
    mb = max(int(B.mean()), 1)
    wr, wg, wb = WB_DAYLIGHT
    wr_g = min(max(mg * 256 // mr, 160), 480)
    wb_g = min(max(mg * 256 // mb, 160), 480)
    wr = (wr * 7 + wr_g * 3) // 10
    wb = (wb * 7 + wb_g * 3) // 10
    Rw = np.minimum(R * wr >> 8, 1023)
    Gw = np.minimum(G * wg >> 8, 1023)
    Bw = np.minimum(B * wb >> 8, 1023)
    return (Rw, Gw, Bw), (wr, wg, wb)


def load_frame(path: str, width: int, height: int, stride: int):
    buf = open(path, "rb").read()
    need = height * stride
    if len(buf) < need:
        sys.exit(f"{path}: {len(buf)} bytes < {need} for {width}x{height}@{stride}")
    raw = unpack_mipi_raw10(buf, width, height, stride)
    print(f"{path}: raw 10-bit min={raw.min()} max={raw.max()} mean={raw.mean():.1f}")
    return raw


# ---------------------------------------------------------------- color math
def srgb_to_linear(c8: np.ndarray) -> np.ndarray:
    x = c8 / 255.0
    return np.where(x <= 0.04045, x / 12.92, ((x + 0.055) / 1.055) ** 2.4)


def linear_to_srgb(x: np.ndarray) -> np.ndarray:
    x = np.clip(x, 0.0, 1.0)
    return np.where(x <= 0.0031308, x * 12.92, 1.055 * x ** (1 / 2.4) - 0.055)


def linear_to_lab(rgb: np.ndarray) -> np.ndarray:
    """linear sRGB (D65) -> CIELAB. rgb shape (...,3), 0..1."""
    M = np.array([[0.4124564, 0.3575761, 0.1804375],
                  [0.2126729, 0.7151522, 0.0721750],
                  [0.0193339, 0.1191920, 0.9503041]])
    xyz = rgb @ M.T
    wp = np.array([0.95047, 1.0, 1.08883])
    t = xyz / wp
    f = np.where(t > (6 / 29) ** 3, np.cbrt(t), t / (3 * (6 / 29) ** 2) + 4 / 29)
    L = 116 * f[..., 1] - 16
    a = 500 * (f[..., 0] - f[..., 1])
    b = 200 * (f[..., 1] - f[..., 2])
    return np.stack([L, a, b], axis=-1)


def delta_e(lin_a: np.ndarray, lin_b: np.ndarray) -> np.ndarray:
    return np.linalg.norm(linear_to_lab(lin_a) - linear_to_lab(lin_b), axis=-1)


# ---------------------------------------------------------------- geometry
def homography_from_unit_square(corners: np.ndarray) -> np.ndarray:
    """3x3 H mapping unit square (0,0)(1,0)(1,1)(0,1) -> corners TL TR BR BL."""
    src = np.array([(0, 0), (1, 0), (1, 1), (0, 1)], dtype=np.float64)
    A, b = [], []
    for (u, v), (x, y) in zip(src, corners):
        A.append([u, v, 1, 0, 0, 0, -u * x, -v * x])
        b.append(x)
        A.append([0, 0, 0, u, v, 1, -u * y, -v * y])
        b.append(y)
    h = np.linalg.solve(np.array(A), np.array(b))
    return np.array([[h[0], h[1], h[2]], [h[3], h[4], h[5]], [h[6], h[7], 1.0]])


def patch_uv_centers() -> np.ndarray:
    """24 patch centers in chart uv space, row-major 6x4."""
    uv = [((c + 0.5) / 6, (r + 0.5) / 4) for r in range(4) for c in range(6)]
    return np.array(uv)


def sample_patches(R, G, B, corners, box=0.30, grid=15):
    """Median-sample each patch. corners are in HALF-RES pixel coords.
    Returns S (24x3 float) and raw sample stats for clip checks."""
    H = homography_from_unit_square(np.asarray(corners, dtype=np.float64))
    h, w = R.shape
    S = np.zeros((24, 3))
    stats = []
    du, dv = box / 6 / 2, box / 4 / 2  # half-extent of sample box in uv
    for p, (cu, cv) in enumerate(patch_uv_centers()):
        us = np.linspace(cu - du, cu + du, grid)
        vs = np.linspace(cv - dv, cv + dv, grid)
        uu, vv = np.meshgrid(us, vs)
        pts = np.stack([uu.ravel(), vv.ravel(), np.ones(uu.size)])
        xy = H @ pts
        xs = np.clip(np.round(xy[0] / xy[2]).astype(int), 0, w - 1)
        ys = np.clip(np.round(xy[1] / xy[2]).astype(int), 0, h - 1)
        r = np.median(R[ys, xs])
        g = np.median(G[ys, xs])
        b = np.median(B[ys, xs])
        S[p] = (r, g, b)
        stats.append((float(max(R[ys, xs].max(), G[ys, xs].max(), B[ys, xs].max())),
                      float(min(R[ys, xs].min(), G[ys, xs].min(), B[ys, xs].min()))))
    return S, stats


# ---------------------------------------------------------------- rendering
def render_rgb(R, G, B, gamma=True) -> np.ndarray:
    """10-bit linear -> uint8 preview, device-ish gamma (0.5*lin + 0.5*sqrt)."""
    rgb = np.stack([R, G, B], axis=-1).astype(np.float64) / 1023.0
    if gamma:
        rgb = 0.5 * rgb + 0.5 * np.sqrt(rgb)
    return (np.clip(rgb, 0, 1) * 255).astype(np.uint8)


def write_diag(path, img8, corners, S_lin, corrected_lin, target_srgb8):
    """Annotated PNG: chart with patch boxes + measured/corrected/reference strip."""
    from PIL import Image, ImageDraw

    im = Image.fromarray(img8, "RGB")
    dr = ImageDraw.Draw(im)
    H = homography_from_unit_square(np.asarray(corners, dtype=np.float64))
    for p, (cu, cv) in enumerate(patch_uv_centers()):
        du, dv = 0.30 / 12, 0.30 / 8
        quad = []
        for u, v in [(cu - du, cv - dv), (cu + du, cv - dv),
                     (cu + du, cv + dv), (cu - du, cv + dv)]:
            x, y, z = H @ np.array([u, v, 1.0])
            quad.append((x / z, y / z))
        dr.polygon(quad, outline=(255, 60, 60))
        dr.text((quad[0][0], quad[0][1] - 12), str(p + 1), fill=(255, 60, 60))

    # swatch strip: rows = measured (WB only), CCM-corrected, reference
    sw, sh = 40, 40
    strip = Image.new("RGB", (sw * 24, sh * 3))
    ds = ImageDraw.Draw(strip)
    meas8 = (linear_to_srgb(S_lin) * 255).astype(np.uint8)
    corr8 = (linear_to_srgb(corrected_lin) * 255).astype(np.uint8)
    for p in range(24):
        ds.rectangle([p * sw, 0, (p + 1) * sw, sh], fill=tuple(meas8[p]))
        ds.rectangle([p * sw, sh, (p + 1) * sw, 2 * sh], fill=tuple(corr8[p]))
        ds.rectangle([p * sw, 2 * sh, (p + 1) * sw, 3 * sh],
                     fill=tuple(target_srgb8[p].astype(np.uint8)))
    out = Image.new("RGB", (max(im.width, strip.width), im.height + strip.height + 8),
                    (20, 20, 20))
    out.paste(im, (0, 0))
    out.paste(strip, (0, im.height + 8))
    out.save(path)
    print(f"wrote {path} (strip rows: measured / CCM-corrected / reference)")


# ---------------------------------------------------------------- fitting
def build_S(paths, width, height, stride, corners):
    """Average the WB'd linear patch samples over one or more frames."""
    S_acc = np.zeros((24, 3))
    wb_used = None
    for p in paths:
        raw = load_frame(p, width, height, stride)
        R, G, B = cell_demosaic(raw)
        (Rw, Gw, Bw), wb = device_wb(R, G, B)
        wb_used = wb
        S, stats = sample_patches(Rw, Gw, Bw, corners)
        for i, (mx, mn) in enumerate(stats):
            if mx >= 1015:
                print(f"  WARN patch {i+1} ({PATCH_NAMES[i]}) near clip (max {mx:.0f})")
        if S[WHITE_IDX].max() >= 1000:
            print("  WARN white patch >=1000 — reduce exposure, fit will be poor")
        S_acc += S
        last = (Rw, Gw, Bw)
    S = S_acc / len(paths)
    print(f"device WB used: wr={wb_used[0]} wg={wb_used[1]} wb={wb_used[2]}")
    return S, last


def solve_ccm(S_lin: np.ndarray, T_lin: np.ndarray, rowsum1: bool):
    """Least squares M with T ~ S @ M.T; optional row-sum==1 constraint."""
    if not rowsum1:
        Mt, *_ = np.linalg.lstsq(S_lin, T_lin, rcond=None)
        return Mt.T
    # substitute c = 1 - a - b per row: t - s_b = a(s_r - s_b) + b(s_g - s_b)
    M = np.zeros((3, 3))
    X = np.stack([S_lin[:, 0] - S_lin[:, 2], S_lin[:, 1] - S_lin[:, 2]], axis=1)
    for row in range(3):
        y = T_lin[:, row] - S_lin[:, 2]
        ab, *_ = np.linalg.lstsq(X, y, rcond=None)
        M[row] = (ab[0], ab[1], 1.0 - ab[0] - ab[1])
    return M


def report(S_lin, T_lin, M, label):
    corr = S_lin @ M.T
    de = delta_e(np.clip(corr, 0, 1), T_lin)
    print(f"\n{label}: per-patch deltaE76")
    for p in range(24):
        flag = " <-- worst" if de[p] == de.max() else ""
        print(f"  {p+1:2d} {PATCH_NAMES[p]:13s} dE={de[p]:6.2f}{flag}")
    print(f"  mean dE={de.mean():.2f}  max dE={de.max():.2f}  "
          f"(neutrals mean dE={de[18:].mean():.2f})")
    return corr, de


def print_c_matrix(M):
    Q = np.round(M * 1024).astype(int)
    print("\nQ10 int matrix for camgrab.c:")
    print("static const int CCM[9] = {")
    for r in range(3):
        print(f"    {Q[r,0]:5d}, {Q[r,1]:5d}, {Q[r,2]:5d},"
              f"   /* row sum {Q[r].sum()} */")
    print("};")
    print("row sums (float):", [f"{s:.3f}" for s in M.sum(axis=1)])
    return Q


# ---------------------------------------------------------------- modes
def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("mode", choices=["render", "pick", "solve", "measure"])
    ap.add_argument("raw", nargs="+")
    ap.add_argument("--width", type=int, default=1592)
    ap.add_argument("--height", type=int, default=1194)
    ap.add_argument("--stride", type=int, default=0, help="bytes/row (0 = auto)")
    ap.add_argument("--corners", default="corners.json",
                    help="json file with 4 [x,y] half-res corners TL TR BR BL")
    ap.add_argument("--ccm", default=None,
                    help="9 comma-separated Q10 ints to evaluate (measure mode)")
    ap.add_argument("--no-rowsum1", action="store_true",
                    help="unconstrained least squares (default constrains rows to sum 1)")
    ap.add_argument("-o", "--out", default=None)
    args = ap.parse_args()

    stride = args.stride or ((args.width * 10 + 7) // 8 + 7) & ~7

    if args.mode == "render":
        raw = load_frame(args.raw[0], args.width, args.height, stride)
        R, G, B = cell_demosaic(raw)
        (Rw, Gw, Bw), wb = device_wb(R, G, B)
        img = render_rgb(Rw, Gw, Bw)
        from PIL import Image
        out = args.out or "chart-preview.png"
        Image.fromarray(img, "RGB").save(out)
        print(f"wrote {out} ({img.shape[1]}x{img.shape[0]} half-res, wb r={wb[0]} b={wb[2]})")
        return

    if args.mode == "pick":
        raw = load_frame(args.raw[0], args.width, args.height, stride)
        R, G, B = cell_demosaic(raw)
        (Rw, Gw, Bw), _ = device_wb(R, G, B)
        img = render_rgb(Rw, Gw, Bw)
        import matplotlib.pyplot as plt
        fig, axp = plt.subplots(figsize=(12, 9))
        axp.imshow(img)
        axp.set_title("Click 4 chart corners: TL (dark-skin end), TR, BR, BL (white end)")
        pts = plt.ginput(4, timeout=0)
        plt.close(fig)
        if len(pts) != 4:
            sys.exit("need exactly 4 clicks")
        json.dump([[float(x), float(y)] for x, y in pts], open(args.corners, "w"))
        print(f"wrote {args.corners}: {pts}")
        return

    corners = json.load(open(args.corners))
    T_lin = srgb_to_linear(CHECKER_SRGB)

    S, (Rw, Gw, Bw) = build_S(args.raw, args.width, args.height, stride, corners)
    # normalize exposure: white patch G defines the S scale
    S_lin = S / 1023.0
    scale = T_lin[WHITE_IDX, 1] / S_lin[WHITE_IDX, 1]
    S_lin = S_lin * scale
    print(f"exposure scale (white-patch anchor): {scale:.4f}  "
          f"white S(wb'd, 10-bit)={S[WHITE_IDX].round(1)}")
    print("neutral patch S ratios r/g,b/g:",
          [(f"{S[i,0]/S[i,1]:.3f}", f"{S[i,2]/S[i,1]:.3f}") for i in range(18, 24)])

    if args.mode == "measure":
        if args.ccm:
            Q = np.array([int(v) for v in args.ccm.split(",")]).reshape(3, 3)
            M = Q / 1024.0
            label = "measure (given CCM applied)"
        else:
            M = np.eye(3)
            label = "measure (identity — WB-only residual)"
        report(S_lin, T_lin, M, label)
        return

    # solve
    M = solve_ccm(S_lin, T_lin, rowsum1=not args.no_rowsum1)
    print("\nfitted M (float):")
    print(np.array_str(M, precision=4, suppress_small=True))
    corr, de = report(S_lin, T_lin, M, "fit")
    _, de0 = report(S_lin, T_lin, np.eye(3), "baseline (no CCM)")
    print(f"\nimprovement: mean dE {de0.mean():.2f} -> {de.mean():.2f}")
    Q = print_c_matrix(M)

    img = render_rgb(Rw, Gw, Bw)
    write_diag(args.out or "ccm-diag.png", img, corners, np.clip(S_lin, 0, 1),
               np.clip(corr, 0, 1), CHECKER_SRGB)


if __name__ == "__main__":
    main()
