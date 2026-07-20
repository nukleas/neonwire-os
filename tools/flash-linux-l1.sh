#!/usr/bin/env bash
# Flash L1 Linux boot (stock kernel + busybox initramfs) OR restore stock Android boot.
#
# Usage:
#   ./tools/flash-linux-l1.sh          # flash boot-linux-l1.img
#   ./tools/flash-linux-l1.sh restore  # restore stock boot.img
#
# Flash offset: 0x1d80000 (boot partition)
# Device must be FULLY powered off; Preloader handshake via USB.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SESSION="$ROOT/reference/dumps/session-20260718"
STOCK="$SESSION/images/boot.img"
LINUX="$ROOT/experiments/linux-initramfs/out/boot-linux-l1.img"
OFFSET=0x1d80000

mode="${1:-flash}"
if [[ "$mode" == "restore" ]]; then
  IMG="$STOCK"
  echo "=== RESTORE stock Android boot ==="
elif [[ "$mode" == "flash" ]]; then
  IMG="$LINUX"
  echo "=== FLASH L1 Linux boot (busybox initramfs) ==="
else
  echo "usage: $0 [flash|restore]"
  exit 1
fi

[[ -f "$IMG" ]] || {
  echo "missing $IMG"
  if [[ "$mode" == "flash" ]]; then
    echo "Build first:"
    echo "  ./experiments/linux-initramfs/build_rootfs.sh"
    echo "  python3 experiments/linux-initramfs/pack_linux_boot.py"
  fi
  exit 1
}

LEN=$(stat -c%s "$IMG")
LEN_HEX=$(printf '0x%x' "$LEN")
echo "image:  $IMG"
echo "size:   $LEN ($LEN_HEX)"
echo "offset: $OFFSET"
echo
echo "mtkclient: wo <offset> <length> <filename>"
echo
echo "Procedure:"
echo "  1) Power tablet FULLY off, unplug USB"
echo "  2) Press Enter here — script waits for Preloader"
echo "  3) Plug USB (no buttons)"
echo "  4) After write OK: unplug, power on"
echo
if [[ "$mode" == "flash" ]]; then
  echo "Expect: black screen or logo freeze without UART; stock kernel may still"
  echo "        bring up framebuffer. Success = init shell on console."
  echo "Restore anytime: $0 restore"
  echo
fi
read -r -p "Press Enter to start mtkclient (needs sudo)..." _

source "$ROOT/tools/venv/bin/activate"
cd "$ROOT/tools/mtkclient"
sudo "$(which python)" mtk.py wo "$OFFSET" "$LEN_HEX" "$IMG"
echo
echo "Done. Unplug USB, power on the tablet."
if [[ "$mode" == "flash" ]]; then
  echo "If it bootloops (Preloader every ~15s), restore:"
  echo "  $0 restore"
fi
