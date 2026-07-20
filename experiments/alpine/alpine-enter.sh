#!/bin/sh
# alpine-enter — drop into the Alpine armv7 dev chroot on /data (ext4).
# Built on the host with docker (arm32v7/alpine:3.12, musl 1.1 = 3.18-kernel safe),
# transferred over the RNDIS USB-net link, extracted to /mnt/data/alpine.
#
#   sh /mnt/data/alpine-enter.sh          # interactive shell inside Alpine
#   sh /mnt/data/alpine-enter.sh -c 'CMD' # run one command in the chroot
A=/mnt/data/alpine

# ensure /data (ext4) is mounted and the rootfs is there
[ -d "$A/usr/bin" ] || { mkdir -p /mnt/data; mount -t ext4 -o rw /dev/mmcblk0p8 /mnt/data 2>/dev/null; }
[ -d "$A/usr/bin" ] || { echo "no Alpine rootfs at $A"; exit 1; }

# bind the live kernel interfaces into the chroot
mount -t proc  proc   "$A/proc"    2>/dev/null
mount -t sysfs sys     "$A/sys"     2>/dev/null
mount --bind   /dev    "$A/dev"     2>/dev/null
mkdir -p "$A/dev/pts"; mount -t devpts devpts "$A/dev/pts" 2>/dev/null
mkdir -p "$A/media/sd"; mount --bind /mnt/sd "$A/media/sd" 2>/dev/null
# resolv.conf so DNS works if the host shares internet over RNDIS
[ -f /etc/resolv.conf ] && cp /etc/resolv.conf "$A/etc/resolv.conf" 2>/dev/null
echo "nameserver 192.168.42.10" > "$A/etc/resolv.conf" 2>/dev/null

export TERM=${TERM:-linux}
if [ "$1" = "-c" ]; then
  shift; exec chroot "$A" /bin/bash -lc "$*"
else
  echo "[ NEONWIRE // Alpine dev chroot — $(chroot "$A" cat /etc/alpine-release) armv7 ]"
  echo "  python3 / gcc / git / vim / tmux / sqlite3 / lua5.3   ·   /media/sd = SD card"
  exec chroot "$A" /bin/bash -l
fi
