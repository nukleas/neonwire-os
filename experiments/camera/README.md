# Camera bring-up — SP2509 (plan track A)

## Status: ★ LIVE PREVIEW on L1 (2026-07-20) — real photos + viewfinder in the OS

**Milestones (all on-device, no vendor HAL):**
- **Real exposed photo** — added the sensor feature-control ioctl `IOC_X_FEATURE`
  (`KDIMGSENSORIOC_X_FEATURECONCTROL`, cmd 15): `SENSOR_FEATURE_SET_ESHUTTER`(3004) +
  `SENSOR_FEATURE_SET_GAIN`(3006). Preview `cfg.Shutter` is IGNORED by the sensor;
  gain/eshutter MUST go through this path (env: `CAMGRAB_GAIN` 1/64 units, `CAMGRAB_ESHUTTER`).
  16x gain → mean ~110-170, gray-world WB → recognizable scene. `photo-first-real-wb.png`.
- **Truncation confirmed** (flashlight test): the half-frame is the TOP 597 lines of a real
  full-height frame (bloom cut off hard at line 597), NOT a vertical squish. Deterministic
  exactly h/2 + `INTX=0x40000000` (DMA_ERR bit30) = half-throughput DMA overrun.
- **camgrab `--preview`** — cheap high-byte debayer (BGGR) + box-downscale to 480xN +
  gray-world WB → `/tmp/preview.rgb` (`[u32 w][u32 h][RGB888]`, atomic rename).
- **Live viewfinder in neonwire** — Camera app (`apps/camera.rs`) loops `camgrab --preview`,
  blits `/tmp/preview.rgb` via `Canvas::blit_rgb` (added to neon-gfx). ~1 fps refreshing
  snapshot, tap status to pause/resume. VERIFIED live on the device fb.

**★ HALF-FRAME FIXED (2026-07-20):** it was a DMA drain-rate overrun — L1 memory can't
drain a full 1592-wide RAW10 line fast enough, so the IMGO FIFO overflows at exactly h/2
(INTX DMA_ERR). Empirical cliff: grab width **≤1550 → full 1194 height**, 1592 → 597.
camgrab `--preview` now caps grab at 1550 (center-cropped, −2.6% FOV) → full-frame live
preview verified on device (FR 356+). Env `CAMGRAB_GRABW` overrides. The "proper" fix
(full 1592 width) needs more IMGO DMA bandwidth (EMI/DRAM clock or SMI larb BW — the
EMI+0x120 TG throttle via ISP_RESET did NOT help) or added sensor horizontal blanking.

**★ AUTO-EXPOSURE + DESTRIPE (2026-07-20):** camgrab `--preview` now:
- **Auto-exposure** across invocations via /tmp/camgrab_exp state: clipped-mean meter
  (ignore <8/>248), target 100, **deadband [85,118]** + tight damping [0.77,1.30] +
  prefer long shutter over gain (SMAX 6000 lines) = low noise, no hunting. Seed 2000/4x.
- **Settle** usleep(600ms) after VF so the just-set exposure applies before grab (killed
  the frame-to-frame brightness wobble from one-shot sensor re-init).
- **Box-average debayer** (fixed 2x2-cell/4x4px, even-aligned) — killed the vertical
  stripes (the 1-vs-2-cell alternation from the 3.23x downscale) AND denoises.
- **Display gamma 0.5** (isqrt LUT, no libm) — lifts shadows so dim scenes stay visible.
Result verified on device: smooth, stable, full-frame, visible in dim light. FR 1267+.

**Color (2026-07-20):** the purple/green speckle was CHROMA NOISE (16x gain, amplified by
the shadow-lift gamma). Added luma-based chroma suppression in write_preview (pull dark
pixels toward gray, sat ramps over Y 0..48) → clean neutral image, real color kept in lit
areas. Still hand-coded — on-device raw libs (libraw/OpenCV) aren't cross-compilable for
bare-metal armv7-musl.

**★ Real WB baked in; CCM is dynamic (2026-07-20):** disassembled `impGetDefaultData<9481>`
in `cam-extract/.../libcameracustom.so` (SP2509 = sensor id 9481=0x2509). The **AWB gains
extract cleanly** — the 3A NVRAM illuminant table (@file-off 0x24e7c, unity 512, G pinned
1.0) has 7 sources; the daylight/D65 entry is **R=717 G=512 B=652 → 1.40 / 1.0 / 1.273**.
Baked into `write_preview` as fixed Q8 gains (wr=358 wg=256 wb=326), **replacing gray-world**
so real colors survive instead of being averaged to gray. **The CCM does NOT exist as static
data** — stock computes it at runtime (`is_to_invoke_dynamic_ccm`, `refine_CCM` is a no-op
stub); an exhaustive scan of the 2.6 MB blob found no 9-coefficient unity-summing matrix in
any fixed-point/float form. Stand-in = a modest 1.25× saturation boost (`SAT_BOOST 320`)
folded into the chroma curve. Current pipeline: box-debayer → **fixed calibrated WB** →
sat-boost/chroma-suppress → gamma0.5.

**★ STREAM + AE FIXED (2026-07-20):** root causes of "slow / meh / distorted / poor color":
1. **Camera app forced `CAMGRAB_GAIN=1024` (16x)** — overrode AE → permanent high-ISO noise
   and wrong color. Removed; AE owns gain again (cap 12x, prefer long shutter).
2. **Spawn-per-frame** (~1 fps): full ISP open/CSI-cal/600ms settle every frame.
   New `camgrab --stream` keeps the pipeline open and re-emits `/tmp/preview.rgb`
   (~2+ fps in 5s smoke test; scales with shutter). App polls the file.
3. **Preview path:** lighter 2-cell debayer (less mushy), gamma ~0.65 (less crushed),
   milder sat boost, gain-linked chroma denoise. Meta at `/tmp/preview.meta` for HUD
   (`S …  G …x  Y …  FR …`).

```sh
# continuous viewfinder (what the Camera app now launches)
/mnt/sd/linux-lab/camgrab /tmp/frame.raw 14 0 --stream
# live-swap UI after rebuild:
#   mount --bind /mnt/sd/linux-lab/neonwire /bin/neonwire && killall neonwire
```

**★ Preview pipeline v3 (iterate toward stock quality):** stock is good because it runs
full 3A + ISP (BNR/LSC/CCM/HW demosaic). We only have raw IMGO, so software post now:
full **10-bit MIPI unpack** + black-level, 2×2 cell average demosaic, **stock daylight WB
× small gray-world nudge**, **luma 3×3 denoise** + chroma suppress, **temporal 50/50
blend** in `--stream`, soft gamma. AE stream caps shutter ~2000 / gain 3× so fps stays
usable (stock preview is 30 fps @ framelength 1234). Deployed to SD `camgrab`.

**Still open:** (1) colorimetric CCM; (2) full-width 1592 DMA; (3) HW BNR/LSC if we can
enable stock EN1 modules without stalling TG; (4) true bilinear demosaic for stills.

## Earlier: FIRST PIXELS on L1 (2026-07-20) — partial frame via `--stock-regs`

**Breakthrough:** `camgrab --stock-regs` writes real RAW10 into the ion buffer
(poison `0xA5`/`0x5A` test proved DMA, not a cache ghost). Debayered crop:

- `frame-stock-regs-597.raw` / `frame-stock-regs-597.png` — **1592×597** (top half)
- Full 1592×1194 still ends with untouched bottom poison (`0x5A`) — IMGO stops at
  exactly `h/2` with freerun-safe EN1. Full height is the remaining task.

```sh
/mnt/sd/linux-lab/camgrab /tmp/frame.raw 14 0 --stock-regs
# exit 0 when buffer fill detected; meta 1592x1194 RAW10 (bottom half may be poison)
```

What worked:
1. Bulk-apply stock Android live-preview **top+dma** regs (`stock_pass1_regs.inc`
   from `/proc/driver/isp_reg` while CameraTest streamed).
2. **Freerun-safe enables:** `EN1=0x40001001` (not stock `0x44b598a9` — that stalls TG).
3. **Fixed-base IMGO** (clear FBC_EN bit14); skip RTBC ENQUE — ring never sets
   `bFilled`, and ENQUE was a red herring for the half-height issue.
4. Success = poison cleared (not `ISP_WAIT_IRQ` / DEQUE).

Still open: why IMGO retires only 597/1194 lines; full stock EN1 stalls TG without
the rest of the HAL pipe; RTBC `bFilled` never sets on this ODM kernel path.

### Earlier: SENSOR ALIVE on L1 (2026-07-19) — reads ID 0x2509 reproducibly

`camprobe` now brings up the sensor master clock and reads the ID on the custom
Linux userland: `SP2509 ONLINE (id=0x2509 drv=0x00010000)`, exit 0, 3/3 reproducible.
The neonwire Camera app shows a green **SENSOR ONLINE** state.

**The fix (MCLK bring-up).** The kernel never wires the sensor MCLK — the stock mtkcam
HAL does it from userspace, and camprobe now replicates that:
1. hold `/dev/camera-isp` open → `ISP_EnableClock` ungates SEN_TG/SEN_CAM;
2. `ISP_SENSOR_FREQ_CTRL=1` → CAMTG clock mux to 48 MHz (univpll_d26);
3. via `ISP_WRITE_REGISTER` (seninf regs are all in the ioctl window [0x4000,0x10000)):
   `SENINF_TOP(0x8000)|=0x400`, `TG1_SEN_CK(0x8304)=0x00010001` (÷2 → 24 MHz),
   **`TG1_PH_CNT(0x8300)=0xA0000001`** = PCEN(bit31) | ADCLK_EN(bit29) | TGCLK_SEL=1,
   `CAM_TG_SEN_MODE(0x4410)|=1` (CMOS_EN).

**Why the earlier attempt failed:** camprobe set only bit 29 (ADCLK_EN, what the kernel's
`ISP_MCLK1_EN` toggles). That *gates* the clock but the **phase counter never ran** —
`PCEN` (bit 31) is the actual MCLK-output enable. No oscillation → sensor unclocked →
I2C ID reads 0x0000. One bit.

**GPIO119/CMMCLK pinmux:** the pad was already muxed to CMMCLK by LK/boot — `/dev/mem`
is blocked (STRICT_DEVMEM) so camprobe can't set it, but it doesn't need to. Register
map came from the MT8127 mtkcam HAL source (`seninf_reg.h`, mt8127-tadpole vendor tree).

## A3 status (2026-07-20) — ★ pixel-width SOLVED; now DMA-writeback (CQ) blocked

### M4U / ion (DONE — was never the low-MVA bug)
On-device `ion_probe 17` + camgrab:

| Step | Result |
|------|--------|
| `ION_ALLOC` MM heap | ok, ~2.4 MB |
| `CONFIG_BUFFER(CAM_IMGO=17)` | ok (without it `GET_PHYS` → EFAULT) |
| `GET_PHYS` | **MVA `0x00040000`**, kernel_len=`2400256` |
| `CONFIG_PORT Virtuality=1` | ok (domain hard-coded 3 in kernel) |

`0x40000` is the **first free 256 KB M4U block** (`MVA_BLOCK_SIZE`). Stock logs
`~0x1e40000` only because the HAL had already allocated many other buffers.
Poison `0xA5` + ion cache **FLUSH** then **INVALID** proves the mapping: buffer
stays full `0xA5` end-to-end (no silent zeroing). No M4U translation faults in
dmesg.

### Datapath config now correct (readbacks)
- `EN1=0x40001001` (TG1\|PAK\|CAM), `DMA=IMGO`, `CQ0` off  
- `FMT=0x00010001` (SCENARIO=1, TG1_FMT=RAW10) — fixed CLR mask for TG1_FMT  
- `MUX2=0x00100000` (`IMGO_MUX_EN`, MUX=0 after PAK) — reg is **0x4078**, not 0x40C4  
- `CLK_EN=0x9efd` (includes `DMA_DP`), `FBC_EN` cleared (stock first-buffer path)  
- `IMGO BASE=0x40000` held for whole capture  

### ★ PIXEL-WIDTH BUG SOLVED (2026-07-20) — via `/proc/driver/isp_reg` diff
The world-readable `/proc/driver/isp_reg` (no root) let us diff stock-live vs L1
directly (`tools/l1-isp-diff.sh` vs `reference/android-capture/camera-live-20260720/`).
Two wrong SENINF writes were the cause; both now match stock:
1. **`SENINF_TOP` (0x8000): was `0xd00`, stock `0x400`.** camgrab OR'd in
   `S1_EN|S1_SEL` (0x900) — a wrong guess that mis-routed SENINF1 pclk.
2. **`SENINF1_CTRL` (0x8010) polarity nibble: was `...0600` (bits 10,9), stock
   `...0280` (bits 9,7).** `tg_input_mipi()` had the hsPol/vsPol bit positions wrong.

Result: **`FRMSIZE` now `0x064004b1` (PXL=1600, was 101)**; SENINF IMG SIZE regs
`0x8024-0x8030` now `0x064004b0` (1600×1200); **`INTER_ST` counter now INCREMENTS**
(was frozen `0x102`) → TG is clocking real frames. Capture meta 1592×1194 RAW10.

### Still failing — ISP Pass1→IMGO DMA writeback (deep-dived 2026-07-20)
```
FRMSIZE = 0x064004b1  ✓ 1600 px/line       INTER = 0x2a2a0203 ✓ frames counting
buffer  = still all 0xA5   got_frame=0     ← IMGO DMA never commits
```
**Register-level pass1 config now fully matches stock** (verified via
`tools/l1-isp-diff.sh` against the golden dump), and these camgrab bugs were fixed:
- `TG_PATH_CFG` (0x4420): was setting **DB_LOAD_DIS (bit8)** — that disables the
  per-SOF double-buffer LOAD that latches the IMGO shadow to active. Now `0x01100000`
  (bit8 clear, bits 20,24 = TG→pass1 routing set) to match stock.
- `MUX_SEL` (0x4074): camgrab never set it; now `0x00100008` (stock).
- `CTL_SEL`(0x4018) DB_EN bit4 kept set; `DMA_EN=0xab`; `IMGO_FBC` bit4+[19:16];
  `IMGO_CON/CON2`=`0x08100850`/`0x00100800`; `EN2` low bits `0x1f`.

**IRQ path is now ALIVE** via **`ISP_SET_USER_PID`** (ioctl cmd 10) — camgrab never
called it. MTK's ISP IRQ handler signals the registered PID (realtime **signal 44**)
and advances the RTBC buffer ring. Added a handler so the signal no longer kills us.

**The remaining wall (needs the ODM kernel ABI, not in our reference tree):**
- Only **ONE** sig44 fires then stops — pass1 is not producing per-frame completion
  IRQs (INTER free-runs at TG level, but CAM pass1 isn't retiring frames to IMGO).
- `ISP_WAIT_IRQ` (cmd 6) returns **EFAULT** every call. The reference-tree
  `ISP_WAIT_IRQ_STRUCT` is 16B {Clear,Type,Status,Timeout} and matches camgrab — so
  the **device's ODM kernel uses a DIFFERENT (larger) struct** (MTK often adds
  `UserKey`/`UserNumber`). Need the device's actual struct.
- Likely the real mechanism is kernel-managed **RTBC via `ISP_BUFFER_CTRL`** (cmd 11,
  `ISP_BUFFER_CTRL_STRUCT`): the kernel IRQ handler owns the IMGO buffer ring, so a
  fixed-base bypass never gets written. **Next: RE the device's ISP_WAIT_IRQ_STRUCT +
  ISP_BUFFER_CTRL enqueue protocol** from the extracted device mtkcam HAL
  (`reference/firmware/work/system-unsparse/cam-extract/vendor/lib/`), then enqueue
  our ion buffer into the RTBC ring instead of writing IMGO_BASE directly.

Kernel ioctls (magic 'k', from `camera_isp.h`): RESET=0 READ_REG=2 WRITE_REG=3
WAIT_IRQ=6 READ_IRQ=7 CLEAR_IRQ=8 SET_USER_PID=10 RT_BUF_CTRL=11 SENSOR_FREQ=14.

### RTBC ring integrated (2026-07-20 pt2) — buffer in kernel ring; grab-error blocks fill
HAL ABI recovered from stock `libcamdrv.so`/`libimageio_plat_drv.so` (exact ioctl
numbers + structs). camgrab now uses the **RTBC ring** instead of fixed IMGO_BASE:
- `ISP_BUFFER_CTRL` = `0xC0106B0B`, 16B struct `{ctrl,buf_id,data_ptr,ex_data_ptr}`.
  `ctrl`: ENQUE=0 DEQUE=2. `buf_id`: **IMGO=4**. `data_ptr`→ 28B
  `ISP_RT_BUF_INFO_STRUCT {memID,size,base_vAddr,base_pAddr,tsS,tsUs,bFilled}`.
- Sequence: SET_USER_PID → program IMGO geometry regs → **ENQUE our ion buf** (kernel
  now owns IMGO_BASE, reloads from base_pAddr each SOF) → VF on → WAIT_IRQ → **DEQUE**.
- **WORKING:** ENQUE ok, DEQUE returns our buffer (`count=1 pa=0x40000`). CQ0_EN (EN2
  bit31) now set so the CQ engine reloads base from the ring.

### ★ KERNEL DISASM (2026-07-20 pt4) — the real gate found
Decompressed the stock kernel (boot.img → gunzip zImage → 15 MB ARM Image, base
0xC0008000, full kallsyms) and disassembled `camera_isp.c` handlers. ROOT CAUSE:
the RTBC frame-done handler (VA 0xC0556D30), ring-advance, and DEQUE are **all gated
on `*(ISP_base+0x414) & 1`** — the pass1/CMOS "streaming active" bit. No frame-done
IRQ ⇒ handler bails ⇒ bFilled never set ⇒ WAIT_IRQ times out (returns −EFAULT; the
EFAULT is a TIMEOUT, not a bad pointer). The single sig-44 we saw is fired
SYNCHRONOUSLY by SET_USER_PID itself (siginfo template sival hardcoded 0x4D2=1234, at
G+0x2E4) — NOT a real interrupt. DEQUE hardcodes report count=1 (G+0x38C:=1), so it
always returns 1 buffer regardless of enqueue depth (ring capacity is really 16 at
channel+0x0C; g_pstRTBuf channel stride 0x1D4; descriptor stride 28, bFilled@+0x34).
Key VAs: ISP_ioctl 0xC055921C; RT_BUF_CTRL 0xC0559D78; ENQUE 0xC055AB68 (first buf
writes H[0x320]=base_pAddr @0xC055AC28); DEQUE 0xC055A27C; WAIT_IRQ 0xC0559648
(timeout→−EFAULT @0xC055A110); SET_USER_PID 0xC05598FC; frame-done 0xC0556D30 (gate
test @0xC0557004); G=0xC0EFB468, g_pstRTBuf=*(G+0x27C).

**Register mapping (critical):** kernel H = ISP_base = CAMINF = physical **0x15004000**
= IMGSYS(0x15000000)+0x4000. camgrab's ISP_WRITE_REGISTER ioctl offset O maps to H+O
but is RESTRICTED to O∈[0x4000,0x10000) — so the **entire lower block H+[0,0x4000)**
(real IMGO base H+0x320, the gate H+0x414, and presumably the real DMA-arm/INT-enable)
was NEVER configured by camgrab. Reach it via `mmap(0x15000000, size≥0x4418)` → gate at
mmap offset **0x4414** (= H+0x414). camgrab now sets it: readback 0x1000→0x1001, write
sticks — but bFilled STILL 0, so the gate bit alone is insufficient (may be HW status,
or the lower-block DMA-enable + frame-done INT-unmask are also needed). **2nd kernel
disasm pass in progress** for the full ordered lower-block DMA-arm chain.

### pt5 — MT8127 ISP driver SOURCE found; register map verified
`reference/upstream/android_kernel_quanta_narnia/mediatek/platform/mt8127/kernel/core/
camera_isp.c` (+ inc header) is the actual MT8127 ISP driver source (CRLF; `tr -d '\r'`).
Register map: `ISP_ADDR=CAMINF_BASE+0x4000=0x15004000`; EN1=+0x4, CTL_EN2=+0x8,
INT_STATUS=+0x24, DMA_INT=+0x28, SW_CTL=+0x5C, IMGO_FBC=+0xF4, IMGO_BASE=+0x300,
IMG2O_BASE=+0x320, **TG_VF_CON=+0x414 (bit0=VFDATA_EN "TG1 Take Picture Request")**.
**camgrab ioctl offset = 0x4000 + (ISP_ADDR reg offset), so kernel H+X = camgrab ioctl
0x4000+X — ALL camgrab CAM_CTL offsets verified correct** (EN1 0x4004, INT 0x4024, IMGO
0x4300, VF_CON 0x4414, FBC 0x40f4). Corrects both disasm passes' "gate/lower-block"
theories: the "gate at H+0x414" is just TG_VF_CON (already set); the "unreachable lower
block" was a base-mapping error (it IS reachable via ioctl). **No hardware INT-enable
register** — ISP_Irq masks in software (IrqInfo.Mask[]), HW IRQ fires on any INT_STATUS
bit. So nothing register-offset-wise is missing.

**TRUE REMAINING BLOCKER (unchanged, now authoritatively framed):** INT_STATUS(0x4024)
bit10 PASS1_TG1_DON never sets ⇒ CAM pass1 back-end never completes a frame ⇒ no IRQ ⇒
RTBC never advances/fills. SENINF/CAM-TG front-end works (frames counted); RTBC enqueue +
kernel IMGO-base write work; VF_CON set. The gap is purely the **pass1 datapath/DMA not
producing an IMGO frame** (full-pipeline EN1 stalls TG; minimal EN1 gives no output). That
is a hardware pass1-config problem, not RTBC/IRQ/offset. Next: derive the exact minimal
pass1 module-enable + IMGO DMA-arm from the SOURCE (search its self-test/StartHW paths and
the CQ0/FBC setup) rather than register guessing.

### Bisect results (2026-07-20 pt3) — grab error is NOT from our reg changes
Systematically tested against the `sival=0x4d2` signal + `bFilled`:
- Removed speculative `CTL_START` pulse + set `IMGO_FBC` to stock `0x02234010`
  (FBC_EN/ring bit14 ON, needed for RTBC): **sival unchanged**, no fill.
- Enabled CQ0 (`EN2` bit31): **sival unchanged**, no fill.
- Enqueued a 3-buffer ring (distinct MVAs 0x40000/0x2c0000/0x540000): all ENQUE ok
  but **DEQUE always returns only buffer 0, count=1, bFilled=0**; sival unchanged.
=> The `sival=0x4d2` (= 1234 = SP2509 framelength, possibly a frame descriptor NOT an
IRQ-status) and the single-buffer DEQUE are **invariant** across FBC/CQ0/multi-buffer/
CTL_START. So the blocker is NOT our register config and NOT buffer count — the kernel
ring accepts one buffer but the **IMGO DMA never executes** into it, and multi-enqueue
doesn't populate the ring as the recipe implied. This needs the **device kernel source**
(ODM camera_isp.c RT_BUF_CTRL + CQ execution) to go further — the observable ioctl/reg
state no longer changes with anything we can poke. WAIT_IRQ still EFAULTs (ODM per-user
IRQ record). **State banked; not a captured frame.** camgrab.c has the full RTBC path.

**Earlier framing — TG1 GRAB ERROR:** the pipeline fires **one** pass1 then halts.
The ISP IRQ signal (realtime sig 44) carries status `sival=0x4d2` = bits {1,4,6,7,10}
= `PASS1_TG1_DON`(10) **+ `TG1_ERR_ST`(4)**. So pass1 completes once WITH a TG grab
error, buffer `bFilled` stays 0, no re-arm. Next: resolve the TG grab error — the
grab window (`GRAB_PXL=0x063a0002`/`GRAB_LIN=0x04ac0002` = 1592x1194) vs the sensor's
actual received line/pixel count must mismatch; try matching grab exactly to the
received SENINF size, or widen/narrow grab end. Secondary: `ISP_WAIT_IRQ` (`0x40106B06`,
16B, over-allocated writable struct) still EFAULTs — device ODM kernel wants something
SET_USER_PID doesn't fully establish (per-user IRQ record/UserKey); can poll DEQUE
`bFilled` instead of relying on WAIT_IRQ. Both structs/ioctls are in camgrab.c now.

### Stock extract map
| Path | Contents |
|------|----------|
| `…/cam-extract/vendor/lib/` | 55 libs: camdrv, imageio_plat_drv, campipe, 3a, m4u, … |
| `…/cam-extract/vendor/lib/hw/camera.mt8127.so` | HAL module |
| `…/cam-extract/re/*.S` | disasm of Pass1 config/enable |

### How to resume
```sh
/mnt/sd/linux-lab/camgrab /tmp/frame.raw 14 0
# TG win: SIZE != 0 (have this)
# full win: non-zero samples in buffer + debayer
```
Debayer: `python3 debayer.py frame.raw out.png --w 1592 --h 1194`

### ★ Unblock plan (2026-07-20): live register diff vs stock Android
Blind register guessing is exhausted (40 combos). Next move is a **register-level diff**
of stock Android (camera preview streaming = working) vs our L1 (broken). This needs a
one-time boot swap to the adb-root stock boot. **Fully staged — see
[../../docs/27-camera-live-capture.md](../../docs/27-camera-live-capture.md).**
- `camdump.c` — read-only ISP/SENINF/TG/MIPI snapshot (built, validated on L1).
- `../../tools/capture-camera-live.sh` — one-shot adb capture (saves + SHAs everything).
- `../../reference/android-capture/camera-live-20260720/L1-cold-baseline.txt` — our
  broken side, already saved so the trip is single-pass.

---

## Historical: A1 (probe tool) + why ID was 0x0000

`camprobe.c` (static musl armv7, ~36 KB) implements the plan's A1 contract:

```
exit 0  "SP2509 ONLINE (drv=0x....)"
exit 1  "SENSOR OFFLINE stage=<nodev|isp|setdriver|i2c>"
```

The neonwire Camera app spawns it to light its ONLINE/OFFLINE state. Modes:
`-v` verbose, `--dump` ISP/seninf register sanity dump.

### What A1 established (all verified live on the DL7006)

- **The ioctl path works.** `/dev/kd_camera_hw` + `KDIMGSENSORIOC_X_SET_DRIVER` +
  `KDIMGSENSORIOC_T_CHECK_IS_ALIVE` drive the full in-kernel sequence — dmesg shows the
  stock driver entering `SP2509MIPIGetSensorID`, doing the board power dance, and
  retrying the ID read 3×.
- **Driver slot order (device truth, docs had it flipped):** idx0 = SP2509, idx1 = SP0A19;
  idx2/3 (GC2355/GC0312) are not compiled in (`SET_DRIVER` -EIO). Payload
  `(MAIN_SOCKET=1)<<16 | idx`.
- **ISP power/registers are alive.** `/dev/camera-isp` open enables the IMG clock gates;
  reg `0x8100` (offset from CAMINF, ioctl window `[0x4000,0x10000)`) has a non-zero
  reset value, and writes stick while the fd is held.
- **`ISP_MCLK1_EN` = bit 29 of CAMINF+0x8300** and the kernel never calls it — the stock
  HAL sets it from userspace. camprobe asserts it (with a keep-alive child, since the
  power cycle clears it once) — **still ID = 0x0000**, so MCLK-enable alone is not enough.
- **The stock DL7006 kernel's SP2509 driver is ODM-modified** (log strings `gpw1 Read
  Sensor ID Fail` don't exist in the Amazon ford reference tree) — treat the reference
  source as approximate.
- **`ISP_mmap` is the master key** (camera_isp.c:4680): mmap of `/dev/camera-isp` by
  physical pgoff exposes IMGSYS, **SENINF**, **PLL (0x10000000)**, **MIPI-RX config
  (0x1500C000) + analog (0x10010000)**, **GPIO (0x10005000)**, EFUSE. This is how the
  stock seninf_drv.cpp does pin-mux/PLL/MIPI config entirely from userspace — and how
  our capture pipeline can too. No kernel patching needed.

### Root cause of ID=0x0000 — ANSWERED by the stock capture (A2, already done)

The A2 stock-Android trace was captured 2026-07-19 and lives in
`reference/android-capture/` (`devicetest-kernel.log` + `devicetest-main.log`, via
`adb logcat -b kernel`). Reading it back answers the MCLK question outright — no new
boot needed:

- Stock reads the ID **successfully**: `[SP2509MIPIRaw] gpw sensorIDL = 0x2509`
  (kernel log 2202.898). So power + I2C + the driver are fine; the only thing our
  camprobe lacked is a **running MCLK**.
- The HAL step our probe skips (main log 2202.638, immediately before the ID read):
  ```
  SensorHal: [setTgPhase] Tg1clk: 24, mclk1: 48000, clkCnt1: 1
  SeninfDrvImp: [setTg1PhaseCounter] pcEn(1) clkPol(0)
  ```
  i.e. it programs the **seninf TG1 phase counter** (source 48 MHz ÷2 → 24 MHz to the
  sensor pad, counter enabled). `ISP_MCLK1_EN` (bit 29 @ CAMINF+0x8300) only *gates*
  that clock — without `setTg1PhaseCounter pcEn=1` there is no oscillation, so the
  sensor can't clock out its ID. **That is the exact camprobe gap.**
- Full stock preview order (main log): `SeninfDrv init` → `setTgPhase`/
  `setTg1PhaseCounter` (MCLK on) → sensor `Open`/ID read → `initTg1CSI2` (CSI-2 lane
  **calibration**, SettleDelay 14) → `setTg1GrabRange`/`setTg1SensorModeCfg`/
  `setTg1InputCfg inSrcTypeSel=8` → ISP pass1 DMA. `ISP_mmap` shows the HAL maps the
  SENINF/PLL/MIPI-RX/GPIO blocks (pgoff 0x10, 0x10000 window) to do all of this from
  userspace — confirming our L1 path needs no kernel patch.

**Next camprobe iteration:** find the TG1 phase-counter register in the seninf block
(mmap `/dev/camera-isp` at SENINF_BASE, or via ISP_WRITE_REGISTER if it falls in the
CAMINF+0x4000..0x10000 window) and replicate `setTg1PhaseCounter pcEn(1)` +
`setTgPhase mclk1=48000` before CHECK_IS_ALIVE. Cross-reference the mt6589-era open
seninf_drv.cpp for the register offsets behind these HAL calls.

## Files

- `camprobe.c` — liveness probe (deployed at `/mnt/sd/linux-lab/camprobe`)
- Build: `armv7l-linux-musleabihf-gcc -Os -static -no-pie -Wall -o camprobe camprobe.c`

## A2 stock-capture reference (already in the repo)

- `reference/android-capture/devicetest-main.log` — mtkcam HAL trace: SeninfDrvImp /
  SensorHal setTgPhase, setTg1PhaseCounter, initTg1CSI2 calibration, grab-range config.
- `reference/android-capture/devicetest-kernel.log` — kernel-side SP2509 driver:
  successful `Read Sensor ID = 0x2509`, GetResolution/GetInfo, `Camera-ISP mmap_kmem`.
- `docs/22-camera-sensor-reference.md` — the sensor reference distilled from these.

## Key kernel-source references

- `reference/upstream/kernel_amazon_mt8127-common/drivers/misc/mediatek/imgsensor/inc/kd_imgsensor.h` — ioctls (magic 'i')
- `.../src/mt8127/kd_sensorlist.c:1056` — CheckIsAlive (power→10ms→ID read→off)
- `arch/arm/mach-mt8127/camera_isp.c` — `ISP_MCLK1_EN` (:5525), `ISP_mmap` (:4680),
  reg window bounds (:1884), `ISP_EnableClock` (:1616)
- `arch/arm/mach-mt8127/include/mach/camera_isp.h` — ISP ioctls (magic 'k': READ_REG=2, WRITE_REG=3)
