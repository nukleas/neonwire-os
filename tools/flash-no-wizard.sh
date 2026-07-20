#!/usr/bin/env bash
# Offline: flash system without SetupWizard + wipe userdata + optional ADB boot.
# Does NOT need an ADB allow dialog.
#
# Usage:
#   ./tools/flash-no-wizard.sh           # system + wipe data + boot-adb
#   ./tools/flash-no-wizard.sh system    # system only
#   ./tools/flash-no-wizard.sh data      # wipe data only
#   ./tools/flash-no-wizard.sh restore-system  # original system 4.bin
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SESSION="$ROOT/reference/dumps/session-20260718"
SYS_NEW="$SESSION/work/system-no-wizard/system.img"
SYS_OLD="$SESSION/raw/mbr-slots/4.bin"
BOOT_ADB="$SESSION/work/boot-adb/boot-adb.img"
BOOT_STOCK="$SESSION/images/boot.img"
MODE="${1:-all}"

source "$ROOT/tools/venv/bin/activate"
cd "$ROOT/tools/mtkclient"
MTK=(sudo "$(which python)" mtk.py)

echo "Mode: $MODE"
echo "Tablet must be OFF. You will plug when Preloader waits."
echo "Each mtk command may need its own unplug/off/plug cycle if handshake fails."
echo
read -r -p "Enter to continue..." _

flash_system() {
  local img=$1
  echo "=== write system partition 4: $img ==="
  "${MTK[@]}" w 4 "$img"
}

wipe_data() {
  echo "=== erase userdata (data) ==="
  "${MTK[@]}" e data
}

flash_boot() {
  local img=$1
  local len
  len=$(stat -c%s "$img")
  echo "=== write boot @ 0x1d80000 ($len bytes) ==="
  "${MTK[@]}" wo 0x1d80000 "$(printf '0x%x' "$len")" "$img"
}

case "$MODE" in
  all)
    flash_system "$SYS_NEW"
    echo "If device rebooted out of Preloader: power off, re-run wait, plug again for next step."
    wipe_data
    flash_boot "$BOOT_ADB"
    ;;
  system)
    flash_system "$SYS_NEW"
    ;;
  data)
    wipe_data
    ;;
  boot-adb)
    flash_boot "$BOOT_ADB"
    ;;
  restore-system)
    flash_system "$SYS_OLD"
    ;;
  restore-boot)
    flash_boot "$BOOT_STOCK"
    ;;
  *)
    echo "Unknown mode: $MODE"
    exit 1
    ;;
esac

echo
echo "Done. Unplug, power on. Expect home launcher without Google account."
echo "Then: adb devices  (hope for 'device' with embedded keys)"
