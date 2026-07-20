# Devlog — reachable from anywhere (Tailscale)

**Date:** 2026-07-19
**One line:** Put the tablet on a Tailscale tailnet so it's `ssh`-able from anywhere by
a stable name, autostarting on every boot — and in doing so, accidentally solved the
LAN IP-drift problem for good.

Continues the story from [24-devlog-wifi-ssh.md](24-devlog-wifi-ssh.md). By this point
the tablet cold-boots onto Wi-Fi with SSH, cable-free. The remaining wish: reach it from
*outside* the LAN, and stop chasing its DHCP address around.

## The gating question: does the kernel even have TUN?

Tailscale needs a TUN device to be a real tailnet node (reachable at its `100.x` IP), not
just an outbound proxy. This is a stock **3.18 kernel with `CONFIG_MODULES=n`** — if TUN
isn't compiled in, we can't `modprobe` it, and we'd be looking at a kernel rebuild.

First thing checked, and the good news came fast:

```
/proc/misc:        200 tun
/dev/net/tun:      c 10 200
```

The TUN driver is **built in**. Biggest risk, cleared in one command. No rebuild.

## Getting it running

- Grabbed the static **`tailscale`/`tailscaled` v1.98.9 arm** build (fully static Go
  binaries — run fine on musl/busybox). 64 MB total, pushed over Wi-Fi with `scp -C` in ~75 s.
- State lives on the SD at `/mnt/sd/linux-lab/ts-state/`. (First bug: I named the state dir
  `tailscale`, which collided with the `tailscale` *CLI binary* sitting in the same folder.
  Renamed to `ts-state`.)
- Started `tailscaled` with `--tun=tailscale0`, ran `tailscale up`, visited the auth URL,
  approved the node. It came up as **`dl7006-neonos` @ `100.x.y.z`**, and `ssh` over the
  tailnet worked on the first try. WireGuard, DERP relay, direct endpoints — all negotiated.

The log had two complaints, both cosmetic: no `iptables`/`ip6tables` (busybox), so it fell
back to nftables ("nft-forced"), and a harmless SELinux log line (SELinux is permissive).
Neither blocks a plain client node.

## The red herring: "restart is broken"

Then I tried to validate boot-restart by killing `tailscaled` and starting it again — and it
kept failing to come back. Hours-of-your-life territory if you take it at face value. Two
things conspired to make it look worse than it was:

1. **Observation was lying.** Every SSH command that successfully started the daemon returned
   *no output* and appeared to hang — because `tailscaled` reconfigures routing as it comes up,
   which disturbs the very LAN SSH session I was watching through. And `pgrep -x tailscaled`
   kept reporting **0** even while I was connected *through* tailscaled — because the process
   shows as its full path (`/mnt/sd/linux-lab/tailscaled`), so the anchored match missed it.
   (Same class of bug as the `neui` process-count contamination from the last chapter.)
2. **The real mechanism.** Because there's no `iptables`, tailscaled manages nftables directly.
   A `kill -9` gives it no chance to tear those rules down, so the orphaned rules trip up the
   *next* daemon that tries to start in place. Restart-in-place fails; a **clean boot** — fresh
   `/tmp`, no orphaned rules — does not.

So instead of fighting kill/restart cycles, I did the honest test: **reboot the device and see
if it comes up on the tailnet by itself.**

## The acid test

`tailscale-up.sh` (clears any stale socket, `setsid`-detaches the daemon, logs to a file) is
launched by `wifi-join.sh` right after DHCP. On a real reboot — done from the tablet's own
on-screen **REBOOT** tile, which was itself nice to confirm works — the whole chain ran
unattended:

```
uptime: up 2 min          # fresh boot, I didn't touch it
dropbear: UP
neui: 1 instance
tailscaled: pid 286  (auto-reconnected from saved state, no re-auth)
```

I SSH'd in over `100.x.y.z` on a boot I never touched. Done.

## The accidental win

That reboot also handed the tablet a *different* LAN IP than before (`.33 → .35` — the AP
router ignored our `udhcpc -r` lease request that cycle). Normally that's the exact annoyance
we'd been fighting. But it no longer matters: **the tailnet address is stable regardless of the
LAN IP.** `ssh root@dl7006-neonos` (or the `100.x` IP) always finds it. Tailscale quietly
retired the IP-drift problem as a side effect of solving the off-LAN one.

## Lessons

1. **Check the gating dependency first.** "Is TUN in the kernel?" was one command and decided
   whether this was an afternoon or a kernel-rebuild saga.
2. **When your observation tool shares a fate with the thing you're observing, it will lie.**
   Watching a network daemon start *through the network it's reconfiguring* is a trap; verify
   from an independent path (or just reboot and check reachability).
3. **`kill -9` skips cleanup.** Anything that installs firewall/routing state (tailscaled,
   here) can leave orphans that only a clean boot clears. Don't generalize "restart-in-place
   fails" to "it's broken."
4. **Solve the right layer.** We spent effort pinning the DHCP lease; the durable fix for
   "reach it by a stable name" was an overlay network, which made the lease question moot.

## State at the end of this chapter

The tablet cold-boots — no cable — into its neon UI, joins Wi-Fi, brings up SSH, and appears
on the tailnet as `dl7006-neonos`, reachable from anywhere. Combined with OTA for both the
userland (`neon-sync.sh`) and the kernel (`neon-selfflash.sh`), it's a genuinely self-sufficient
little machine. Next up: a UI and functionality pass on the NEONWIRE face itself.
