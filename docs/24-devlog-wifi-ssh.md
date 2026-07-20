# Devlog — the day the tablet cut the cord (Wi-Fi + SSH)

**Date:** 2026-07-19
**One line:** Got CONSYS Wi-Fi working, then dropbear SSH over it, and retired the
USB serial cable — after discovering that the "impossible" Wi-Fi wall had been four
self-inflicted misdiagnoses stacked on top of each other.

This is a notes-for-the-showcase entry: the story and the lessons, not just the fix.

## Where we started

For weeks the integrated Wi-Fi/BT (a MediaTek CONSYS block on the MT8127 die) was
*the* wall of this project. We had a working custom Linux ("L1": stock 3.18 kernel +
busybox initramfs), a neon framebuffer HUD, touch — but no networking, so every deploy
was a slow base64 crawl over a USB serial ACM. We had written an exhaustive, rigorous,
and completely wrong diagnosis concluding the connsys silicon was simply dead.

## The four false walls (in order)

Each of these felt like bedrock at the time. Each was wrong.

1. **"The kernel can't load a driver" (CONFIG_MODULES=n).** True, but irrelevant — the
   driver was built-in all along.
2. **"It's a power/clock problem" (chipId reads 0).** We instrumented the kernel, proved
   the rails, the MTCMOS domain, the bus clock, the TOPAXI bus firewall were all perfect,
   and the chip *still* read `chipId = 0`. We concluded the fault was inside the silicon.
   **The falsification:** we finally booted *stock Android* (where Wi-Fi works) and read its
   kernel log — and `Read CONSYS chipId(0x00000000)` prints there too. That AP-side register
   is never how the chip is identified. We had spent days measuring a register that reads 0
   even when Wi-Fi works fine.
3. **"It's starved of firmware."** Closer. The driver loads a patch blob from hardcoded
   paths (`/etc/firmware/...`); we'd staged it to `/tmp/fw`, which the kernel never reads.
   Fixing the path got us further — the chip identified over the STP link — but then:
4. **"The SDIO transport won't enumerate."** `hif_sdio_stp_on: no supported func probed`.
   We concluded the connsys SDIO card never appears on the bus. **Wrong again, and this one
   was purely our bug:** the MT8127's consys is *on-die* and talks over **BTIF**, not SDIO.
   We had passed STP mode `0x24` (SDIO, for *external* combo chips). The correct value is
   `0x23` (BTIF). One nibble.

## The actual fix (two real bugs, both ours)

Reading the `conn_soc` driver source and disassembling the stock `wmt_launcher` gave the
two genuine issues:

- **Transport:** STP mode must be `0x23` (BTIF), not `0x24` (SDIO). `wmt_ctrl_stp_conf`
  literally calls `mtk_wcn_stp_open_btif()`.
- **The kernel asks userspace for the patch.** The patch-download path posts `srh_patch`
  *up* to a userspace launcher through `/dev/stpwmt` and waits 2 seconds. The launcher must
  answer with `SET_PATCH_NUM`/`SET_PATCH_INFO` ioctls (patch count + load addresses parsed
  from 4 bytes at offset 24 of each `ROMv2_patch_*_hdr.bin`) and write `"ok"` back. Stock
  Android's launcher stalls on L1 (it blocks on Android's property service), so it never did
  this. We wrote a ~1.3 KB freestanding launcher, `wmtctl2.c`, that does the whole dance.

Result, matching Android's boot trace line for line:
`FUNC_ON(WMT)=0` → `patch dwn frag(45,720) ok` + `frag(82,64) ok` → `FUNC_ON(WIFI)=0`
→ `wmt call wlan probe ok` → **`wlan0`**.

## Then: actually getting online

- **wpa_supplicant.** Android's own binary SIGABRTs on L1 (welded to keystore/binder). Built
  a static musl `wpa_supplicant 2.11`. First try used the WEXT driver — it *associated but
  never completed the 4-way handshake* on this chip. Rebuilt with **nl80211 + libnl-tiny**
  (needs `-D_GNU_SOURCE` or musl hides `struct ucred`). That completed the handshake → DHCP
  lease → **192.168.4.32**.
- **SSH.** Built static `dropbear 2022.83`. Pubkey auth, persistent host key on the SD.
  `ssh root@192.168.4.32` is now passwordless from the dev host; `scp` works too.
- **The serial cable is retired.** Fittingly, the USB ACM wedged mid-session — and it no
  longer mattered. All remaining work went over Wi-Fi.

## Two red herrings worth remembering

While building the Wi-Fi UI into the HUD, two things *looked* like problems and weren't:

- **"The UI keeps flashing between states."** Two `neui` processes were both painting the
  framebuffer. Because this is a command-mode MIPI panel, each one pan-flips the hardware
  buffers every frame to force a refresh, so two instances ping-pong the screen between their
  outputs. One was the init respawn loop, one was a test copy. Root cause: *me*, not the code.
- **"How is the CPU so heavy for a simple UI?"** It wasn't. Load average pinned at ~8 with
  `neui` killed and every process at 0.0% CPU. The cause is ~8 stock MediaTek kernel threads
  (battery poller, CPU-hotplug governor, hang detector, display-capture overlay, ...) stuck in
  **D-state** — uninterruptible hardware waits under our non-Android userland. Linux's load
  average counts D-state tasks, so the number is inflated while the CPU is actually idle.

## Lessons (the reusable ones)

1. **Instrument the thing that _works_ before concluding the thing that doesn't is impossible.**
   One `adb logcat -b kernel` on stock Android falsified a week of "silicon wall" analysis in
   a single line.
2. **A rigorous wrong answer is still wrong.** We had measurements, diagrams, and a clean
   story — all built on never checking the premise (that `chipId=0` meant anything).
3. **On embedded, most "hardware walls" are protocol/userspace bugs.** All four walls here were
   config, path, or a single wrong constant.
4. **`load average` ≠ CPU usage on Linux.** D-state inflates it.

## What this unlocks

The tablet is now a normal networked Linux box you `ssh` into. Deploy is: cross-compile on
the host → `scp` (or `tools/net-push.py`) over Wi-Fi → run. Next: a self-update path so the
device pulls new builds itself (`tools/publish.sh` + `experiments/net/neon-sync.sh`), and
eventually Bluetooth (same STP/patch recipe, function 1).

*A fun retrofit of old, otherwise mostly useless hardware — a $30 white-label Android tablet
now runs a hand-built Linux with its own neon OS face, on-screen Wi-Fi manager, and SSH.*
