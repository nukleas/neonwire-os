# Camera — sensor reference (DL7006)

**Date:** 2026-07-19
Captured from working stock Android (camera app live) + the matching kernel source
(`kernel_amazon_mt8127-common/.../imgsensor/src/mt8127/sp2509mipiraw_*`).

## The hardware
- **Rear sensor: SuperPix `SP2509`** — 2 MP MIPI-CSI Bayer raw, **1600 × 1200**
  (`CAM_SIZE_2M_WIDTH/HEIGHT`). Single camera — no front sensor populated.
- **I2C write id `0x7a`** (7-bit `0x3d`), on the camera I2C bus.
- **Sensor ID `0x2509`** (`SP2509MIPI_SENSOR_ID`); modules seen: "BLX", "HuaQuan".
- **PCLK 24 MHz**; MCLK 24 MHz; MIPI raw.
- Driver: `SP2509_MIPI_RAW_SensorInit`, registered in `kd_sensorlist.c` under
  `SENSOR_DRVNAME_SP2509_MIPI_RAW`. **Built into the stock kernel** → present on L1.

## The stack (why it works on Android)
```
SP2509 (I2C 0x7a, register init tables in the kernel driver — no fw blob)
   │  MIPI CSI-2
   ▼
seninf  (MIPI receiver)  ──►  ISP pass1  ──DMA──►  raw Bayer in DRAM
   │
   ▼
mtkcam HAL (userspace): hal3a (AE/AF/AWB) + IspTuningMgr + pass2/dip → YUV/JPEG
```
Live evidence: the `3ATHREAD` (`[SP2509MIPIRaw] gpw write 0x24 = 60` …) is the HAL's
3A loop driving exposure/gain over I2C in real time — the sensor is fully alive.

Nodes: `/proc/driver/camsensor` (present), plus the proprietary `/dev/kd_camera_hw`,
`/dev/camera-*`, `/dev/CAM_CAL_DRV` the HAL opens.

## What it takes to get camera on L1 — honest assessment
Unlike Wi-Fi (a firmware-path fix) or audio (a DAPM clock gate), **the camera has no
easy handle**:

- **MT8127 3.18 has NO V4L2 interface.** You cannot `open("/dev/video0")` and grab
  frames. The pipeline is programmed through MTK's proprietary `kd_camera_hw` / ISP
  ioctls, orchestrated entirely by the closed **mtkcam** HAL (dozens of
  `libmtkcam*.so`, bionic-only, needs cameraserver + gralloc/ion). Running that HAL
  under busybox/musl is not realistic.
- **The bounded RE path** (weeks, not hours): write custom code that
  (1) powers the sensor via `kd_camera_hw`, (2) pushes SP2509's register init table
  over I2C `0x7a`, (3) configures **seninf** (the MIPI receiver), (4) programs **ISP
  pass1** to DMA raw Bayer into DRAM, then (5) **debayers in software** to RGB. That
  yields a still/preview with no 3A tuning — a genuine but large undertaking, all in
  undocumented ISP register territory.

**Verdict:** the reference is now fully in hand (sensor, address, resolution,
in-kernel driver). Camera capture on L1 is an *experimental stretch goal* — sequence
it after Wi-Fi (nearly done) and audio. If pursued, the target is a minimal raw-Bayer
grab via seninf + ISP pass1, not the full HAL.
