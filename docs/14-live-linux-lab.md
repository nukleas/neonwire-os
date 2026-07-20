# Live Linux lab notes (L1.3)

Stock **3.18.35** kernel + busybox initramfs + USB ACM console.  
Host: `picocom -b 115200 /dev/ttyACM0` or `./tools/serial-cmd.py 'cmd'`.

## Confirmed working

| Item | Status |
|------|--------|
| USB serial | `0e8d:2007 L1-Linux-ACM` → `/dev/ttyACM0` |
| Root shell | `dl7006#` uid=0 |
| eMMC | `/dev/mmcblk0` (~7.1 GiB user area) |
| SD | `/dev/mmcblk1` ~116 GiB vfat |
| Framebuffer | `/sys/class/graphics/fb0` **mtkfb** 32bpp, virtual 1024×1824, stride 4096 |
| LCM | `ZS070BE3019B3H7II_713` |

## Partition map (`mmcblk0`)

| Device | Size (approx) | Role |
|--------|----------------|------|
| `mmcblk0p1` | 1 KiB | (tiny / metadata) |
| `mmcblk0p2` | 10 MiB | boot-ish |
| `mmcblk0p3` | 10 MiB | recovery-ish |
| `mmcblk0p4` | 6 MiB | logo / misc class |
| `mmcblk0p5` | 1 MiB | small |
| **`mmcblk0p6`** | **1.5 GiB** | **`/system`** (ext4) |
| **`mmcblk0p7`** | **256 MiB** | **`/cache`** (ext4) |
| **`mmcblk0p8`** | **~5.2 GiB** | **`/data`** (ext4) |

No `by-name` nodes without Android `ueventd`; use p-numbers.

## Mount recipes (on device)

```sh
make-block-nodes
mkdir -p /mnt/system /mnt/data /mnt/cache /mnt/sd

mount -t ext4 -o ro,noload /dev/mmcblk0p6 /mnt/system
mount -t ext4 -o ro,noload /dev/mmcblk0p7 /mnt/cache
mount -t ext4 -o ro,noload /dev/mmcblk0p8 /mnt/data
# or RW:
# mount -t ext4 -o rw /dev/mmcblk0p8 /mnt/data

mount -t vfat -o rw /dev/mmcblk1p1 /mnt/sd
```

Firmware on system: `/mnt/system/vendor/firmware/WIFI_RAM_CODE_8127` etc.

## Host helpers

```bash
./tools/serial-cmd.py 'uname -a'
./tools/serial-cmd.py 'df'
# close picocom first
```

## Workspace

Prefer **SD** `/mnt/sd/linux-lab` for large downloads (Alpine rootfs, etc.) so Android data stays safer.  
Optional: `/mnt/data/linux-lab` on userdata.

## Wi‑Fi bring-up notes (in progress)

Android stack (bionic) **does** run under our busybox with `LD_LIBRARY_PATH` + `/system` bind:

| Step | Result |
|------|--------|
| `mount --bind /mnt/system /system` (+ vendor) | OK |
| `wmt_loader` | Runs; dmesg: `WLAN-GEN2 driver init, ret:0` |
| Char majors | `190 mtk_stp_wmt`, `153 mtk_wmt_wifi_chrdev`, `154` detect |
| Nodes | Create with `mknod` from `/sys/class/{stpwmt,wmtWifi,wmtdetect}/…/dev` |
| `wmt_launcher -p /tmp/fw/` | Starts; kernel thread **`mtk_wmtd`** runs |
| `echo 1 > /dev/wmtWifi` | **Still fails** — `opfunc_pwr_on` / `wmt_core_stp_init fail` |
| CONSYS | `18070000.consys` pm_runtime; VCN regulators exist but not user-toggleable |
| Firmware | copy to `/tmp/fw/` incl. `WMT.cfg` (= `WMT_SOC.cfg`) |

**Blocker:** connectivity chip **power path** — see below.

On-device helper (after mounts): `/mnt/sd/linux-lab/wifi-bringup.sh`

### CONSYS power-path diagnosis (2026-07-19)

Observed while `wmt_loader` + `wmt_launcher` + `echo 1 > /dev/wmtWifi`:

| Signal | Value / meaning |
|--------|------------------|
| `Read CONSYS chipId` | **`0x00000000`** (chip not on bus; expect `0x8127`) |
| `connsys_bus` clk | `enable_count=0`, `rate=0` — **bus clock never enabled** |
| VCN regulators | `vcn18` / `vcn28` / `vcn33_wifi` **use_count=0 open_count=0** |
| Userspace regulator | **No** `enable` file (consumer-only; only kernel driver can enable) |
| `pm_runtime_get_sync` | **`fail(1)`** when forced `power/control=on` — return `1` means “already active”, MTK treats as error |
| After unbind/rebind | `power/runtime_status` → **`unsupported`** (RPM path degraded) |
| Final userspace | `WIFI_write: WMT turn on WIFI fail!` / `wmt_core_stp_init fail` |

**Interpretation:** The WMT stack and char-devs are fine. **Rails + MTCMOS + `connsys_bus` clock are not actually coming up**, so chip ID stays 0 and STP cannot start.

Amazon/GPL reference (`mtk_wcn_consys_hw.c`) power-on order (older API, same idea):

1. PMIC: **VCN_1V8** then **VCN28** (if `co_clock_flag=0` — our `WMT_SOC.cfg`)  
2. **`conn_power_on()`** MTCMOS (scpsys)  
3. Enable **infra CONNMCU / connsys bus** clock  
4. Poll chip ID until `0x8127`  
5. GPIO / BGF EINT  

Our `WMT_SOC.cfg`: `co_clock_flag=0` → SW control of VCN28 expected.

**Likely fix paths (not yet done):**

1. **Kernel-side:** ensure consys probe holds regulators + genpd; fix `if (pm_runtime_get_sync())` to `if (... < 0)`; never leave RPM in a half-on state.  
2. **Userspace workaround:** none for VCN (no enable sysfs) unless we add a tiny kernel helper or poke PMIC via a privileged interface (none found yet; only `/dev/MT_pmic_adc_cali`).  
3. **Heavier:** run more of Android `init` connectivity services in a bionic mini-env (still needs kernel power_on to succeed).

**Do not** leave `power/control=on` on `18070000.consys` while debugging — it makes `get_sync()` return 1 and the driver aborts.

Launcher usage (from binary strings):

```text
wmt_launcher -p /vendor/firmware/
# or -p /system/etc/firmware/
```

Expected after full bring-up (Android):

```sh
echo 1 > /dev/wmtWifi   # create wlan0
# then wpa_supplicant / iwconfig
```

`ueventd.mt8127.rc` permissions: `/dev/stpwmt`, `/dev/wmtWifi`, `/dev/wmtdetect` — nodes are kernel-created; we may need full WMT probe (power/regulators) before `stpwmt` appears.

**Careful:** heavy WMT attempts once dropped USB ACM briefly (device recovered as `0e8d:2007` again). Prefer short commands; power-cycle if shell goes silent (no unplug required).

### Suggested on-device sequence (when shell healthy)

```sh
export PATH=/system/bin:/vendor/bin:$PATH
export LD_LIBRARY_PATH=/system/lib:/vendor/lib
mkdir -p /system /vendor /system/etc/firmware /data
mount --bind /mnt/system /system
mount --bind /mnt/system/vendor /vendor
mount --bind /mnt/data /data
ln -sf /vendor/firmware/* /system/etc/firmware/
cp /vendor/firmware/WMT_SOC.cfg /vendor/firmware/WMT.cfg
wmt_loader
# check: ls /dev/stpwmt /dev/wmtWifi /dev/wmtdetect
# if stpwmt exists:
wmt_launcher -p /vendor/firmware/ &
sleep 2
echo 1 > /dev/wmtWifi
ls /sys/class/net
```

## Next targets

1. Get `/dev/stpwmt` + `wlan0` up  
2. Framebuffer solid-color / simple UI on `mtkfb`  
3. Alpine armv7 rootfs on SD + chroot  
4. Avoid reflash unless initramfs must change
