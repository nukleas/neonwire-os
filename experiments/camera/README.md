# Camera bring-up — SP2509 (plan track A)

## Status after A1 (2026-07-19): probe tool works, sensor not answering I2C yet

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

### Root-cause hypotheses for ID=0x0000 (next session, in order)

1. **CMMCLK pin mux** not in clock-function mode — dump/set via the GPIO block mmap.
2. **MCLK divider/PLL** unconfigured — bit 29 gates a clock whose source isn't running;
   seninf_drv.cpp (mt6589-era open HAL trees) documents the required PLL + `SENINF_CK` regs.
3. Reg map delta between DL7006 seninf and the reference tree.

Decisive experiment: boot stock Android, run its camera, and dump SENINF + PLL + GPIO
blocks (same mmap offsets); diff against L1 state. Every delta is a register we must set.

## Files

- `camprobe.c` — liveness probe (deployed at `/mnt/sd/linux-lab/camprobe`)
- Build: `armv7l-linux-musleabihf-gcc -Os -static -no-pie -Wall -o camprobe camprobe.c`

## Key kernel-source references

- `reference/upstream/kernel_amazon_mt8127-common/drivers/misc/mediatek/imgsensor/inc/kd_imgsensor.h` — ioctls (magic 'i')
- `.../src/mt8127/kd_sensorlist.c:1056` — CheckIsAlive (power→10ms→ID read→off)
- `arch/arm/mach-mt8127/camera_isp.c` — `ISP_MCLK1_EN` (:5525), `ISP_mmap` (:4680),
  reg window bounds (:1884), `ISP_EnableClock` (:1616)
- `arch/arm/mach-mt8127/include/mach/camera_isp.h` — ISP ioctls (magic 'k': READ_REG=2, WRITE_REG=3)
