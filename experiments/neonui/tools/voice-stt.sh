#!/bin/sh
# voice-stt.sh — mic → xAI STT (raw WAV upload to Grok speech).
#
# Usage:
#   voice-stt.sh [seconds] [mode]
#     seconds  default 5 (wake defaults to 2)
#     mode     command (default) | wake
#
# IMPORTANT: keyterm values with spaces MUST be passed as single -F args
# ("keyterm=play a song"). Unquoted -F keyterm=play a song makes curl treat
# "a" / "song" as extra hosts → "Could not resolve host: song".

set -e
SECS="${1:-}"
MODE="${2:-command}"

case "$MODE" in
  wake|command) ;;
  *) MODE=command ;;
esac

if [ -z "$SECS" ]; then
  if [ "$MODE" = "wake" ]; then SECS=2; else SECS=6; fi
fi
case "$SECS" in
  ''|*[!0-9]*) if [ "$MODE" = "wake" ]; then SECS=2; else SECS=6; fi ;;
esac
if [ "$SECS" -lt 1 ]; then SECS=1; fi
if [ "$SECS" -gt 20 ]; then SECS=20; fi

LAB=/mnt/sd/linux-lab
ALP=/mnt/data/alpine
KEYF=$LAB/xai.key
WAV_HOST=/tmp/voice-ask.wav
WAV_ALP=/tmp/voice-ask.wav
[ "$MODE" = "wake" ] && WAV_HOST=/tmp/voice-wake.wav && WAV_ALP=/tmp/voice-wake.wav

if [ ! -f "$KEYF" ]; then
  echo "voice-stt: missing $KEYF" >&2
  exit 2
fi
KEY=$(cat "$KEYF")
[ -n "$KEY" ] || { echo "voice-stt: empty key" >&2; exit 2; }

if [ ! -f "$ALP/etc/resolv.conf" ] || ! grep -q nameserver "$ALP/etc/resolv.conf" 2>/dev/null; then
  [ -f /etc/resolv.conf ] && cp /etc/resolv.conf "$ALP/etc/resolv.conf" || echo "nameserver 8.8.8.8" > "$ALP/etc/resolv.conf"
fi
# Prefer working DNS (8.8.8.8) if resolv is USB-gadget only
if ! grep -qE '8\.8\.8\.8|1\.1\.1\.1|68\.105' "$ALP/etc/resolv.conf" 2>/dev/null; then
  echo "nameserver 8.8.8.8" >> "$ALP/etc/resolv.conf"
fi
for d in proc sys dev; do
  mountpoint -q "$ALP/$d" 2>/dev/null || mount --bind "/$d" "$ALP/$d" 2>/dev/null || true
done

if [ -x "$LAB/mic-up.sh" ]; then
  sh "$LAB/mic-up.sh" >/tmp/mic-up.log 2>&1 || true
fi

echo "voice-stt[$MODE]: record ${SECS}s..." >&2
rm -f "$ALP$WAV_ALP" "$WAV_HOST"
if ! chroot "$ALP" /usr/bin/arecord -D hw:0,1 -f S16_LE -c1 -r16000 -d "$SECS" "$WAV_ALP" >/tmp/arecord.log 2>&1; then
  echo "voice-stt: arecord failed" >&2
  cat /tmp/arecord.log >&2
  exit 3
fi
cp "$ALP$WAV_ALP" "$WAV_HOST" 2>/dev/null || true

eval "$(chroot "$ALP" /usr/bin/python3 -c "
import struct
d=open('$WAV_ALP','rb').read()
off=44 if d[:4]==b'RIFF' else 0
s=struct.unpack('<%dh'%((len(d)-off)//2), d[off:off+((len(d)-off)//2)*2])
peak=max(abs(x) for x in s) if s else 0
rms=(sum(x*x for x in s)/len(s))**0.5 if s else 0
print(f'PEAK={peak};RMS={int(rms)}')
" 2>/dev/null || echo 'PEAK=0;RMS=0')"

echo "voice-stt[$MODE]: peak=$PEAK rms=$RMS" >&2
MINP=1200
[ "$MODE" = "command" ] && MINP=800
if [ "$PEAK" -lt "$MINP" ] 2>/dev/null; then
  echo "voice-stt[$MODE]: energy too low — skip STT" >&2
  echo ""
  exit 0
fi

cp "$WAV_HOST" "$ALP$WAV_ALP"

# Quote every keyterm — spaces must stay inside the -F value.
if [ "$MODE" = "wake" ]; then
  RESP=$(chroot "$ALP" /usr/bin/curl -sS --max-time 45 \
    -X POST "https://api.x.ai/v1/stt" \
    -H "Authorization: Bearer ${KEY}" \
    -F "language=en" \
    -F "format=true" \
    -F "keyterm=hey hax" \
    -F "keyterm=ok hax" \
    -F "keyterm=okay hax" \
    -F "keyterm=hey tablet" \
    -F "keyterm=hi hax" \
    -F "keyterm=hax" \
    -F "keyterm=listen" \
    -F "keyterm=computer" \
    -F "file=@${WAV_ALP};type=audio/wav" 2>/tmp/stt.err) || {
    echo "voice-stt: curl failed" >&2
    cat /tmp/stt.err >&2
    exit 4
  }
else
  RESP=$(chroot "$ALP" /usr/bin/curl -sS --max-time 60 \
    -X POST "https://api.x.ai/v1/stt" \
    -H "Authorization: Bearer ${KEY}" \
    -F "language=en" \
    -F "format=true" \
    -F "keyterm=play a song" \
    -F "keyterm=play music" \
    -F "keyterm=stop music" \
    -F "keyterm=stop song" \
    -F "keyterm=turn on" \
    -F "keyterm=turn off" \
    -F "keyterm=sprinkler" \
    -F "keyterm=garden" \
    -F "keyterm=water" \
    -F "keyterm=lights" \
    -F "keyterm=camera" \
    -F "keyterm=network" \
    -F "keyterm=home assistant" \
    -F "keyterm=open camera" \
    -F "keyterm=go home" \
    -F "keyterm=status" \
    -F "keyterm=volume" \
    -F "keyterm=hey hax" \
    -F "keyterm=hax" \
    -F "file=@${WAV_ALP};type=audio/wav" 2>/tmp/stt.err) || {
    echo "voice-stt: curl failed" >&2
    cat /tmp/stt.err >&2
    exit 4
  }
fi

[ "$MODE" = "command" ] && cp "$WAV_HOST" /tmp/voice-last-command.wav 2>/dev/null || true

TEXT=$(chroot "$ALP" /usr/bin/python3 -c "
import json,sys
try:
  d=json.loads(sys.argv[1])
  print((d.get('text') or '').strip())
except Exception:
  pass
" "$RESP" 2>/dev/null || true)

if [ -z "$TEXT" ]; then
  TEXT=$(printf '%s' "$RESP" | sed -n 's/.*"text"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -1)
  TEXT=$(printf '%s' "$TEXT" | sed 's/\\n/ /g; s/\\t/ /g; s/\\"/"/g')
fi

if [ -z "$TEXT" ]; then
  echo "voice-stt[$MODE]: empty transcript" >&2
  echo ""
  exit 0
fi
echo "voice-stt[$MODE]: \"$TEXT\"" >&2
printf '%s\n' "$TEXT"
