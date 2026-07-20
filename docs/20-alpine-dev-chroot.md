# Alpine dev chroot + RNDIS USB-net — a real Linux userland on the DL7006

**Date:** 2026-07-19
The tablet is an **offline armv7 computer** (975 MB RAM, 3.18 kernel, no on-chip
wireless). This doc turns it into a genuine dev box: a package-managed **Alpine
Linux** userland (python3, gcc, git, vim, tmux, sqlite, lua, pip) running via
`chroot`, transferred over a fast **RNDIS USB-net** link.

## Why Alpine 3.12 specifically
Alpine **3.13+ uses musl 1.2** (64-bit `time_t` → leans on kernel-5.1 `*_time64`
syscalls) — unsafe on our 3.18 kernel. **Alpine 3.12 uses musl 1.1.24** (32-bit
time, legacy syscalls) → runs cleanly on 3.18. Confirmed on-device: python3 3.8.10,
gcc 9.3.0 compiles+runs native ARM code, git 2.26.3, lua5.3, sqlite3 all work.

## Build the rootfs (host, one-time, needs docker + qemu binfmt)
```bash
# register ARM emulation once:
docker run --privileged --rm tonistiigi/binfmt --install arm
# build + populate:
docker run --platform linux/arm/v7 --name alpine-build arm32v7/alpine:3.12 sh -c '
  apk add --no-cache python3 py3-pip gcc musl-dev make git vim nano tmux htop less \
    file bash coreutils findutils grep sed gawk sqlite lua5.3 curl wget linux-headers pkgconf ncurses'
docker export alpine-build | gzip -1 > experiments/alpine/rootfs-armv7.tar.gz
docker rm alpine-build
```
Result: ~200 MB rootfs, ~81 MB gzipped. Add more with `apk add` in the container.

## RNDIS USB-net link (the fast transfer channel — reusable!)
The stock kernel has RNDIS built in. `experiments/net/net-up.sh` reconfigures the
USB gadget to **`rndis,acm`** (keeps the serial shell) and runs udhcpd. Key facts
learned:
- Run it **detached** from the serial shell: `setsid sh net-up.sh </dev/null &`
  (the gadget resets mid-run and would kill a foreground shell).
- After re-enum: host gets a USB net iface (e.g. `enp0s20f0u7u1`) auto-DHCP'd to
  **192.168.42.10**; tablet = **192.168.42.1**. Ping ~0.5 ms.
- **The serial shell survives but moves to `/dev/ttyACM1`** (the composite shifts the
  ACM device number). Use `serial-cmd.py -p /dev/ttyACM1`.
- **Host firewall:** `ufw` blocks incoming on the new iface → `sudo ufw allow from
  192.168.42.0/24` so the tablet can reach the host's HTTP server.
- `telnetd` needs a pty; our init doesn't mount devpts, so telnet is dead — just use
  `ttyACM1`. (`alpine-enter.sh` mounts devpts inside the chroot so pty tools work there.)

Transfer (host serves, tablet pulls — 81 MB in seconds):
```bash
# host:
( cd experiments/alpine && python3 -m http.server 8000 --bind 192.168.42.10 & )
# tablet (over ttyACM1):
mount -t ext4 -o rw /dev/mmcblk0p8 /mnt/data
wget http://192.168.42.10:8000/rootfs-armv7.tar.gz -O /mnt/data/rootfs.tar.gz
wget http://192.168.42.10:8000/alpine-enter.sh     -O /mnt/data/alpine-enter.sh
```

## Install + enter (device)
The rootfs must live on **ext4** (the SD is FAT32 → no symlinks/perms). Use `/data`
(`mmcblk0p8`, ~4.5 GB free), persistent across reboots.
```sh
cd /mnt/data && mkdir -p alpine && gunzip -c rootfs.tar.gz | tar -x -C alpine
sh /mnt/data/alpine-enter.sh            # interactive Alpine shell
sh /mnt/data/alpine-enter.sh -c 'CMD'   # one-shot
```
`alpine-enter.sh` binds `/proc`,`/sys`,`/dev`,`/dev/pts`, maps the SD to `/media/sd`,
sets resolv.conf, and `chroot`s into `/bin/bash -l`.

## Next: internet ON the tablet (when tethered)
The RNDIS link + host NAT would give the tablet (and the chroot) **real internet**
through the host — then `apk add`, `pip install`, `git clone` work on-device:
```bash
# host (needs sudo): NAT the tablet out the host's internet iface (e.g. enp5s0)
sudo sysctl -w net.ipv4.ip_forward=1
sudo iptables -t nat -A POSTROUTING -o enp5s0 -j MASQUERADE
sudo iptables -A FORWARD -i enp0s20f0u7u1 -o enp5s0 -j ACCEPT
sudo iptables -A FORWARD -i enp5s0 -o enp0s20f0u7u1 -m state --state RELATED,ESTABLISHED -j ACCEPT
# tablet: route + DNS
ip route add default via 192.168.42.10; echo nameserver 1.1.1.1 > /etc/resolv.conf
```
That flips it from "offline computer" to "fully online dev box whenever it's plugged in."

## Artifacts
`experiments/alpine/alpine-enter.sh`, `rootfs-armv7.tar.gz` (gitignored);
device: `/mnt/data/alpine/` (rootfs), `/mnt/data/alpine-enter.sh`.
