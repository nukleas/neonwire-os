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

say "0. loopback + SD (persistent tools)"
# lo is DOWN on this minimal init — nothing needed it until we ran a local
# service. Without it, binding 127.0.0.1 fails EADDRNOTAVAIL (cost us a
# confusing 'Address not available' from the zeroclaw gateway, 2026-07-21).
ifconfig lo 127.0.0.1 netmask 255.0.0.0 up 2>/dev/null
mount -t vfat -o rw /dev/mmcblk1p1 /mnt/sd 2>/dev/null
[ -x $LAB/wmtctl2 ] || { echo "  *** $LAB/wmtctl2 missing — push it first"; exit 1; }

say "0b. /mnt/data (ext4 userdata: Alpine dev chroot)"
# Deliberately BEFORE the wifi steps — they can exit 1, and the chroot must not
# depend on wifi. (2026-07-20: this mount was missing entirely; after a reboot
# the Music app played into a powered-off amp because speaker_amp() shelled out
# to the chroot's amixer. neonwire now sets the mixer natively, but the chroot
# still lives here.)
mkdir -p /mnt/data
grep -q " /mnt/data " /proc/mounts || mount -t ext4 /dev/mmcblk0p8 /mnt/data 2>/dev/null
grep -q " /mnt/data " /proc/mounts && echo "  /mnt/data OK" || echo "  *** mmcblk0p8 mount FAILED"

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

say "6. wpa_supplicant (static musl, nl80211 driver)"
# NB: use nl80211, NOT wext — wext associates but never completes the 4-way
# handshake on this MT8127 wl chip. The wpas on SD is built with nl80211+libnl-tiny.
ifconfig wlan0 up
if ! ps | grep -q "[w]pas"; then
  CONF=/tmp/wpa.conf
  if [ -s $LAB/wpa.conf ]; then cp $LAB/wpa.conf $CONF; else printf "ctrl_interface=/tmp/wpa\nupdate_config=1\n" > $CONF; fi
  $LAB/wpas -iwlan0 -Dnl80211 -c$CONF -B
fi

say "7. join"
if grep -q ssid /tmp/wpa.conf 2>/dev/null; then
  sh $LAB/wifi-join.sh          # saved network -> associate + DHCP + telnetd
else
  echo "  no saved network. Scan: $LAB/wpa_cli -p /tmp/wpa -i wlan0 scan"
  echo "  then: sh $LAB/wifi-join.sh \"SSID\" \"passphrase\""
fi

say "8. UI — prefer live-updated neonwire from SD (camera stream etc.)"
# Boot init may start the initramfs /bin/neonwire first. After SD is up, bind the
# SD copy over it and restart so the latest UI runs without reflashing boot.
# IMPORTANT: file bind-mounts pin an inode. After replacing $LAB/neonwire on the
# SD, we must umount + remount or the tablet keeps running the old binary
# (control plane silently missing → neonctl timeouts).
if [ -x $LAB/neonwire ]; then
  killall neonwire camgrab 2>/dev/null
  if mount | grep -q "on /bin/neonwire "; then
    umount /bin/neonwire 2>/dev/null && echo "  unbound old /bin/neonwire inode"
  fi
  mount --bind $LAB/neonwire /bin/neonwire 2>/dev/null && echo "  bind SD neonwire -> /bin/neonwire" \
    || echo "  bind failed — init may use stale /bin/neonwire"
  [ -f /tmp/camgrab_exp ] || echo "4000 128" > /tmp/camgrab_exp
  echo "  neonwire restart requested (init respawns in ~2s)"
else
  echo "  $LAB/neonwire missing — leaving baked-in UI"
fi

say "9. zeroclaw — on-device AI agent daemon (127.0.0.1:42617)"
# Separate process; the neonwire ASSISTANT app POSTs to /webhook. Needs lo (see
# step 0) and a provider configured in $LAB/zeroclaw/cfg. Detached so a failure
# here never blocks the UI.
#
# `daemon`, NOT `gateway start`: the heartbeat worker (periodic log triage, see
# cfg/agents/watcher/workspace/HEARTBEAT.md) only runs under `daemon`, which is
# gateway + channels + heartbeat + scheduler. `gateway start` serves /webhook
# but never ticks the heartbeat — that cost us a silent no-op (2026-07-22).
ZC=$LAB/zeroclaw/zeroclaw
if [ -x "$ZC" ]; then
  if ps | grep -q "[z]eroclaw"; then
    echo "  already running"
  else
    setsid "$ZC" --config-dir $LAB/zeroclaw/cfg daemon \
      >/tmp/zc.log 2>&1 </dev/null &
    echo "  daemon launched (log /tmp/zc.log)"
  fi
else
  echo "  $ZC not present — skipping"
fi
