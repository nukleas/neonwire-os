#!/usr/bin/env bash
# Source from the repo root:  source tools/mtk-env.sh
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$ROOT/.." && pwd)"
# shellcheck disable=SC1091
source "$ROOT/venv/bin/activate"
export MTKCLIENT_DIR="$ROOT/mtkclient"
export ANDROIDS_DIR="$REPO"

mtk() {
  (cd "$MTKCLIENT_DIR" && python mtk.py "$@")
}

mtk-wait-preloader() {
  echo "Watching for MediaTek Preloader (0e8d:2000)..."
  echo "Power tablet OFF, then plug USB (optionally hold Vol Up)."
  journalctl -kf | rg --line-buffered -i '0e8d|preloader|mediatek|android'
}

echo "mtkclient env ready."
echo "  mtk <args>              # e.g. mtk printgpt"
echo "  mtk-wait-preloader      # dmesg watch helper"
echo "  cd \$MTKCLIENT_DIR"
