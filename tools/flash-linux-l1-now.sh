#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
source "$ROOT/tools/venv/bin/activate"
IMG="$ROOT/experiments/linux-initramfs/out/boot-linux-l1.img"
[[ -f "$IMG" ]] || { echo "missing $IMG"; exit 1; }
LEN_HEX=$(printf '0x%x' $(stat -c%s "$IMG"))
echo "Waiting for Preloader — power OFF tablet, unplug, plug USB (no buttons)..."
echo "Image: $IMG ($LEN_HEX) @ 0x1d80000"
cd "$ROOT/tools/mtkclient"
python mtk.py wo 0x1d80000 "$LEN_HEX" "$IMG"
echo "Done. Unplug, power on. Restore: $ROOT/tools/flash-linux-l1.sh restore"
