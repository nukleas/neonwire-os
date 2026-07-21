# CCM calibration plan — solve the SP2509 → sRGB color matrix

**Goal:** replace the placeholder saturation-boost in `camgrab.c`'s preview pipeline
with a real 3×3 **color-correction matrix (CCM)** fitted from a photographed color
target, so colors read accurately instead of washed/muted.

> **STATUS (2026-07-20 evening): PAUSED — no physical color target on hand.**
> Everything else is ready: `fit_ccm.py` is written and smoke-tested end-to-end
> (render/pick/solve/measure), the capture command is verified on-device (see
> corrected command below — the original in this doc was wrong), and geometry is
> confirmed. Resume = get a ColorChecker (user has no color printer either),
> light it with the daylight LED, aim via the shell Camera app, then follow
> the Workflow section. Also note: shoot in daylight-ish light only.

**Why this is the remaining lever:** WB is already correct (stock daylight gains
1.40/1.0/1.273, baked in). But the sensor's raw RGB primaries are *not* sRGB — the
color filters are wide and overlapping, so even perfectly white-balanced output looks
low-saturation and milky. The CCM maps sensor-space RGB into sRGB. Stock firmware
computes it dynamically (confirmed — no static matrix in `libcameracustom.so`), so we
must **measure our own** against a known color target. This is a one-time calibration:
the matrix is a property of the sensor + our WB point, not the scene.

---

## Current state (so we can start cold)

- **Device:** `ssh root@100.x.y.z` (dl7006-neonos, over Tailscale). No python on
  device (busybox). Binary lives at `/mnt/sd/linux-lab/camgrab`; the neonwire Camera
  app loops `camgrab --stream`. Deploy = `rm -f /mnt/sd/linux-lab/camgrab; cat > … ;
  chmod 755` (unlink+write, since the app holds the running inode).
- **Build:** `export PATH=$HOME/toolchains/armv7l-linux-musleabihf-cross/bin:$PATH`
  then `armv7l-linux-musleabihf-gcc -Os -static -no-pie -Wall -o camgrab camgrab.c`
  (from `experiments/camera/`). Static musl → **no libm** (no `pow`/`sqrt`; use integer
  math or LUTs, as the gamma code already does).
- **Capture (VERIFIED 2026-07-20):**
  `CAMGRAB_GRABW=1550 CAMGRAB_GAIN=64 ./camgrab --stock-regs /tmp/frame.raw 14 0`
  writes full-frame **1550×1194 RAW10, stride 1944** (`--stock-regs` is REQUIRED —
  bare camgrab takes the old RTBC path and truncates at 597/1194 lines; so does any
  width >1550, and the 1550 auto-cap only applies in `--preview` mode, hence the
  explicit `CAMGRAB_GRABW=1550`). Confirm `lines_hit~1194/1194 got_frame=1` on stderr.
  Drive exposure with `CAMGRAB_ESHUTTER` (gain stays 64 = 1×). MIPI packing: 4 px per
  5 bytes; BGGR/RAW_B Bayer. Pull with `ssh root@… cat /tmp/frame.raw > frame.raw`.
- **Aiming:** open the shell Camera app (live viewfinder); the host can also pull
  `/tmp/preview.rgb` ([u32 w][u32 h][RGB888]) over SSH to check framing remotely.
  Stop the Camera app before still captures (it holds camgrab in a stream loop).
- **Preview pipeline today** (`write_preview` in `camgrab.c`, ~line 773): 10-bit unpack
  + black-level → 2×2 Bayer-cell average demosaic → **WB in 10-bit linear** (daylight
  358/256/326 Q8, blended 70/30 with gray-world) → luma 3×3 denoise + chroma retention
  (the **1.25× sat-boost `SAT_BOOST` is the CCM stand-in — REMOVE it when the CCM lands**,
  or they compound) → gamma 0.5 (isqrt LUT). **The CCM slots in right after WB, in linear
  10-bit space, before denoise/gamma.**
- Host helper that already unpacks RAW10: `experiments/camera/debayer.py` (run from repo
  root; `--height`, not `--h`). Extend or copy it for the calibration tool.

---

## The math (least-squares 3×3)

We want `M` (3×3) such that, for each color patch, `linear_sRGB ≈ M · sensorRGB_wb`,
where both are **linear** (not gamma-encoded).

1. Photograph a target with N patches of *known* sRGB values (ColorChecker = 24).
2. For each patch, average the sensor RGB **after black-level + WB, before gamma** →
   `S` (N×3, our measured linear sensor color).
3. Convert each known patch's 8-bit sRGB to **linear** sRGB → `T` (N×3, the target).
   De-gamma: `c/255` then sRGB EOTF (`x≤0.04045 ? x/12.92 : ((x+0.055)/1.055)^2.4`).
4. Normalize exposure: scale `S` (or `T`) so the neutral/white patch matches — the CCM
   should not change overall brightness, only hue/saturation.
5. Solve `M = (Sᵀ S)⁻¹ Sᵀ T` (ordinary least squares, `numpy.linalg.lstsq` on host).
   Optionally constrain **each row of M to sum to 1** so a neutral input stays neutral
   (do this by fitting on WB'd data and letting the white patch anchor it, or solve with
   the row-sum constraint via Lagrange/substitution). Rows will be diagonal-dominant with
   small negative off-diagonals — that's the expected shape; if not, the patch sampling or
   exposure is off.
6. Quantize `M` to fixed point for the device: **Q10 (×1024)**, `int16`. Apply on-device
   as `out = (m0*R + m1*G + m2*B) >> 10`, clamp `[0,1023]`.

---

## Workflow

### A. Shoot the target
- Use even, **daylight-ish** lighting (window light or a daylight LED) to match the baked
  daylight WB. Fill the frame with the chart, roughly fronto-parallel, no glare/specular.
- **Expose so nothing clips:** the white patch must stay below ~1000/1023 (not saturated)
  and the black patch above the black level. Drive exposure via `CAMGRAB_ESHUTTER` /
  `CAMGRAB_GAIN` env (gain in 1/64 units) — prefer long shutter, **low gain** (gain adds
  noise that corrupts patch averages).
- Capture a still: `camgrab /tmp/frame.raw 14 0` (no `--preview`/`--stream` — we want the
  full RAW). Pull `frame.raw` to the host. Grab 2–3 frames to average out noise.

### B. Fit on host (python + numpy)
**DONE — `experiments/camera/fit_ccm.py` exists and is smoke-tested.** Usage:
```
python3 fit_ccm.py render  frame.raw --width 1550 -o chart.png   # framing check
python3 fit_ccm.py pick    frame.raw --width 1550                # click 4 corners
python3 fit_ccm.py solve   frame.raw [more.raw] --width 1550 --corners corners.json
python3 fit_ccm.py measure frame.raw --width 1550 --corners corners.json --ccm 9,q10,ints
```
It replicates the device pipeline bit-exactly (BLACK_LVL 16, BGGR 2×2 cell average,
70/30 daylight+gray-world WB in the same integer math) and does:
1. Unpacks RAW10 → 10-bit Bayer → demosaic → applies **the same black-level + WB** the
   device uses (so the fit matches the live pipeline). Keep it **linear** (no gamma).
2. Locates the 24 patch centers. **Manual is more robust than auto:** print the frame,
   have the user click/enter the 4 chart corners, then interpolate the 6×4 grid and sample
   a central ~20×20 px box per patch (median, to reject dust/noise).
3. Builds `S` (measured) and `T` (linear sRGB reference, table below), solves lstsq,
   prints: the float matrix, per-patch residual / mean ΔE, and the **Q10 int16 matrix**
   pasted as a C initializer.

### C. Bake into camgrab.c
- Insert the CCM multiply in `write_preview` **immediately after the WB loop** (currently
  ~line 848, the `R[i]*wr>>8` block), operating on the 10-bit `R/G/B[]` buffers in place:
  ```c
  /* SP2509 -> sRGB color-correction matrix (Q10), from color-target calibration.
   * See ccm-calibration-plan.md. Applied in linear light, after WB, before gamma. */
  static const int CCM[9] = { /* r_from_rgb, g_from_rgb, b_from_rgb */
      1229, -160,  -45,   /* example placeholder — REPLACE with fitted values */
      -178, 1306, -104,
       -20, -287, 1331 };
  for (size_t i = 0; i < npix; i++) {
      int r = R[i], g = G[i], b = B[i];
      int nr = (CCM[0]*r + CCM[1]*g + CCM[2]*b) >> 10;
      int ng = (CCM[3]*r + CCM[4]*g + CCM[5]*b) >> 10;
      int nb = (CCM[6]*r + CCM[7]*g + CCM[8]*b) >> 10;
      R[i] = nr<0?0:nr>1023?1023:nr;
      G[i] = ng<0?0:ng>1023?1023:ng;
      B[i] = nb<0?0:nb>1023?1023:nb;
  }
  ```
- **Neutralize the chroma stage** (drift note: `SAT_BOOST` no longer exists by that name —
  it's the `sat_cap` chroma-retention in the denoise block, currently capped at 220/256
  ≈ 0.86×). When the CCM lands, set the low-gain cap to 256 (1.0×) so the CCM isn't
  fought by desaturation; keep the gain-based rolloff + dark-chroma suppression
  (noise control, not color).
- Rebuild, redeploy, done.

### D. Validate
- Reshoot the same chart, run `fit_ccm.py` in "measure" mode (no solve) to report residual
  ΔE per patch. Target: mean ΔE < ~5 is good for a phone-class sensor. Iterate lighting /
  exposure if the grayscale patches drift (that's a WB/black-level issue, not CCM).

---

## ColorChecker Classic — reference sRGB (8-bit), for `T`

If no physical chart: a **printed** chart works but is limited by printer gamut (mark it
approximate); displaying these patches on a decent phone/monitor and shooting the screen is
a rough fallback (screens aren't colorimetric, and moiré/backlight will add error). A real
X-Rite/Calibrite ColorChecker is strongly preferred.

```
 1 dark skin    115  82  68     13 blue         56  61 150
 2 light skin   194 150 130     14 green        70 148  73
 3 blue sky      98 122 157     15 red         175  54  60
 4 foliage       87 108  67     16 yellow      231 199  31
 5 blue flower  133 128 177     17 magenta     187  86 149
 6 bluish green 103 189 170     18 cyan          8 133 161
 7 orange       214 126  44     19 white       243 243 242
 8 purplish blue 80  91 166     20 neutral 8   200 200 200
 9 moderate red 193  90  99     21 neutral 6.5 160 160 160
10 purple        94  60 108     22 neutral 5   122 122 121
11 yellow green 157 188  64     23 neutral 3.5  85  85  85
12 orange yellow224 163  46     24 black        52  52  52
```
(Widely-cited sRGB values; if the physical chart has an official data sheet, prefer its
numbers.) The 6 neutral patches (19–24) double as a WB / black-level / gamma sanity check.

---

## Gotchas
- **Work in linear light.** CCM before gamma. Our WB is already linear-domain — insert CCM
  right after it. Never fit or apply on gamma-encoded values.
- **Don't clip.** A saturated white patch or crushed black poisons the fit. Meter the chart.
- **Low gain.** Noise in patch averages tilts the matrix. Long shutter, gain ≈ 1×.
- **Match pipelines.** The host fit must use the identical black-level + WB constants as the
  device, or the on-device result won't match the fit.
- **One matrix per WB/illuminant.** This CCM is paired with the daylight WB preset. If we
  later add multi-illuminant AWB, each illuminant ideally gets its own CCM (stock has 7).
- **Sign check.** Expect diagonal-dominant rows summing ≈1 with small negative off-diagonals.
  All-positive or wild values = bad sampling/exposure.

## Deliverables when done
1. `experiments/camera/fit_ccm.py` (host tool).
2. Fitted Q10 matrix baked into `write_preview`, `SAT_BOOST` neutralized.
3. A before/after shot of the chart + mean ΔE in the README color section.
