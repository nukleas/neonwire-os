#!/usr/bin/env bash
# Regenerate SHA256SUMS for the active dump session.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SESSION="${1:-$ROOT/reference/dumps/session-20260718}"
cd "$SESSION"
{
  echo "# $(basename "$SESSION") — sha256"
  echo "# generated $(date -u +%Y-%m-%dT%H:%MZ)"
  sha256sum \
    raw/flash-user.bin \
    raw/boot1.bin \
    raw/boot2.bin \
    raw/prefix_0x0_9MiB.bin \
    raw/mbr-slots/0.bin \
    raw/mbr-slots/1.bin \
    raw/mbr-slots/2.bin \
    raw/mbr-slots/3.bin \
    raw/mbr-slots/4.bin \
    images/boot.img \
    images/recovery.img \
    meta/build.prop \
    meta/printgpt-session.txt
} | tee SHA256SUMS
echo "Wrote $SESSION/SHA256SUMS"
