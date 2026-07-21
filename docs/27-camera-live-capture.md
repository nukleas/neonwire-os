# Camera live capture — stock-Android register diff (the A3 unblock)

**Goal:** capture the *working* ISP/SENINF/TG/MIPI register state while stock Android
streams a live camera preview, so we can diff it against our L1 (camgrab) state and
find why **SENINF never clocks CAM_TG** (`TG_SOF_CNT`/`TG_FRMSIZE` stuck at 0).

The A2 capture we already have (`devicetest-*.log`) is only HAL **log strings** — it
tells us the HAL *calls* `setTg1PhaseCounter`/`initTg1CSI2`, not the resulting register
**values**. This trip captures the values. One Android boot, everything saved, then back.

## Why a boot swap is needed (and why it's safe)

We never touch system/vendor — only the **boot partition** (`bootimg @ 0x1d80000`,
16 MiB). Fully reversible over the Preloader (mtkclient) on both legs.

| Image | sha256 (prefix) | Role |
|-------|-----------------|------|
| `reference/dumps/session-20260718/work/boot-adb/boot-adb.img` | `0ffb3d21…` | **adb-root Android** (verified below) |
| `experiments/linux-initramfs/out/boot-linux-l1-neonos.img` | `59168998…` | **NeonOS** (flash back after capture) |
| `reference/dumps/session-20260718/images/boot.img` | `df7db881…` | pristine stock (ultimate restore) |

### boot-adb.img verification (done 2026-07-20)

Its hash (`0ffb3d21…`) does **not** match the builds listed in `docs/10` (a later
repack), so it was re-verified by unpacking the actual image:

- `default.prop`: `ro.secure=0`, `ro.adb.secure=0`, `ro.debuggable=1`,
  `persist.sys.usb.config=mtp,adb` → **adb root works, no RSA popup**
- `adb_keys` present (0640), `init` + `sbin/adbd` are **0750 (executable)** — i.e. **not**
  the 0644 bootloop build from `docs/10`
- kernel block is stock MTK (byte-reused)
- ⚠️ ramdisk wires an `auto_shutdown` service (`init.mt8127.rc:708`) that runs
  `reboot -p` **~50s after boot IF `/data/AUTO_SHUTDOWN` exists**. Deleting the flag
  *after* the loop starts does **not** stop it. **Checked 2026-07-20: the flag is
  ABSENT on userdata (p8), so the service exits harmlessly.** Belt-and-suspenders is
  baked into the capture script (`rm -f /data/AUTO_SHUTDOWN` first thing).

## Runbook

### Leg 1 — into Android (Preloader, cabled)

```sh
cd $REPO
./tools/flash-boot-adb.sh          # tablet OFF+unplugged, plug USB at "Waiting"
# power on -> stock Android boots with adb root
adb wait-for-device && adb root && adb shell id -u   # expect 0
```

### Capture — one shot, saves everything

```sh
./tools/capture-camera-live.sh
# -> reference/android-capture/camera-live-20260720/
```
The script: drops SELinux to permissive (`setenforce 0`) so root can ioctl
`/dev/camera-isp`; scans no-root `/proc`+`/sys/debug` camera nodes (fallback if the
ioctl is denied); pushes `camdump`; snapshots registers **idle**, then **during live
preview** (t+2/4/7/11s) while `com.DeviceTest/.CameraTest` runs; pulls kernel+main
logcat, `getprop`, `dumpsys media.camera`; writes `SHA256SUMS`.

**Before flashing back, VERIFY the trip worked:**
- at least one `stock-preview-*.txt` shows `TG_SOF_CNT != 0` **and** `TG_FRMSIZE != 0`
  (proof the pipeline was actually streaming when snapshotted)
- `SHA256SUMS` present and non-empty
If preview dumps still show `TG_SOF_CNT = 0`, the ioctl was likely blocked (check
`stock-idle.txt` opened the fd) — inspect `proc-debug-nodes.txt` and re-run before
leaving Android.

### Leg 2 — back to NeonOS (Preloader, cabled)

```sh
./tools/flash-neonos.sh            # flashes boot-linux-l1-neonos.img
# power on -> NeonOS; verify over Tailscale:
ssh root@100.x.y.z uptime
```

## The diff (the actual payoff)

```sh
cd reference/android-capture/camera-live-20260720
# L1-cold-baseline.txt is our broken side (already saved).
diff <(grep '^0x' L1-cold-baseline.txt) <(grep '^0x' stock-preview-t7.txt) | less
```
Focus on the SENINF→TG clock path: `TG1_PH_CNT (0x8300)`, `TG1_SEN_CK (0x8304)`,
`SENINF_TOP (0x8000)`, `TG_SEN_MODE (0x4410)`, `TG_PATH_CFG (0x4420)`, the APMIXED/PLL
mmap block (CAMTG mux), and MIPI-RX analog. Whatever bit stock sets that our L1 doesn't
is the fix for camgrab.

## RESOLVED (2026-07-20) — register dump obtained via `/proc/driver/isp_reg`

The capture-boot build is **NOT needed**. `/proc/driver/isp_reg` is **world-readable
(0444)** on the stock kernel and SELinux permits the `shell` domain — full live ISP
register dump with no root. Same kernel on L1 => same node => direct cross-diff.
Captured stock live-preview dumps; the bug is decoded. See
`reference/android-capture/camera-live-20260720/FINDINGS.md`:
- TG FRMSIZE stock `0x064004b1` (1600 px/line) vs L1 `0x006504b1` (**101 px/line**) —
  SENINF delivers 101 px/line instead of 1600. Not lanes, not TG, not resolution.
- Prime suspects (stock sets, camgrab may not): SENINF size regs **0x8024-0x8030**
  = `0x064004b0` (1600x1200), and **0x8108 = 0x2b07d000** (CSI-2 DT 0x2b RAW10 +
  wordcount 0x7d0 = 2000 B/line). camgrab mislabels 0x8108 as CSI2_INTEN.
- Close it on L1 (no more Android): `tools/l1-isp-diff.sh` diffs a live camgrab
  `/proc/driver/isp_reg` against `STOCK-WORKING-ref.txt`.
- `camsensor` proc node is a runaway (spews GB) — never cat it. `camio_reg` empty.

### (superseded) earlier plan — register dump via rebuilt boot
The notes below were the fallback before we found the world-readable proc node.

## OUTCOME (2026-07-20) — register dump BLOCKED on this image; source facts won instead

The boot-adb.img is a **`user`** build. On it: `adb root` refuses (shell stays uid 2000),
no `su`, SELinux **Enforcing** and `setenforce 0` denied by policy. `camdump` therefore
gets **EACCES** on `/dev/camera-isp`. The MTK HAL programs SENINF/CSI-2/TG from userspace
via `ISP_mmap` (kernel log: `[Camera-ISP][mmap_kmem] pgoff(0x10)`), so those writes are
never logged, and HAL verbose logging is locked (shell `setprop` denied). **No register
dump is reachable from this image.**

What the trip *did* confirm (logs, no root — via logd like A2), cross-checked against the
driver source `.../imgsensor/src/mt8127/sp2509mipiraw_Sensor.c`:

| Fact | Value | Source |
|------|-------|--------|
| preview timing | pclk 24 MHz, **linelength 947, framelength 1234** | stock log == driver `.pre` |
| grabwindow | **1600×1200 full** (947 is a line-*period*, not px; MIPI 864 Mbps/lane) | driver:313 |
| MIPI lanes | **1 lane** (`SENSOR_MIPI_1_LANE`) | driver:397 |
| receiver | **NCSI2** (`MIPI_OPHY_NCSI2`) | driver:393 |
| bayer | **RAW_B (BGGR)** | driver:395 |
| settle | AUTO, lp2hs 30 ns | driver:394,316 |

camgrab **already** matches 1-lane (`CSI2_CTRL=0x431`, `dlane=0`) and full 1600×1200.
So the `PXL≈101` bug is **not** lane count or resolution — it is deeper CSI-2/SENINF
**pixel-mode / packing / datatype**. That is precisely what a register dump would reveal
and what this image cannot give.

### Decision fork for a real register dump
Getting `camdump` to run needs **uid 0 AND SELinux permissive simultaneously** (the node
is root-owned; permissive-only shell still fails DAC). Cleanest = rebuild a **capture
boot**: patch the ramdisk sepolicy permissive + add an `init` service that runs camdump
as root on a property trigger (`setprop` from shell → dump to /data), so a live preview +
one `adb setprop` yields the dump with no adb-root. Real build effort (mode-preservation
risk per docs/10). **Weigh against** just iterating the pixel-packing fix on L1, where we
have full register R/W and the confirmed sensor params above — likely cheaper.

## Tooling

- `experiments/camera/camdump.c` — **read-only** full-window register snapshot
  (ISP_READ_REGISTER sweep `[0x4000,0x10000)` + mmap of IMGSYS/MIPI-analog/PLL/GPIO).
  Never writes. Built armv7 musl static; validated live on L1 2026-07-20 (12288/12288
  reads OK). Binary at `experiments/camera/camdump`.
- `tools/capture-camera-live.sh` — host adb orchestrator (above).
- `reference/android-capture/camera-live-20260720/L1-cold-baseline.txt` — our broken
  side of the diff, captured on L1 before the trip.
```
```
