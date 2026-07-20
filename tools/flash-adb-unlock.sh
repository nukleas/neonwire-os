#!/usr/bin/env bash
# Fix unauthorized ADB: reflash system (adb.secure=0) + boot (keys + init copy to /data)
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SESSION="$ROOT/reference/dumps/session-20260718"
SYS="$SESSION/work/system-no-wizard/system.img"
BOOT="$SESSION/work/boot-adb/boot-adb.img"

source "$ROOT/tools/venv/bin/activate"
cd "$ROOT/tools/mtkclient"

echo "=== hashes ==="
sha256sum "$SYS" "$BOOT"
echo
echo "Two Preloader cycles. OFF + unplug before each."
echo
read -r -p "Enter for SYSTEM write (partition 4)..." _
sudo "$(which python)" mtk.py w 4 "$SYS"
echo
echo "Power OFF + unplug again."
read -r -p "Enter for BOOT write..." _
sudo "$(which python)" mtk.py wo 0x1d80000 "$(printf '0x%x' "$(stat -c%s "$BOOT")")" "$BOOT"
echo
echo "Unplug, power on, wait 2 min, then: adb kill-server; adb devices"
