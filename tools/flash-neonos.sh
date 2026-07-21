#!/usr/bin/env bash
# Flash the NEONWIRE OS boot image (L1 Linux + neui framebuffer UI at boot),
# or restore to a known-good fallback.
#
# Usage:
#   ./tools/flash-neonos.sh              # flash boot-linux-l1-neonos.img
#   ./tools/flash-neonos.sh restore      # restore known-good L1 (serial shell)
#   ./tools/flash-neonos.sh restore-stock  # restore stock Android boot
#
# Recovery ladder if the NEONOS image bootloops (Preloader every ~15s):
#   1) ./tools/flash-neonos.sh restore        -> plain L1, serial shell, no UI
#   2) ./tools/flash-neonos.sh restore-stock  -> stock Android
# Device must be FULLY powered off; Preloader handshake over USB.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SESSION="$ROOT/reference/dumps/session-20260718"
STOCK="$SESSION/images/boot.img"
STOCK_LOGO="$SESSION/images/logo.bin"
L1="$ROOT/experiments/linux-initramfs/out/boot-linux-l1.img"
NEONOS="$ROOT/experiments/linux-initramfs/out/boot-linux-l1-neonos.img"
NEONOS_LOGO="$ROOT/experiments/linux-initramfs/out/logo-neonos.bin"
OFFSET=0x1d80000    # boot partition (default)

mode="${1:-flash}"
case "$mode" in
  flash)         IMG="$NEONOS"; echo "=== FLASH NEONWIRE OS (L1 + neonwire Rust boot face, neui recovery) ===" ;;
  restore)       IMG="$L1";     echo "=== RESTORE known-good L1 (serial shell only) ===" ;;
  restore-stock) IMG="$STOCK";  echo "=== RESTORE stock Android boot ===" ;;
  logo)          IMG="$NEONOS_LOGO"; OFFSET=0x4400000; echo "=== FLASH NEONWIRE boot splash (logo partition) ===" ;;
  restore-logo)  IMG="$STOCK_LOGO";  OFFSET=0x4400000; echo "=== RESTORE stock DIGILAND logo ===" ;;
  *) echo "usage: $0 [flash|restore|restore-stock|logo|restore-logo]"; exit 1 ;;
esac

[[ -f "$IMG" ]] || { echo "missing $IMG"; [[ "$mode" == flash ]] && echo "Build: ./experiments/linux-initramfs/build_rootfs.sh && python3 experiments/linux-initramfs/pack_linux_boot.py --output $NEONOS"; exit 1; }

LEN=$(stat -c%s "$IMG"); LEN_HEX=$(printf '0x%x' "$LEN")
echo "image:  $IMG"
echo "size:   $LEN ($LEN_HEX)"
echo "offset: $OFFSET"
echo
echo "Procedure:"
echo "  1) Power tablet FULLY off, unplug USB"
echo "  2) Press Enter here — script waits for Preloader"
echo "  3) Plug USB (no buttons)"
echo "  4) After write OK: unplug, power on"
if [[ "$mode" == flash ]]; then
  echo
  echo "Expect: DIGILAND logo, then the NEONWIRE UI paints the screen."
  echo "Serial shell (0e8d:2007 /dev/ttyACM0) still comes up for recovery."
  echo "If it bootloops: $0 restore   (then $0 restore-stock)"
fi
echo
read -r -p "Press Enter to start mtkclient (needs sudo)..." _

source "$ROOT/tools/venv/bin/activate"
cd "$ROOT/tools/mtkclient"
sudo "$(which python)" mtk.py wo "$OFFSET" "$LEN_HEX" "$IMG"
echo
echo "Done. Unplug USB, power on the tablet."
