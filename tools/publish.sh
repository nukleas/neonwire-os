#!/usr/bin/env bash
# publish.sh — build all on-device artifacts and publish them to a GitHub release
# that the tablet can pull with neon-sync.sh. Run on the host (or a cloud agent).
#
# Produces a flat dist/ of everything the device runs, plus manifest.txt
# (path<TAB>sha256<TAB>mode), and uploads it as release "neon-latest".
#
#   tools/publish.sh
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT=$(pwd)
DIST="$ROOT/dist"
REL="neon-latest"

rm -rf "$DIST"; mkdir -p "$DIST"

echo "==> building framebuffer UI (neofb, neui)"
( cd experiments/fbui && ./build.sh >/dev/null )
cp experiments/fbui/neofb experiments/fbui/neui "$DIST/"

echo "==> building WMT launcher (wmtctl2)"
CC=$HOME/toolchains/armv7l-linux-musleabihf-cross/bin/armv7l-linux-musleabihf-gcc
STRIP=$HOME/toolchains/armv7l-linux-musleabihf-cross/bin/armv7l-linux-musleabihf-strip
( cd experiments/net && "$CC" -nostdlib -static -no-pie -Os -o wmtctl2 wmtctl2.c && "$STRIP" wmtctl2 )
cp experiments/net/wmtctl2 "$DIST/"

# prebuilt binaries that rarely change (checked in as release assets, not git):
# wpas, wpa_cli, dropbearmulti — copy from the local build tree if present.
for b in wpas wpa_cli dropbearmulti; do
  if [ -f "experiments/net/$b" ]; then cp "experiments/net/$b" "$DIST/"
  else echo "   (skip $b — not in experiments/net; add it to ship it)"; fi
done

echo "==> staging scripts + firmware"
cp experiments/net/wifi-up2.sh experiments/net/wifi-join.sh experiments/net/ssh-up.sh \
   experiments/net/udhcpc.script "$DIST/"
# consys firmware blobs the driver needs (small, from reference)
mkdir -p "$DIST/firmware"
cp reference/firmware/consys/* "$DIST/firmware/" 2>/dev/null || \
  echo "   (firmware not in reference/firmware/consys — device already has it staged)"

echo "==> manifest"
( cd "$DIST" && find . -type f ! -name manifest.txt -printf '%P\n' | sort | while read -r f; do
    printf '%s\t%s\t%s\n' "$f" "$(sha256sum "$f" | cut -d' ' -f1)" "$(stat -c%a "$f")"
  done > manifest.txt )
echo "   $(wc -l < "$DIST/manifest.txt") files"

echo "==> publishing release $REL"
if gh release view "$REL" >/dev/null 2>&1; then
  gh release upload "$REL" "$DIST"/* --clobber
else
  gh release create "$REL" "$DIST"/* --title "NEONWIRE rolling build" \
    --notes "Rolling artifacts pulled by the tablet's neon-sync.sh. Auto-updated by publish.sh."
fi
echo "==> done. On device: sh /mnt/sd/linux-lab/neon-sync.sh"
