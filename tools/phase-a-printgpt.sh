#!/usr/bin/env bash
# Phase A: wait for Preloader and print GPT.
# Usage:
#   1. Unplug tablet, hold Power ~15s so it is fully off
#   2. Run this script
#   3. When it says Waiting..., plug USB (no buttons first)
#   4. If handshake fails, unplug, re-run, hold Vol Up while plugging
set -euo pipefail
ROOT="$(cd "$(dirname "$0")" && pwd)"
# shellcheck disable=SC1091
source "$ROOT/venv/bin/activate"
cd "$ROOT/mtkclient"

DUMP_DIR="$ROOT/../reference/dumps"
mkdir -p "$DUMP_DIR"
OUT="$DUMP_DIR/printgpt-$(date +%Y%m%d-%H%M%S).txt"

echo "=== Phase A printgpt ==="
echo "Output: $OUT"
echo
echo "1) Tablet must be FULLY off and UNPLUGGED now."
echo "2) When you see 'Waiting for PreLoader', plug USB (no buttons)."
echo "3) Leave it alone for ~30s."
echo
sleep 2

# Prefer stock DA path on old MTK; fall back handled by retries inside mtk
set +e
python mtk.py printgpt --stock 2>&1 | tee "$OUT"
rc=${PIPESTATUS[0]}
set -e

if [[ $rc -ne 0 ]] || rg -q 'Handshake failed|Couldn.t detect' "$OUT"; then
  echo
  echo "=== Retry without --stock ==="
  python mtk.py printgpt 2>&1 | tee -a "$OUT"
  rc=${PIPESTATUS[0]}
fi

echo
echo "Exit code: $rc"
echo "Log: $OUT"
exit "$rc"
