#!/usr/bin/env bash
# Cross-compile the fbui binaries (static ARMv7) and gzip them for serial push.
set -euo pipefail
cd "$(dirname "$0")"

CC=${CC:-$HOME/toolchains/armv7l-linux-musleabihf-cross/bin/armv7l-linux-musleabihf-gcc}
STRIP=${STRIP:-$HOME/toolchains/armv7l-linux-musleabihf-cross/bin/armv7l-linux-musleabihf-strip}
CFLAGS="-Os -static -no-pie -fno-pie -D_GNU_SOURCE -ffunction-sections -fdata-sections -Wl,--gc-sections -Wall"

python3 genfont.py

for bin in neofb neui; do
  "$CC" $CFLAGS -o "$bin" "$bin.c"
  "$STRIP" "$bin"
  gzip -9 -f -k "$bin"
  printf '%-6s %7d bytes (gz %6d)  sha256 %s\n' \
    "$bin" "$(stat -c%s "$bin")" "$(stat -c%s "$bin.gz")" "$(sha256sum "$bin" | cut -d' ' -f1)"
done
