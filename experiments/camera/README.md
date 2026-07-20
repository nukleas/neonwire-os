# Camera bring-up — SP2509 (plan track A)

## Status: SENSOR ALIVE on L1 (2026-07-19) — reads ID 0x2509 reproducibly

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

## A3 status: full capture pipeline built + safe; CSI-2 receiver not locking yet

`camgrab.c` assembles the entire HAL-free pass1 path and runs without crashing or
corrupting memory. Every stage initializes correctly EXCEPT the MIPI CSI-2 receive:

- ✅ MCLK + sensor open (reads 0x2509) + **streaming** — dmesg confirms
  `SP2509MIPIPreview`/`SP2509MIPIControl` ran (X_CONTROL preview scenario works).
- ✅ **ion DMA buffer + M4U** — `ION_MM_CONFIG_BUFFER(CAM_IMGO=17)` → `GET_PHYS` MVA
  0x00040000 → **`MTK_M4U_T_CONFIG_PORT(CAM_IMGO, Virtuality=1)` via /proc/M4U_device**
  makes the MVA translate. SAFETY GATE: camgrab aborts before enabling the DMA if the
  port config fails (else the engine emits the MVA as a raw phys addr into low memory).
- ✅ ISP TG1 grab-window + IMGO DMA config (offsets from isp_reg.h: IMGO BASE 0x4300 /
  XSIZE 0x4308 / YSIZE 0x430C / STRIDE 0x4310; TG grab 0x4418/0x441C; CTL_EN1 0x4004
  TG1_EN|CAM_EN; DMA_EN 0x400C IMGO_EN). VF-enable at 0x4414 bit0.
- ❌ **No frame completes.** `PASS1_TG1_DON` (INT bit 10) times out after 2 s;
  `CAM_TG_FRM_CNT_ST`(0x4444)=0 and `CAM_TG_FRMSIZE_ST`(0x4448 line count)=0 the whole
  time. So the sensor streams but **no MIPI data reaches the TG** — the CSI-2 PHY isn't
  locking. `SENINF1 status`(0x8014)=0x8000007f, constant (idle/unlocked).

**Next (the remaining RE target): CSI-2 analog PHY bring-up.** camgrab's `csi2_analog()`
replicates initTg1CSI2 (LDO/BG enable, lane enables, HW cal handshake via the mmap'd
0x10010000 block + in-window 0xC024/0xC038/0xC03C), but it's evidently not achieving
lane lock. Likely needs: (a) verify the analog mmap writes actually stick (MIPI power
domain), (b) the exact settle-delay / lane-enable / calibration values — best obtained
by reading the SENINF+MIPIRX-analog register values live from a stock capture (dump
0x8000-0x8130 and the analog block right after stock streams), (c) possibly dlane_num=1
lane-field encoding in CSI2_CTRL. This is the one undocumented-register stage left.

The MCLK milestone + this pipeline prove the userspace-register approach works end to
end; only the MIPI receive handshake remains between here and photons.

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
