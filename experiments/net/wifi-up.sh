#!/bin/sh
# wifi-up — CONSYS/Wi-Fi bring-up for L1. Encodes everything learned on-device 2026-07-19.
#
# HOW FAR THIS GETS (all verified live over serial):
#   * firmware staged at the REAL kernel paths (/etc/firmware — the WLAN RAM code path)
#   * wmt_loader runs (bionic, via the mounted stock /system linker) and DETECTS the chip:
#       "set current consys chipid (0x8127)" ... "mtk_stp_wmt driver(major 190) installed"
#       ... "[mtk_wmtd] wmtd thread starts"  — the full STP driver stack + mtk_wmtd come up
#   * /dev/stpwmt et al created (no udev on L1, so we mknod from /sys/class)
#   * wmtctl (our native tool) does SET_STP_MODE(SDIO)=OK, then FUNC_ON(WMT/WIFI)
#
# THE REMAINING BLOCKER (a layer below firmware — SDIO transport, not silicon, not power):
#   hif_sdio_stp_on: "no supported func probed"  ->  wmt_ctrl_stp_conf: "invalid Handle of
#   WmtStp"  ->  func-on fails. The connsys chip's SDIO FUNCTION never enumerates on the
#   msdc bus (`/sys/bus/sdio/devices` empty), so STP has no transport to attach to. On stock
#   Android the connsys SDIO card appears once the chip's SDIO interface powers up. Next
#   investigation: why 11230000.mmc doesn't enumerate the connsys function on L1 (msdc rescan
#   after WMT power-on / connsys SDIO-interface power rail / DTB msdc config vs Android).
#
# CRITICAL GOTCHAS (cost hours):
#   - mkdir ALL mountpoints (/mnt/system /system /vendor) BEFORE bind — a missing dir makes
#     `mount --bind` fail SILENTLY, and the bionic linker (/system/bin/linker) won't resolve.
#   - /system becomes a READ-ONLY bind of stock; stage firmware ONLY to /etc/firmware (RAM).
#   - RUN wmt_loader EXACTLY ONCE. A second run oopses (sdio_detect_exit double-unregister)
#     and REBOOTS the device.
#
#   sh /tmp/wifi-up.sh 2>&1
set +e
say(){ echo; echo "==== $* ===="; }

say "1. mount stock /system + bind (bionic linker + WMT tools + firmware)"
mkdir -p /mnt/system /system /vendor          # ALL of them, or bind fails silently
mount -t ext4 -o ro,noload /dev/mmcblk0p6 /mnt/system 2>/dev/null
mount --bind /mnt/system /system 2>/dev/null
mount --bind /mnt/system/vendor /vendor 2>/dev/null
export LD_LIBRARY_PATH=/system/lib:/vendor/lib
[ -e /system/bin/linker ] && echo "  linker OK" || { echo "  linker MISSING — abort"; exit 1; }

say "2. stage firmware at /etc/firmware (the hardcoded WLAN RAM path; /vendor/firmware is ro+ready)"
mkdir -p /etc/firmware
cp /mnt/system/vendor/firmware/* /etc/firmware/ 2>/dev/null
[ -s /etc/firmware/WIFI_RAM_CODE_8127 ] && echo "  WIFI_RAM_CODE_8127 OK" || echo "  *** RAM code MISSING ***"

say "3. wmt_loader — ONCE (detect chip, load STP stack, spawn mtk_wmtd)"
if [ ! -e /dev/stpwmt ] && ! grep -q mtk_stp_wmt /proc/devices 2>/dev/null; then
  dmesg -c >/dev/null 2>&1
  /vendor/bin/wmt_loader; echo "  loader ran (exit $?)"
  sleep 2
else
  echo "  STP driver already loaded — SKIP (never run wmt_loader twice: it oopses+reboots)"
fi

say "4. create /dev nodes from /sys/class (no udev on L1)"
for d in stpwmt stpbt stpgps wmtWifi wmtdetect; do
  dev=$(cat /sys/class/$d/$d/dev 2>/dev/null)
  [ -n "$dev" ] && [ ! -e /dev/$d ] && mknod /dev/$d c "${dev%:*}" "${dev#*:}" && echo "  + /dev/$d ($dev)"
done
echo "  mtk_wmtd: $(ps 2>/dev/null | grep -c '[m]tk_wmtd')   stpwmt: $(ls /dev/stpwmt 2>/dev/null || echo MISSING)"

say "5. wmtctl — SET_STP_MODE(SDIO) + FUNC_ON (our native launcher; stock wmt_launcher stalls)"
if [ -x /tmp/wm ]; then
  /tmp/wm > /tmp/wm.out 2>&1 & sleep 6
  cat /tmp/wm.out
else
  echo "  /tmp/wm (wmtctl_min) not pushed — build: arm-gcc -nostdlib -static -no-pie -Os wmtctl_min.c"
fi

say "6. RESULT"
dmesg 2>/dev/null | grep -iE 'ic info|SOC_CONSYS|patch dwn|no supported func|invalid Handle|wlan probe|WMT-FUNC' | tail -12
echo "  interfaces: $(ls /sys/class/net | tr '\n' ' ')"
echo "  wlan0: $([ -e /sys/class/net/wlan0 ] && echo UP || echo absent)"
echo "  sdio devices: $(ls /sys/bus/sdio/devices/ 2>/dev/null | tr '\n' ' ' || echo NONE)"
