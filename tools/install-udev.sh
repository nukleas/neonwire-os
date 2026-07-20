#!/usr/bin/env bash
# Install MediaTek udev rules for mtkclient / Preloader access.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")" && pwd)"
RULES_DIR="$ROOT/mtkclient/Setup/Linux"

if [[ $EUID -ne 0 ]]; then
  echo "Re-running with sudo..."
  exec sudo bash "$0" "$@"
fi

cp -v "$RULES_DIR/52-mtk.rules" /etc/udev/rules.d/
cp -v "$RULES_DIR/51-edl.rules" /etc/udev/rules.d/
# Optional: broad Android USB ACLs
# cp -v "$RULES_DIR/50-android.rules" /etc/udev/rules.d/

udevadm control --reload-rules
udevadm trigger
echo "udev rules installed. Unplug/replug the tablet."
