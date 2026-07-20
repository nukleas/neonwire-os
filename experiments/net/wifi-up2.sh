#!/bin/sh
# wifi-up2 — CONSYS Wi-Fi bring-up for L1, the WORKING sequence (2026-07-19).
# Everything persistent lives on the SD: /mnt/sd/linux-lab/{wmtctl2,wpas,wpa_cli,
# udhcpc.script,wifi-join.sh,wpa.conf}.  Run this once per boot, then wifi-join.sh.
#
# What was wrong before (see docs/21 + wmtctl2.c):
#   * STP mode must be BTIF (0x23), not SDIO (0x24) — MT8127 consys is on-die, no SDIO func.
#   * The kernel asks USERSPACE for patch info ("srh_patch" on /dev/stpwmt): must answer
#     with SET_PATCH_NUM/SET_PATCH_INFO ioctls + write "ok" back. wmtctl2 does all of it.
#
# GOTCHAS (cost hours, do not regress):
#   - mkdir ALL mountpoints before bind; /system bind is READ-ONLY (stage fw to /etc/firmware).
#   - RUN wmt_loader EXACTLY ONCE per boot (2nd run oopses in sdio_detect_exit -> reboot).
set +e
LAB=/mnt/sd/linux-lab
say(){ echo; echo "==== $* ===="; }

say "0. SD (persistent tools)"
mount -t vfat -o rw /dev/mmcblk1p1 /mnt/sd 2>/dev/null
[ -x $LAB/wmtctl2 ] || { echo "  *** $LAB/wmtctl2 missing — push it first"; exit 1; }

say "1. mount stock /system + bind (bionic linker + wmt_loader + firmware)"
mkdir -p /mnt/system /system /vendor
mount -t ext4 -o ro,noload /dev/mmcblk0p6 /mnt/system 2>/dev/null
mount --bind /mnt/system /system 2>/dev/null
mount --bind /mnt/system/vendor /vendor 2>/dev/null
export LD_LIBRARY_PATH=/system/lib:/vendor/lib
[ -e /system/bin/linker ] && echo "  linker OK" || { echo "  linker MISSING — abort"; exit 1; }

say "2. stage firmware at /etc/firmware (hardcoded kernel path)"
mkdir -p /etc/firmware
cp /mnt/system/vendor/firmware/* /etc/firmware/ 2>/dev/null
[ -s /etc/firmware/WIFI_RAM_CODE_8127 ] && echo "  WIFI_RAM_CODE_8127 OK" || echo "  *** RAM code MISSING ***"

say "3. wmt_loader — ONCE (detect chip 0x8127, install STP driver stack)"
if [ ! -e /dev/stpwmt ] && ! grep -q mtk_stp_wmt /proc/devices 2>/dev/null; then
  /vendor/bin/wmt_loader; echo "  loader ran (exit $?)"
  sleep 2
else
  echo "  STP driver already loaded — SKIP (never run twice)"
fi

say "4. /dev nodes (no udev on L1)"
for d in stpwmt stpbt stpgps wmtWifi wmtdetect; do
  dev=$(cat /sys/class/$d/$d/dev 2>/dev/null)
  [ -n "$dev" ] && [ ! -e /dev/$d ] && mknod /dev/$d c "${dev%:*}" "${dev#*:}" && echo "  + /dev/$d"
done

say "5. wmtctl2 — BTIF mode, patch registration, srh_patch responder, FUNC_ON(WMT+WIFI)"
if ! ps | grep -q "[w]mtctl2"; then
  $LAB/wmtctl2 > /tmp/wmtctl2.out 2>&1 &
  sleep 10
fi
cat /tmp/wmtctl2.out 2>/dev/null
[ -e /sys/class/net/wlan0 ] && echo "  wlan0 OK" || { echo "  *** wlan0 absent"; dmesg | grep -iE "wmt|patch" | tail -8; exit 1; }

say "6. wpa_supplicant (static musl, WEXT driver)"
ifconfig wlan0 up
if ! ps | grep -q "[w]pas"; then
  CONF=/tmp/wpa.conf
  if [ -s $LAB/wpa.conf ]; then cp $LAB/wpa.conf $CONF; else printf "ctrl_interface=/tmp/wpa\nupdate_config=1\n" > $CONF; fi
  $LAB/wpas -iwlan0 -Dwext -c$CONF -B
fi

say "7. join"
if grep -q ssid /tmp/wpa.conf 2>/dev/null; then
  sh $LAB/wifi-join.sh          # saved network -> associate + DHCP + telnetd
else
  echo "  no saved network. Scan: $LAB/wpa_cli -p /tmp/wpa -i wlan0 scan"
  echo "  then: sh $LAB/wifi-join.sh \"SSID\" \"passphrase\""
fi
