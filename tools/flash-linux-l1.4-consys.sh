#!/usr/bin/env bash
# Flash L1.4: stock kernel with mtk_wcn_consys_power_on SPM patch + busybox initramfs
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
IMG="$ROOT/experiments/linux-initramfs/out/boot-linux-l1.4-consys.img"
STOCK="$ROOT/reference/dumps/session-20260718/images/boot.img"
OFFSET=0x1d80000
mode="${1:-flash}"
if [[ "$mode" == "restore" ]]; then IMG="$STOCK"; echo "=== RESTORE stock ===";
elif [[ "$mode" == "flash" ]]; then echo "=== FLASH L1.4 consys power patch ===";
else echo "usage: $0 [flash|restore]"; exit 1; fi
[[ -f "$IMG" ]] || { echo "missing $IMG"; exit 1; }
LEN=$(stat -c%s "$IMG"); LEN_HEX=$(printf '0x%x' "$LEN")
echo "image: $IMG ($LEN / $LEN_HEX) @ $OFFSET"
echo "Power tablet FULLY OFF, unplug USB, press Enter, then plug USB (Preloader)."
read -r -p "Enter to flash..." _
source "$ROOT/tools/venv/bin/activate"
cd "$ROOT/tools/mtkclient"
sudo "$(which python)" mtk.py wo "$OFFSET" "$LEN_HEX" "$IMG"
echo "Done. Unplug, power on. Expect L1 shell; then test Wi-Fi power path."
