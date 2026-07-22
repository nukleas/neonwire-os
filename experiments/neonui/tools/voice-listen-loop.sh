#!/bin/sh
# voice-listen-loop.sh — continuous wake-word watcher for neonwire ASSIST.
#
# While /tmp/voice-listen.on exists:
#   1) short STT scan (wake keyterms + energy gate)
#   2) if text matches wake words → write /tmp/voice-wake.hit and exit 0
#      (neonwire then records the command and talks to the agent)
#
# Started/stopped by the ASSIST "LISTEN" chip. Cheap when quiet (no STT if peak low).

LAB=/mnt/sd/linux-lab
FLAG=/tmp/voice-listen.on
HIT=/tmp/voice-wake.hit
STT=$LAB/voice-stt.sh

rm -f "$HIT"
echo "voice-listen: armed (say: hey hax)" >&2

is_wake() {
  # lowercase match on common wake phrases
  t=$(printf '%s' "$1" | tr 'A-Z' 'a-z')
  case "$t" in
    *hey\ hax*|*ok\ hax*|*okay\ hax*|*hi\ hax*|*hey\ tablet*|*hey\ tax*|*a\ hax*)
      return 0 ;;
    *hax*|*tablet*)
      # bare "hax" / "tablet" only if short (avoid mid-sentence false positives)
      words=$(printf '%s' "$t" | wc -w)
      [ "$words" -le 3 ] && return 0
      return 1 ;;
    *listen*|*computer*)
      words=$(printf '%s' "$t" | wc -w)
      [ "$words" -le 2 ] && return 0
      return 1 ;;
    *) return 1 ;;
  esac
}

while [ -f "$FLAG" ]; do
  TEXT=$("$STT" 2 wake 2>/tmp/voice-wake-scan.log) || TEXT=""
  TEXT=$(printf '%s' "$TEXT" | tr -d '\r' | head -1)
  if [ -n "$TEXT" ] && is_wake "$TEXT"; then
    echo "voice-listen: WAKE \"$TEXT\"" >&2
    printf '%s\n' "$TEXT" > "$HIT"
    # leave FLAG for UI to clear; exit so only one wake fires
    exit 0
  fi
  # brief pause between scans (arecord already took ~2s)
  sleep 0.3 2>/dev/null || usleep 300000 2>/dev/null || sleep 1
done
echo "voice-listen: disarmed" >&2
exit 0
