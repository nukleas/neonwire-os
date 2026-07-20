# NEONWIRE — cyberpunk framebuffer UI (fbui)

**Date:** 2026-07-19
**Workspace:** `experiments/fbui/`
**Depends on:** L1 live Linux ([14-live-linux-lab.md](14-live-linux-lab.md)), writable `/dev/fb0` (mtkfb)
**Aesthetic source:** `in-repo palette` design tokens (JetBrains Mono, `cd-*` palette, CRT scanline + glow motifs)

---

## What this is

The first piece of the DL7006's *custom OS face*: a neon system dashboard
painted directly onto the panel framebuffer. No X, no GL, no Android — a single
static ARM binary using raw framebuffer ioctls + `/proc`.

Rendered on device (`shot2.png`): glowing `NEONWIRE // DL-7006` wordmark,
`[ SYSTEM ]` + `[ SUBSYSTEMS ]` panels with corner brackets and title tabs,
live stats (kernel, cpu, uptime, load, mem bar), colour-coded subsystem status
dots (wifi **OFFLINE** in magenta — honest to reality), CRT scanlines, faint
cyan pixel-grid, bottom status bar.

## Canvas (confirmed live)

| Property | Value |
|----------|-------|
| Device | `/dev/fb0` (mtkfb), 32bpp |
| Visible | 1024×600 |
| Virtual | 1024×1824 (panned to yoffset 0 by neofb) |
| Stride | 4096 B/row |
| Channel order | BGRA — r@16 g@8 b@0 (read from `FBIOGET_VSCREENINFO`) |

## Palette (from cyberdesign tokens)

`bgBase #05060a` · `bgPanel #0b0f18` · `borderMid #26324c` · `cyan #47f6ff` ·
`cyanBright #bdffff` · `magenta #ff2bd6` · `greenBright #52ff9f` ·
`amber #ffaa00` · `text #e8e4e0` · scanline `#000 @18%` · pixel-grid `cyan @5.5%`.

## Two programs

| Binary | What it is |
|--------|-----------|
| **`neofb`** | one-shot / looping system dashboard (the boot-splash candidate) |
| **`neui`** | **touch launcher** — nav rail + 5 live panels, driven by the touchscreen |

### neui panels (all live kernel data)

| # | Panel | Source |
|---|-------|--------|
| 0 | SYSTEM | uname + `/proc` (host/kernel/cpu/uptime/load/mem bar) |
| 1 | PROCESS | walks `/proc/<pid>/stat`, sorts by RSS, shows state/RSS/comm |
| 2 | STORAGE | `/proc/mounts` + `statvfs` usage meters per fs |
| 3 | KERNLOG | kernel ring buffer via `klogctl(3)` (colourises fail/WMT lines) |
| 4 | NETWORK | `/proc/net/dev` ifaces; `wlan0 OFFLINE` (CONSYS, see doc 15) |

### Action bar — tool tiles + do-actions

A row of tappable buttons under the content panel. Two kinds (one `ACT[]` table):

**Tool tiles** (`cmd` set) — run a shell command via `popen` and render its stdout
in a scrollable neon **overlay** (modal, `[X]` / tap-outside to close, tap lower/upper
half to scroll, colour-codes fail/error lines red):

| Tile | Command |
|------|---------|
| DF | `df -h` |
| MEM | `free` + `/proc/meminfo` |
| MOUNTS | `mount` |
| DMESG | `dmesg \| tail -150` (scrolls) |

**Do-actions** (`fn` set) — immediate effect + toast; destructive ones confirm-guarded:

| Tile | Effect |
|------|--------|
| SYNC | `sync()` → toast |
| REBOOT | `reboot()` — **two taps** (arms → `CONFIRM?`, 5s) |

Add a tool: `{ "LABEL", accent, 0, 0, "shell cmd", "[ title ]" }` in `ACT[]`.
Add a do-action: `{ "LABEL", accent, confirm, fn, 0, 0 }`. Overlay state + `run_tool`
+ `draw_overlay`/`overlay_tap` in `neui.c`.

### Touch

`mtk-tpd` evdev at **`/dev/input/event4`** — a **type-A multitouch** device.
Confirmed by `--evdump`: it reports `BTN_TOUCH` (KEY 330) then `ABS_MT_POSITION_X/Y`
(codes 53/54) + `ABS_MT_TRACKING_ID` (57), **not** `ABS_X`/`ABS_Y`. `EVIOCGABS`
still returns ranges **X[0..1024] Y[0..600]** = 1:1 to the panel, no swap/flip.

**Tap detection (important):** arm on the `BTN_TOUCH=1` press edge, then fire on
the next `SYN_REPORT` so the fresh MT X/Y are in. Do NOT use a contact-latch keyed
on release — on this driver `BTN_TOUCH=0` and the empty frame can arrive in an
order that sticks the latch, so only the first tap ever fires. (That was the
original "taps don't work" bug.) The handler also reads codes 53/54 for position.

Debug/calibration: `--evdump` (dump every raw event — tap and watch),
`--probe` (ranges + which tile/action a tap hits), `--tap X Y` (headless tap
injection over serial), `--swap --flipx --flipy` (orientation, unused here).
The live UI logs each dispatched tap to stderr → `/tmp/neui.log`.

## Files

```text
experiments/fbui/
  genfont.py    bakes JetBrains Mono Bold -> font_neon.h (11x25 AA coverage, ASCII 32-126)
  fbgfx.h       shared engine: palette, glyph/glow, panels, bar, scanlines, fb lifecycle, /proc
  neofb.c       system dashboard (uses fbgfx.h)
  neui.c        touch launcher + 5 panels + evdev input (uses fbgfx.h)
  font_neon.h   generated (do not edit)
  build.sh      cross-compile both static ARMv7 (no-pie) + gzip
  push.py       stream gzip(binary) over serial -> SD, sha256 verify (--target for neui)
  pull_shot.py  render on device + pull framebuffer -> PNG (md5-verified; --args passthrough)
  neofb / neui  built binaries (gitignored)
```

## Toolchain

`~/toolchains/armv7l-linux-musleabihf-cross/bin/armv7l-linux-musleabihf-gcc` (GCC 11.2, musl, static).

## Iterate loop (NO reflash needed)

The binary lives on the **persistent SD card** and runs on demand — visuals can
be iterated entirely over serial:

```bash
cd experiments/fbui
./build.sh                                    # compile BOTH + gzip, prints sha256
python3 push.py neofb.gz                       # -> /mnt/sd/linux-lab/neofb
python3 push.py neui.gz --target /mnt/sd/linux-lab/neui
python3 pull_shot.py --out /tmp/shot.png       # screenshot neofb
python3 pull_shot.py --bin /mnt/sd/linux-lab/neui --args "--panel 1" --out /tmp/p1.png
```

On device directly:

```sh
/mnt/sd/linux-lab/neofb              # one-shot dashboard paint, exit
/mnt/sd/linux-lab/neofb --loop 1 &   # live HUD, redraw every 1s
/mnt/sd/linux-lab/neui >/tmp/neui.log 2>&1 &   # live touch launcher (tap tiles)
/mnt/sd/linux-lab/neui --panel 3     # jump straight to a panel
```

## Display refresh — command-mode panel (critical)

The **ZS070BE3019B3H7II** is a **command-mode MIPI panel**: mtkfb only pushes a
frame to the glass on **`FBIOPAN_DISPLAY`**, and skips the flush if the pan offset
is unchanged. A plain `memcpy` into the mmap updates *memory* but not the screen —
so the first (startup pan) frame shows and everything after looks frozen, even
though the app's state is updating. This also masked the bug: memory-read
screenshots looked perfectly live while the physical panel sat still.

**Fix (`fb_present` in fbgfx.h):** cycle through the 3 hardware buffers
(`yres_virtual/yres`) each present — draw to the next buffer, `FBIOPAN_DISPLAY`
to that offset — so every frame changes offset and forces a real refresh. Result:
smooth ~1 Hz updates + instant repaint on tap.

## How it works (notes for next agent)

- **Transport:** `base64 -d | gunzip` receiver on device; host streams chunked
  base64 ending in Ctrl-D. Pull side wraps output in `__B64_BEGIN__/END__`
  sentinels *split with `""`* so the echoed command line doesn't false-trigger.
  ACM can drop a byte under flood → pull verifies md5 and retries.
- **Pan to 0:** neofb `FBIOPAN_DISPLAY`s the visible pane to yoffset 0 so it maps
  to the start of `/dev/fb0` (makes `--shot` / capture deterministic).
- **Glyphs:** 8-bit coverage font, alpha-blended over BG; integer-scaled for
  headings. `textg()` adds a 4-way dim-offset bloom = cyberdesign text-shadow.
- **Scanlines:** final pass unpacks each pixel, darkens 2 of every 4 rows ~18%.

## Boot face (autostart)

`neui` is embedded in the L1 initramfs and launched by `init` at boot, so it
paints the screen on power-on instead of the DIGILAND logo. Built by
`experiments/linux-initramfs/build_rootfs.sh` (installs `bin/neui`+`bin/neofb`,
adds a respawn supervisor to `init` — started AFTER the serial shell so a broken
UI still leaves a recovery shell on `ttyGS0`).

```bash
experiments/fbui/build.sh                         # binaries
experiments/linux-initramfs/build_rootfs.sh       # embed + init
python3 experiments/linux-initramfs/pack_linux_boot.py \
    --output experiments/linux-initramfs/out/boot-linux-l1-neonos.img
./tools/flash-neonos.sh                            # flash (sudo, Preloader, user present)
```

- Image: `out/boot-linux-l1-neonos.img` (~6.55 MB). **Kernel = stock zImage,
  unchanged** — only the ramdisk differs, so far lower risk than the L1.4 patch.
- Known-good `out/boot-linux-l1.img` is left intact as the recovery target.
- **Recovery ladder:** `./tools/flash-neonos.sh restore` (plain L1, serial shell) →
  `./tools/flash-neonos.sh restore-stock` (stock Android).

## Done

- [x] `neofb` dashboard on mtkfb, cyberdesign-styled
- [x] `neui` **touch launcher** + 5 live panels (SYSTEM/PROCESS/STORAGE/KERNLOG/NETWORK)
- [x] evdev touch on event4, 1:1 axis map, tap-to-switch confirmed
- [x] **action bar** — SYNC / CLR LOG / REMOUNT / REBOOT(confirm) with toast feedback,
      dispatch verified headlessly via `--tap`

## Next targets

1. **Boot splash + autostart** — embed `neofb`/`neui` in the L1 initramfs and have
   `init` launch the UI at boot (replaces the DIGILAND logo as the OS face).
   **Requires reflash** of `boot-linux-l1.img` → do with user present (bootloop
   risk; restore path known).
2. **Output-capturing action tiles** — run a shell cmd and render its stdout in an
   overlay (extend `ACT[]` with a variant that pipes into a scroll view).
3. **Long-press / gestures** — scroll KERNLOG/PROCESS, swipe between panels.
4. **Network panel goes live** once CONSYS/Wi-Fi is unblocked (see [15](15-consys-power-path.md)).
5. **Theme variant** — cyberdesign `theme-orange` palette swap (flag or tile).
