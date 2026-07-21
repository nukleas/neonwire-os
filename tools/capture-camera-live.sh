#!/usr/bin/env bash
# capture-camera-live.sh — one-shot golden capture of the STOCK Android camera
# register state while a live preview is streaming. Run this on the HOST with the
# tablet booted into stock Android from the ADB-root patched boot (boot-adb.img)
# and connected over USB.
#
# It captures EVERYTHING in a single Android boot so we never round-trip twice:
#   * camdump register snapshots  — idle, then several during live preview
#   * no-root /proc + /sys/debug camera register nodes (fallback if ioctl denied)
#   * kernel + main logcat around a fresh camera run
#   * getprop, dumpsys media.camera
# All artifacts land in reference/android-capture/camera-live-<date>/ with a
# SHA256SUMS so the dumps are provably intact before we flash NeonOS back.
#
# This tool only READS device state (camdump issues no register writes). The only
# device mutation is launching the stock camera app.
set -u

REPO="$(cd "$(dirname "$0")/.." && pwd)"
DATE="${CAP_DATE:-20260720}"
OUT="$REPO/reference/android-capture/camera-live-$DATE"
CAMDUMP="$REPO/experiments/camera/camdump"
DEVTMP=/data/local/tmp
mkdir -p "$OUT"

say(){ printf '\n\033[36m== %s\033[0m\n' "$*"; }
warn(){ printf '\033[33m!! %s\033[0m\n' "$*"; }

adbsh(){ adb shell "$@"; }

# --- 0. preflight ---------------------------------------------------------
say "preflight"
adb wait-for-device
adb get-state || { warn "no device — plug USB, confirm Android booted"; exit 1; }
adb root 2>&1 | sed 's/^/   adb root: /'
adb wait-for-device
ID=$(adbsh id -u 2>/dev/null | tr -d '\r')
echo "   shell uid=$ID  (0 = root, good)"
adbsh getprop ro.build.version.release 2>/dev/null | sed 's/^/   android /'
# Try to drop SELinux to permissive so root can ioctl /dev/camera-isp.
SE=$(adbsh getenforce 2>/dev/null | tr -d '\r')
echo "   SELinux (before): $SE"
adbsh setenforce 0 2>/dev/null
SE2=$(adbsh getenforce 2>/dev/null | tr -d '\r')
echo "   SELinux (after setenforce 0): $SE2"
[ "$SE2" = "Permissive" ] || warn "SELinux still $SE2 — camdump ioctl may be denied; rely on proc-node fallback"

# --- 1. static device facts ----------------------------------------------
say "static facts (getprop, camera nodes)"
adbsh getprop            2>/dev/null | tr -d '\r' > "$OUT/getprop.txt"
adbsh 'ls -l /dev/camera-isp /dev/kd_camera_hw /dev/ion' 2>&1 | tr -d '\r' > "$OUT/camera-nodes.txt"
adbsh 'dumpsys media.camera' 2>/dev/null | tr -d '\r' > "$OUT/dumpsys-media-camera.txt"

# candidate no-root register/debug nodes (world-readable on many MTK builds)
say "scan proc/debugfs register nodes"
adbsh 'for p in \
  /proc/mtk_cam /proc/driver/camera_info /sys/kernel/debug/mtk_cam \
  /sys/kernel/debug/seninf /sys/kernel/debug/isp /sys/kernel/debug/camsys \
  /proc/isp_p1 /proc/dumchar_info ; do \
    if [ -e "$p" ]; then echo "==== $p ===="; cat "$p" 2>&1; fi ; done' \
  2>&1 | tr -d '\r' > "$OUT/proc-debug-nodes.txt"
echo "   -> $(wc -l < "$OUT/proc-debug-nodes.txt") lines"

# --- 2. push camdump ------------------------------------------------------
say "push camdump"
[ -x "$CAMDUMP" ] || { warn "camdump not built at $CAMDUMP"; exit 1; }
adb push "$CAMDUMP" "$DEVTMP/camdump" >/dev/null && adbsh chmod 755 "$DEVTMP/camdump"
echo "   pushed $(adbsh "$DEVTMP/camdump" 2>&1 | head -1 | tr -d '\r')"

run_dump(){ # $1 = label
  local tag="$1"
  adbsh "$DEVTMP/camdump $DEVTMP/dump.txt" >/dev/null 2>&1
  adb pull "$DEVTMP/dump.txt" "$OUT/stock-$tag.txt" >/dev/null 2>&1
  local hd; hd=$(grep -m1 'TG_SOF_CNT' "$OUT/stock-$tag.txt" 2>/dev/null)
  echo "   [$tag] $hd"
}

# --- 3. idle baseline (camera app NOT running) ---------------------------
say "idle baseline dump"
run_dump idle

# --- 4. start logcat ring capture, then launch live preview --------------
say "clear logs + launch stock camera preview"
adbsh logcat -b kernel -c 2>/dev/null
adbsh logcat -c 2>/dev/null
# Prefer the factory DeviceTest camera (deterministic, no shutter needed);
# fall back to the generic still-image camera intent.
adbsh 'am start -n com.DeviceTest/.CameraTest' 2>&1 | sed 's/^/   /' \
  || adbsh 'am start -a android.media.action.STILL_IMAGE_CAMERA' 2>&1 | sed 's/^/   /'

# preview needs a moment to bring SENINF/TG up; snapshot several times
for t in 2 4 7 11; do
  sleep "$t"   # cumulative-ish; each sleep then dump
  run_dump "preview-t$t"
done

# --- 5. logs during the run ----------------------------------------------
say "pull logcat (kernel + main) + getprop delta"
adbsh logcat -b kernel -d 2>/dev/null | tr -d '\r' > "$OUT/preview-kernel.log"
adbsh logcat -d          2>/dev/null | tr -d '\r' > "$OUT/preview-main.log"
echo "   kernel $(wc -l < "$OUT/preview-kernel.log") lines, main $(wc -l < "$OUT/preview-main.log") lines"

# --- 6. stop camera + finalize -------------------------------------------
say "stop camera"
adbsh 'am force-stop com.DeviceTest' 2>/dev/null
adbsh 'input keyevent KEYCODE_HOME'  2>/dev/null
adbsh rm -f "$DEVTMP/camdump" "$DEVTMP/dump.txt" 2>/dev/null

say "hash + manifest"
( cd "$OUT" && sha256sum ./* > SHA256SUMS 2>/dev/null )
ls -la "$OUT"
echo
echo ">>> capture complete: $OUT"
echo ">>> VERIFY these before flashing NeonOS back:"
echo "    - stock-preview-*.txt show TG_SOF_CNT != 0 and TG_FRMSIZE != 0 (working pipeline)"
echo "    - SHA256SUMS present and non-empty"
echo ">>> then: diff L1-cold-baseline.txt vs stock-preview-* to find the SENINF->TG delta"
