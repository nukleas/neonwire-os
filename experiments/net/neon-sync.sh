#!/bin/sh
# neon-sync — the tablet updates ITSELF: pull the latest artifacts published by
# tools/publish.sh (GitHub release "neon-latest") over Wi-Fi and self-apply.
# No git needed on-device — just wget. This is how the OS improves without an
# ssh session: a cloud agent commits + publishes; the device pulls on demand
# (or on a timer / at boot).
#
# Needs a GitHub token for the PRIVATE repo, read from /mnt/sd/linux-lab/.gh_token
# (a fine-grained PAT, read-only Contents on nukleas/neonwire-os). Drop it there once.
#
#   sh /mnt/sd/linux-lab/neon-sync.sh          # pull changed artifacts, apply, restart UI
#   sh /mnt/sd/linux-lab/neon-sync.sh --check  # report what would change, don't apply
LAB=/mnt/sd/linux-lab
REPO=nukleas/neonwire-os
REL=neon-latest
TOK=$(cat $LAB/.gh_token 2>/dev/null)
API="https://api.github.com/repos/$REPO/releases/tags/$REL"
CHECK=0; [ "$1" = "--check" ] && CHECK=1
AUTH=""; [ -n "$TOK" ] && AUTH="--header=Authorization: token $TOK"

command -v wget >/dev/null || { echo "no wget"; exit 1; }
tmp=/tmp/neon-sync; mkdir -p $tmp

echo "==> fetching release metadata"
# GitHub API returns asset list w/ per-asset API url; private assets need the
# octet-stream Accept header + token to download.
wget -q -O $tmp/rel.json "$AUTH" --header="Accept: application/vnd.github+json" "$API" || {
  echo "release query failed (token? network?)"; exit 1; }

# pull manifest.txt first (asset), then compare sha256 against local copies
get_asset() { # $1 = asset name -> $2 = dest
  url=$(sed -n 's/.*"name": *"'"$1"'".*/&/p' $tmp/rel.json >/dev/null 2>&1; \
        awk -v n="\"$1\"" '$0 ~ n {f=1} f && /"url":.*assets/ {print; exit}' $tmp/rel.json \
        | sed 's/.*"url": *"\([^"]*\)".*/\1/')
  [ -n "$url" ] || return 1
  wget -q -O "$2" "$AUTH" --header="Accept: application/octet-stream" "$url"
}

get_asset manifest.txt $tmp/manifest.txt || { echo "no manifest asset"; exit 1; }

changed=0
while IFS="$(printf '\t')" read -r path sha mode; do
  [ -n "$path" ] || continue
  dst="$LAB/$path"
  cur=$(sha256sum "$dst" 2>/dev/null | cut -d' ' -f1)
  if [ "$cur" = "$sha" ]; then continue; fi
  changed=$((changed+1))
  echo "   update: $path"
  [ "$CHECK" = 1 ] && continue
  mkdir -p "$(dirname "$dst")"
  if get_asset "$(basename "$path")" "$dst.new"; then
    got=$(sha256sum "$dst.new" | cut -d' ' -f1)
    if [ "$got" = "$sha" ]; then mv "$dst.new" "$dst"; chmod "$mode" "$dst"
    else echo "   *** sha mismatch on $path, kept old"; rm -f "$dst.new"; fi
  else echo "   *** download failed: $path"; rm -f "$dst.new"; fi
done < $tmp/manifest.txt

echo "==> $changed changed"
[ "$CHECK" = 1 ] && exit 0
[ "$changed" = 0 ] && { echo "already up to date"; exit 0; }

# firmware may have refreshed -> restage where the kernel reads it
[ -d "$LAB/firmware" ] && cp "$LAB"/firmware/* /etc/firmware/ 2>/dev/null

# restart the UI so a new neui/neofb takes effect (single instance)
if pgrep -x neui >/dev/null 2>&1; then
  echo "==> restarting neui"
  killall neui 2>/dev/null; sleep 1
  # the /init respawn loop relaunches /bin/neui (bind-mounted to SD copy)
fi
echo "==> sync complete"
