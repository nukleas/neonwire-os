#!/bin/sh
# net-up — add a USB-Ethernet (RNDIS) link alongside the ACM serial shell, so we
# can telnet in and move files at USB speed instead of base64-over-serial.
#
# Run from the DL7006 shell (over ACM). NOT run at boot on purpose: if the gadget
# reconfigure misbehaves, just power-cycle the tablet — the default ACM-only boot
# comes back with zero reflash. Nothing here is persistent.
#
#   Host side after this runs:
#     - a usb network iface appears (usb0/enpXsY). It gets 192.168.42.10 via DHCP,
#       or set it: ip addr add 192.168.42.100/24 dev <iface>; ip link set <iface> up
#     - telnet 192.168.42.1        (root shell)
#     - tftp / ftp to 192.168.42.1 for files
set +e
IP=192.168.42.1
MASK=255.255.255.0
AUSB=/sys/class/android_usb/android0

echo "[net-up] reconfiguring USB gadget: rndis + acm ..."
if [ -d "$AUSB" ]; then
  echo 0 > "$AUSB/enable" 2>/dev/null
  # a stable locally-administered MAC keeps the host iface name steady
  [ -e "$AUSB/f_rndis/ethaddr" ] && echo "02:d1:70:06:00:01" > "$AUSB/f_rndis/ethaddr" 2>/dev/null
  echo rndis,acm > "$AUSB/functions" 2>/dev/null
  echo 0 > "$AUSB/bDeviceClass" 2>/dev/null
  echo 1 > "$AUSB/enable" 2>/dev/null
else
  echo "[net-up] no android_usb node — is this the L1 kernel?" >&2
fi

# wait for the rndis/usb iface to appear
IFACE=
n=0
while [ "$n" -lt 15 ]; do
  for c in rndis0 usb0 eth0; do
    [ -e "/sys/class/net/$c" ] && IFACE="$c" && break
  done
  [ -n "$IFACE" ] && break
  sleep 1; n=$((n+1))
done
if [ -z "$IFACE" ]; then
  echo "[net-up] no rndis/usb iface came up. Recover: power-cycle the tablet." >&2
  echo "[net-up] (ACM serial shell may have dropped; re-plug or power-cycle.)" >&2
  exit 1
fi
echo "[net-up] iface=$IFACE  ->  $IP"
ifconfig "$IFACE" "$IP" netmask "$MASK" up

# minimal DHCP server so the host auto-configures
mkdir -p /etc /var/lib/misc
cat > /etc/udhcpd.conf <<EOF
start       192.168.42.10
end         192.168.42.20
interface   $IFACE
option subnet $MASK
option lease 86400
lease_file  /var/lib/misc/udhcpd.leases
pidfile     /var/run/udhcpd.pid
EOF
: > /var/lib/misc/udhcpd.leases
killall udhcpd 2>/dev/null
udhcpd -S /etc/udhcpd.conf 2>/dev/null && echo "[net-up] udhcpd serving 192.168.42.10-20"

# telnet shell (no auth — private point-to-point USB link only)
killall telnetd 2>/dev/null
telnetd -b "$IP:23" -l /bin/sh && echo "[net-up] telnetd on $IP:23 (telnet $IP)"

echo "[net-up] up. Host: expect DHCP, then  telnet $IP   /   tftp $IP"
echo "[net-up] tear down with: net-down"
