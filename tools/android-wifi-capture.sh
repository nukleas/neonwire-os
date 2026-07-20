#!/usr/bin/env bash
# Host helper: push capture script to stock Android, run it, pull results.
#
# Prerequisites: tablet on **stock Android**, USB debugging / adb.
#   ./tools/android-wifi-capture.sh
#
# Device is currently often on L1 (0e8d:2007) — restore stock boot first:
#   ./tools/flash-linux-l1.sh restore
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SCRIPT="$ROOT/experiments/net/android-wifi-toggle-capture.sh"
OUT_HOST="$ROOT/reference/probe/android-wifi-toggle"
REMOTE_SH=/data/local/tmp/android-wifi-toggle-capture.sh
REMOTE_OUT=/sdcard/wifi-toggle-capture

if ! command -v adb >/dev/null; then
  echo "adb not found"; exit 1
fi

echo "=== adb devices ==="
adb devices -l
state="$(adb get-state 2>/dev/null || true)"
if [[ "$state" != "device" ]]; then
  echo "No Android adb device. Current USB may be L1 ACM (0e8d:2007)."
  echo "Flash stock boot, enable USB debugging, re-run."
  exit 2
fi

mkdir -p "$OUT_HOST"
adb push "$SCRIPT" "$REMOTE_SH"
adb shell "chmod 755 $REMOTE_SH"

# prefer root for dmesg -C / debugfs
adb root 2>/dev/null || true
sleep 1
adb shell "sh $REMOTE_SH" || true

stamp="$(date +%Y%m%d-%H%M%S)"
dest="$OUT_HOST/$stamp"
mkdir -p "$dest"
adb pull "$REMOTE_OUT" "$dest/" || adb pull /sdcard/wifi-toggle-capture "$dest/"
echo "Saved under $dest"
ls -la "$dest" || true
echo
echo "Read: $dest/*/SUMMARY.txt  and  logcat-wifi.txt / dmesg-wifi-filtered.txt"
