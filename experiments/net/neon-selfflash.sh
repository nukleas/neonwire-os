#!/bin/sh
# neon-selfflash — OTA the KERNEL + INITRAMFS from the running Linux itself.
# No Preloader, no cable: root can write the eMMC boot region directly. This is
# the ESP32-style OTA for the part that isn't already covered by neon-sync.sh
# (which handles the userland: UI, wifi, ssh, tools).
#
#   sh neon-selfflash.sh /mnt/sd/linux-lab/boot-new.img       # verify + backup + flash
#   sh neon-selfflash.sh --restore                            # re-flash the backup we saved
#
# SAFETY (read carefully — this writes raw eMMC):
#   * The boot image is validated (ANDROID! magic, sane size) before ANYTHING.
#   * The CURRENT boot region is backed up to SD first (rollback image).
#   * After writing, the region is READ BACK and sha256-verified against the
#     source. We only report success on a byte-exact match. Do NOT reboot on
#     a failed verify.
#   * Ultimate backstop is unchanged: a bad image bootloops -> recover over the
#     Preloader with mtkclient (tools/flash-neonos.sh restore). The SD backup
#     is the image to restore.
#
# Boot region (from /proc/dumchar_info): bootimg @ 0x1d80000, size 0x1000000.
DEV=/dev/mmcblk0
SEEK=60416            # 0x1d80000 / 512
MAXSECT=32768         # 0x1000000 / 512  (16 MiB partition ceiling)
LAB=/mnt/sd/linux-lab
BACKUP=$LAB/boot-backup.img

fail(){ echo "!! $*"; exit 1; }

if [ "$1" = "--restore" ]; then
  [ -s "$BACKUP" ] || fail "no backup at $BACKUP"
  IMG=$BACKUP
  echo "== restoring saved boot image =="
else
  IMG=$1
  [ -s "$IMG" ] || fail "usage: $0 <boot.img> | --restore   (image not found)"
fi

# --- validate the image ---
magic=$(dd if="$IMG" bs=8 count=1 2>/dev/null | od -A n -t x1 | tr -d ' \n')
[ "$magic" = "414e44524f494421" ] || fail "not an ANDROID! boot image (magic=$magic)"
bytes=$(( $(wc -c < "$IMG") ))
sect=$(( (bytes + 511) / 512 ))
[ "$sect" -le "$MAXSECT" ] || fail "image too big ($bytes B > 16 MiB partition)"
srcsha=$(sha256sum "$IMG" | cut -d' ' -f1)
echo "   image: $bytes bytes ($sect sectors), sha $srcsha"

# --- back up the current boot region (rollback image) unless we're restoring ---
if [ "$1" != "--restore" ]; then
  echo "== backing up current boot region -> $BACKUP =="
  dd if=$DEV of="$BACKUP" bs=512 skip=$SEEK count=$sect 2>/dev/null || fail "backup read failed"
  echo "   backed up $sect sectors"
fi

# --- write ---
echo "== writing to $DEV @ sector $SEEK =="
dd if="$IMG" of=$DEV bs=512 seek=$SEEK conv=fsync 2>/dev/null || fail "write failed"

# --- read back + verify (only the image's own length) ---
echo "== verifying read-back =="
rbsha=$(dd if=$DEV bs=512 skip=$SEEK count=$sect 2>/dev/null | dd bs=1 count=$bytes 2>/dev/null | sha256sum | cut -d' ' -f1)
if [ "$rbsha" = "$srcsha" ]; then
  echo "== OK: boot region matches the image byte-for-byte =="
  echo "   safe to reboot. If it bootloops: $0 --restore over Preloader, or"
  echo "   tools/flash-neonos.sh restore-stock from the host."
else
  echo "!! VERIFY FAILED (read-back sha != source). DO NOT REBOOT."
  echo "   src=$srcsha"
  echo "   got=$rbsha"
  echo "   the old boot may be damaged — reflash $BACKUP over Preloader if needed."
  exit 2
fi
