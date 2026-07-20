#!/usr/bin/env bash
# Flash patched boot-adb.img OR restore stock boot.
# Usage:
#   ./tools/flash-boot-adb.sh          # flash adb-patched boot
#   ./tools/flash-boot-adb.sh restore  # restore stock boot.img
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SESSION="$ROOT/reference/dumps/session-20260718"
STOCK="$SESSION/images/boot.img"
PATCHED="$SESSION/work/boot-adb/boot-adb.img"
OFFSET=0x1d80000

mode="${1:-flash}"
if [[ "$mode" == "restore" ]]; then
  IMG="$STOCK"
  echo "=== RESTORE stock boot ==="
else
  IMG="$PATCHED"
  echo "=== FLASH patched boot-adb ==="
fi

[[ -f "$IMG" ]] || { echo "missing $IMG"; exit 1; }
LEN=$(stat -c%s "$IMG")
LEN_HEX=$(printf '0x%x' "$LEN")
echo "image:  $IMG"
echo "size:   $LEN ($LEN_HEX)"
echo "offset: $OFFSET"
echo
echo "mtkclient syntax: wo <offset> <length> <filename>"
echo
echo "1) Power tablet FULLY off, unplug USB"
echo "2) This script will wait for Preloader"
echo "3) Plug USB (no buttons) when 'Waiting' appears"
echo
read -r -p "Press Enter to start mtkclient (needs sudo)..." _

source "$ROOT/tools/venv/bin/activate"
cd "$ROOT/tools/mtkclient"
# wo requires: offset length filename
sudo "$(which python)" mtk.py wo "$OFFSET" "$LEN_HEX" "$IMG"
echo
echo "Done. Unplug, power on, enable USB debugging if prompted."
echo "Test: adb devices"
echo "Restore: $0 restore"
