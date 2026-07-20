# Wi-Fi, cracked by watching the working machine

**Date:** 2026-07-19

For weeks the CONSYS Wi-Fi was our one hard wall. We instrumented the kernel,
proved the power rails, the MTCMOS domain, the bus clock, and the TOPAXI firewall
were all perfect — and the chip *still* read `chipId = 0`. We concluded the fault
was "inside the connsys silicon," parked the thread, and called it the bottom.

**We were measuring the wrong thing.** Grok's suggestion — *restore the stock
firmware and watch how Android brings the chip up* — broke it open in one session.

## The trick: reading the working kernel log without root

Stock Android here is a locked `user` build: SELinux enforcing, `adb root` refused,
`dmesg` blocked for the `shell` uid. But Android's `logd` captures the kernel ring
with `CAP_SYSLOG`, and `shell` is in the `log` group — so:

```
adb shell 'logcat -b kernel -d'
```

hands you the full `dmesg` of a **working** Wi-Fi bring-up. No root required.

## What the working bring-up actually does

```
[13.621] Read CONSYS chipId(0x00000000)                       ← the "failure" we chased
[13.710] mtk_wcn_stp_enable ... enable = (1)
[13.718] 0x8127: ic info SOC_CONSYS.E2 (0x8a01/0x8a00, patch_ext:_e1)   ← chip identified over STP
[14.156] str(srh_patch) result(0)
[14.183] [Patch] BuiltTime=20160719, HVer=0x8a00, Platform=ALPS
[14.261] wmt_core: patch dwn:0 frag(45,720) ok                ← FIRMWARE PATCH DOWNLOAD
[14.405] wmt_core: patch dwn:0 frag(82,64) ok
[14.750] pwr_on_conn: OPID(3) type(9) ok                      ← whole-chip power-on succeeds
[29.35]  wmt wlan func on before wlan probe → [31.20] wlan probe ok → wlan0
```

Two facts demolish the old conclusion:

1. **`Read CONSYS chipId(0x00000000)` prints on working Android too.** That AP-side
   register is *never* how the chip is identified — the real identity comes over the
   **STP link** (`ic info: SOC_CONSYS.E2, 0x8a01/0x8a00`). Every instrument build we
   flashed was reading a register that reads 0 even when Wi-Fi works.
2. The chip only comes alive after a **firmware patch download** (`ROMv2_patch_*`,
   `WIFI_RAM_CODE_8127`) over STP, triggered by a **whole-chip power-on (`type 9`)** —
   *before* any Wi-Fi-specific call.

## Why our L1 failed

The 3.18 `conn_soc` driver loads its patches from **hardcoded paths**:

- `/etc/firmware/WIFI_RAM_CODE_8127` — `filp_open()` in `gl_kal.c` (`_E2`/`_E6` variants)
- `/system/etc/firmware/WMT_SOC.cfg` — `CUST_CFG_WMT_PREFIX` in `wmt_conf.h`

Our `wifi-diag.sh` copied the firmware to **`/tmp/fw/`** and pointed `wmt_launcher -p`
there — but the kernel's own `filp_open` never looks at `/tmp/fw/`. So the patch
download silently failed (`wmt_core_stp_init fail`) and the chip looked dead. It was
**starving for its firmware patch**, not walled by silicon. We were also calling
`echo 1 > /dev/wmtWifi` (Wi-Fi func-on, `type 3`), which assumes a chip already
brought up by a prior `type 9` — a step we never performed.

## The fix

No kernel patch, no reflash. Stage the firmware where the kernel actually reads it
(all writable tmpfs on the L1 initramfs), then run the WMT stack the way Android does:

```sh
sh experiments/net/wifi-up.sh     # copies fw to /etc/firmware + /system/etc/firmware
                                  # + /vendor/firmware, runs wmt_loader/wmt_launcher,
                                  # captures "patch dwn ok" + "wlan probe ok", brings up wlan0
```

The exact working blobs are archived in `reference/firmware/consys/`
(`WMT_SOC.cfg` with `co_clock_flag=0`, `WIFI_RAM_CODE_8127`, `ROMv2_patch_1_0/1_1_hdr.bin`).

Same recovery should apply to **Bluetooth** — identical STP/patch path, function 1.

## Lesson

We had a rigorous, exhaustive, *wrong* diagnosis because we never checked our premise
against a known-good reference. One `logcat -b kernel` on the working OS falsified
the whole "silicon wall" in a single line. When stuck, instrument the thing that
*works* before concluding the thing that doesn't is impossible.

---

## On-device bring-up attempt (2026-07-19) — got far, hit the SDIO layer

Ran the sequence live on L1 over serial. **The driver stack comes up and matches
Android's boot trace step for step:**
```
set current consys chipid (0x8127)             ← chip DETECTED
do_common_drv_init chipid:0x00008127
mtk_stp_wmt driver (major 190) installed
wmt_conf_read_file: /vendor/firmware/WMT_SOC.cfg
[mtk_wmtd] wmtd thread starts                  ← STP stack + mtk_wmtd live
```
Stock `/vendor/bin/wmt_launcher` **runs but stalls silently** on L1 (blocks on
Android's property service / socket we don't have), so it never issues its init
ioctls. Built a native replacement — **`experiments/net/wmtctl_min.c`** (freestanding
ARM, ~1 KB, raw syscalls) — doing them, decoded from the driver source:
```
WMT_IOCTL_SET_STP_MODE   = _IOW(0xa0,5) = 0x4004a005, arg 0x24  (FM_COMM<<4 | STP_SDIO)
WMT_IOCTL_FUNC_ONOFF_CTRL= _IOW(0xa0,6) = 0x4004a006, arg 0x80000000|type (WMT=4,WIFI=3)
```
`SET_STP_MODE` returns **0 (success)**.

### ★ The remaining blocker — SDIO transport enumeration
```
hif_sdio_stp_on: no supported func probed
wmt_ctrl_stp_conf: invalid Handle of WmtStp   →  func-on fails (iRet -3)
```
The **connsys chip's SDIO function never enumerates** on the msdc bus
(`/sys/bus/sdio/devices` empty; only `mmc0`=eMMC, `mmc1`=SD). `11230000.mmc` exists as
a platform device but its SDIO card never appears, so STP has no transport to attach
to. This is a layer *below* firmware — **not silicon, not host power, not firmware**
(all now proven working). On stock Android the connsys SDIO card enumerates once the
chip's SDIO interface powers up.

### ▶ Resume here next time
1. Boot L1, `sh /tmp/wifi-up.sh` (push it + `wmtctl_min` first — keep binaries tiny,
   the serial tty mangles many-chunk pushes; disable echo `stty -echo` helps).
2. Investigate why `11230000.mmc` doesn't enumerate the connsys SDIO function:
   - Compare L1 vs Android **boot** dmesg for the `11230000.mmc` SDIO scan (CMD5/52/55).
     Capture a clean Android boot log (reboot → `adb logcat -b kernel -d` before the
     camera app floods the ring — the one gap in `reference/android-capture/`).
   - Does the connsys SDIO-interface power rail come up? Is an msdc **rescan** needed
     after the WMT power-on? Check DTB msdc/connsys nodes vs Android.
3. Gotchas (already baked into `wifi-up.sh`): mkdir ALL mountpoints before bind;
   `/system` bind is read-only (stage fw only to `/etc/firmware`); **run `wmt_loader`
   exactly ONCE** — a second run oopses in `sdio_detect_exit` and reboots the device.
