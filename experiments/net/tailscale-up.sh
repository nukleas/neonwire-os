#!/bin/sh
# tailscale-up — start tailscaled for off-LAN access (SSH from anywhere via the tailnet).
# State persists on the SD; the TUN driver is built into this 3.18 kernel (/proc/misc: tun).
#
#   sh tailscale-up.sh            # start the daemon (idempotent)
#   then, first time only:
#   /mnt/sd/linux-lab/tailscale --socket=/tmp/tailscaled.sock up --hostname=dl7006-neonos \
#       --accept-dns=false        # prints an auth URL to visit once
#
# After auth, the node is reachable at its 100.x tailnet IP from any of your devices.
LAB=/mnt/sd/linux-lab
STATE=$LAB/ts-state          # NB: not "tailscale" — that's the CLI binary's name
SOCK=/tmp/tailscaled.sock
LOG=/tmp/tailscaled.log

mkdir -p $STATE
[ -e /dev/net/tun ] || { mkdir -p /dev/net; mknod /dev/net/tun c 10 200; }

if pgrep -f "[t]ailscaled" >/dev/null 2>&1; then
  echo "tailscaled already running (pid $(pgrep -f '[t]ailscaled' | head -1))"
else
  rm -f $SOCK   # a leftover socket file blocks the new daemon from binding
  # fully detached + logged (redirect INSIDE the setsid shell so it survives ssh)
  setsid sh -c "$LAB/tailscaled \
      --state=$STATE/tailscaled.state \
      --socket=$SOCK \
      --statedir=$STATE \
      --tun=tailscale0 >$LOG 2>&1" </dev/null >/dev/null 2>&1 &
  # wait for the socket to appear
  n=0
  while [ $n -lt 15 ]; do [ -S $SOCK ] && break; sleep 1; n=$((n+1)); done
fi

echo "== tailscaled log =="
tail -12 $LOG 2>/dev/null
echo "== backend status =="
$LAB/tailscale --socket=$SOCK status 2>&1 | head -6
