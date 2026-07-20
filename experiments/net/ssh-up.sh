#!/bin/sh
# ssh-up — start dropbear SSH on L1 (needs wifi up + dropbearmulti on SD).
# Persistent host key + our authorized_keys live on the SD so they survive reboots.
# Pubkey auth only (-s disables password login). Run after wifi-up2.sh + join.
LAB=/mnt/sd/linux-lab
DB=$LAB/dropbearmulti
KEY=$LAB/dropbear/ed25519_host

[ -x "$DB" ] || { echo "no dropbearmulti on SD"; exit 1; }
mkdir -p $LAB/dropbear /root/.ssh

# install our public key (staged next to this script) as root's authorized_keys
if [ -f $LAB/authorized_keys ]; then
  cp $LAB/authorized_keys /root/.ssh/authorized_keys
  chmod 700 /root/.ssh; chmod 600 /root/.ssh/authorized_keys
fi

# persistent host key (generate once)
[ -f "$KEY" ] || "$DB" dropbearkey -t ed25519 -f "$KEY"

# dropbear needs a pty; L1 has /dev/ptmx but no devpts mounted
[ -d /dev/pts ] || mkdir -p /dev/pts
mount | grep -q ' /dev/pts ' || mount -t devpts devpts /dev/pts 2>/dev/null

# client-side helpers on PATH (dropbearmulti is a busybox-style multi-call binary)
ln -sf "$DB" /bin/dropbear 2>/dev/null
ln -sf "$DB" /bin/dropbearkey 2>/dev/null
ln -sf "$DB" /bin/scp 2>/dev/null

# -s: no password logins (pubkey only)  -r: host key  -p 22
pgrep -f 'dropbearmulti dropbear' >/dev/null 2>&1 || \
  "$DB" dropbear -s -r "$KEY" -p 22 -P /tmp/dropbear.pid
echo "dropbear up on :22 (pubkey auth). ssh root@$(ifconfig wlan0 2>/dev/null | sed -n 's/.*inet addr:\([0-9.]*\).*/\1/p')"
