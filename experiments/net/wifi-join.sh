#!/bin/sh
# wifi-join — join a WPA2-PSK network and go online. Run AFTER wifi-up2.sh.
#   sh wifi-join.sh "SSID" "passphrase"     # first time (saves to SD for next boots)
#   sh wifi-join.sh                          # reconnect using saved config
LAB=/mnt/sd/linux-lab
CLI="$LAB/wpa_cli -p /tmp/wpa -i wlan0"

if [ -n "$1" ]; then
  id=$($CLI add_network) || exit 1
  $CLI set_network "$id" ssid "\"$1\"" >/dev/null
  $CLI set_network "$id" psk "\"$2\"" >/dev/null
  $CLI enable_network "$id" >/dev/null
  $CLI save_config >/dev/null
  cp /tmp/wpa.conf "$LAB/wpa.conf" 2>/dev/null   # persist creds on SD
  echo "network $id ($1) added + saved"
fi

echo -n "associating"
i=0
while [ $i -lt 30 ]; do
  $CLI status 2>/dev/null | grep -q wpa_state=COMPLETED && break
  echo -n "."; sleep 1; i=$((i+1))
done
echo
$CLI status | grep -E "wpa_state|ssid|bssid|freq"
$CLI status | grep -q wpa_state=COMPLETED || { echo "*** did not associate"; exit 1; }

# request our previous lease so the IP stays stable across boots (the server
# usually honours -r). udhcpc.script saves each obtained IP to $LAB/.last_ip.
LAST=$(cat "$LAB/.last_ip" 2>/dev/null)
ROPT=""; [ -n "$LAST" ] && ROPT="-r $LAST"
udhcpc -i wlan0 $ROPT -n -q -s "$LAB/udhcpc.script" || {
  # a stale reservation can be refused; retry once without the hint
  [ -n "$ROPT" ] && udhcpc -i wlan0 -n -q -s "$LAB/udhcpc.script"
} || { echo "*** DHCP failed"; exit 1; }

# wireless cockpit: SSH over the LAN, no USB needed (dropbear, pubkey auth).
# ssh-up.sh mounts devpts (needed for ptys), installs authorized_keys, starts dropbear.
if [ -x "$LAB/dropbearmulti" ]; then
  sh "$LAB/ssh-up.sh"
else
  # fallback: insecure telnet bootstrap if dropbear isn't staged yet
  [ -d /dev/pts ] || mkdir -p /dev/pts
  mount | grep -q " /dev/pts " || mount -t devpts devpts /dev/pts 2>/dev/null
  pgrep telnetd >/dev/null 2>&1 || telnetd -l /bin/sh -p 23
  echo "(dropbear not staged — started telnet fallback on :23)"
fi
echo "==== ONLINE ===="
ifconfig wlan0 | grep "inet addr"
echo "telnet to the address above to work wirelessly."
