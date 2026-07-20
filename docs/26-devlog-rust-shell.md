# Devlog — the OS grows up (Rust shell, sound, a camera that breathes)

**Date:** 2026-07-19 → 2026-07-20
**One line:** Rewrote the whole OS face from C into a Rust app platform, gave it real
sound, ported every old panel plus a Wi-Fi manager, brought the camera sensor to life,
and added power management — all live on the tablet, all reachable over Tailscale.

Continues from [25-devlog-tailscale.md](25-devlog-tailscale.md). Going in, the device
cold-booted onto Wi-Fi/SSH/Tailscale into the C `neui` HUD (mostly system panels). This
session turned that HUD into an actual little operating system.

## First sound

The MT8127's speaker had never made a noise under our Linux. The
[audio recipe](../experiments/audio/audio-recipe.md) from the stock-HAL capture said it
should be trivial — one mixer control (`Speaker_Amp_Switch On`) plus a live PCM stream on
`hw:0,5`, letting DAPM auto-power the DAC/clock. It was: a clean 440 Hz tone came out of
the tablet. Hard driver facts learned: `hw:0,5` = S16_LE/2ch/44100, period **2048** with
buffer **4096 as a hard max** (bigger requests clamp; `speaker-test`'s tiny writes underrun
into an ugly rasp — `aplay` with the right geometry plays clean).

## The Rust rewrite

Decision: port the C engine + shell to Rust and build a real app platform. New workspace
`experiments/neonui/` (`neon-gfx` engine + `neonwire` shell), static
`armv7-unknown-linux-musleabihf`, deps = **`libc` only**. Milestones, each verified on
glass before the next:

- **M0** — a validation probe retired every toolchain risk: musl-1.2 `time64` falls back
  via ENOSYS on the 3.18 kernel, `getrandom` works, and **strudel-core evaluated a pattern
  cycle on-device** (proving the edition-2024 path deps cross-compile and run).
- **M1/M2** — ported `fbgfx.h`: the PAN-cycle present (command-mode MIPI panel contract
  preserved), canvas/draw/font/theme with the cyberdesign tokens and a switchable
  5-accent theme system. A test card rendered pixel-identical to the C engine.
- **M3** — evdev touch: the load-bearing tap machine (arm on `BTN_TOUCH` press edge, fire
  on `SYN_REPORT`), with a **hand-declared 16-byte `input_event`** because libc's musl-1.2
  layout is wrong for this 32-bit 3.18 kernel.
- **M4** — the shell: a top status bar (Wi-Fi/Tailscale IP, battery, mem, CPU, uptime), a
  home tile grid, an `App` trait + `HitMap` (topmost-wins tap resolution, replacing the C
  global rect arrays).
- **M5** — SYSTEM app: INFO/PROC/DISK/KLOG tabs + the DF/MEM/MOUNTS/DMESG tool overlay +
  a two-tap REBOOT confirm.
- **M6** — the full Wi-Fi manager: wpa ctrl-socket client, scan list with RSSI bars,
  known-network fast-rejoin, the join→DHCP→telnetd state machine, and the 3-page PSK
  keyboard generalized into a reusable `TextPrompt` widget.

The C `neui` stays in the initramfs as a permanent recovery face; the Rust shell runs via
a `mount --bind` over `/bin/neui` (instant rollback with `umount`).

## Sound with a beat

The MUSIC app became a real drum machine. A libasound-free PCM writer talks straight to
`/dev/snd/pcmC0D5p` via tinyalsa-style ioctls (structs copied from the **3.18** uapi
`asound.h` — they grew fields in later kernels). On top, an audio thread runs a
**strudel-core** pattern as the clock: it queries a `Pattern` per 2048-frame block and
fires hand-rolled synth drum voices at exact sample offsets. Mini-notation presets
(`bd*4, [~ hh]*4, ~ sd ~ sd`, euclidean `bd(3,8)`) eval on-device through `strudel-mini`.
The drums slap.

## A camera that breathes

The big one. The rear **SP2509** sensor had read `0x0000` on every prior attempt. Mining
the stock capture logs pinned it: the kernel never wires the sensor master clock — the HAL
does it from userspace, and it programs the seninf **TG1 phase counter** before reading
the ID. Our probe had set only `ADCLK_EN` (bit 29); the actual output enable is **`PCEN`
(bit 31)**. One bit. With `SENINF_TG1_PH_CNT = 0xA0000001`, the sensor clocks and reports
**`0x2509`**, reproducibly. The Camera app shows a green SENSOR ONLINE.

Then A3 — capturing an actual frame. Built the entire HAL-free pass1 pipeline
([`camgrab.c`](../experiments/camera/camgrab.c)) and it runs clean, no crashes:
- **ion + M4U buffer** (the make-or-break): `/dev/ion` multimedia-heap alloc →
  `CONFIG_BUFFER(CAM_IMGO=17)` → `GET_PHYS` MVA → **`MTK_M4U_T_CONFIG_PORT(Virtuality=1)`**
  via `/proc/M4U_device` makes the MVA translate. A safety gate aborts before enabling the
  DMA if the port config fails (else the engine would emit the MVA as a raw physical
  address into low memory).
- Sensor streaming (X_CONTROL preview — dmesg confirms `SP2509MIPIPreview` ran), CSI-2
  digital + analog setup, ISP TG1 grab window + IMGO DMA config, all from the verified
  `isp_reg.h` offsets.

**Where it stopped:** `PASS1_TG1_DON` times out — the TG frame/line counters stay 0, so no
MIPI data reaches the ISP. Every structural unknown (M4U, DMA target, pass1 trigger)
*works*; the remaining gap is the **CSI-2 receiver locking** onto the sensor's stream.
Parked one MIPI handshake short of an image. Next: capture stock's exact SENINF/analog
register values (the existing capture is logcat, not register dumps) or disassemble the
HAL's `setTg1CSI2`.

## Keeping the lights on (barely)

The tablet ran flat to 0% and died mid-session — no power management at all. Added, in the
shell:
- **Idle backlight-blank** (`/sys/class/leds/lcd-backlight`): 30 s no-touch → off; touch →
  on, first tap swallowed. Physically confirmed dims + wakes.
- **Low-battery safety**: a red banner under 15%, and a clean `sync` + poweroff before 0%
  (debounced, charging-guarded) so we never hard-crash empty again.
- Fixed the status bar to show **real CPU%** from `/proc/stat` deltas instead of the
  loadavg ~8 — which turned out to be MTK vendor kthreads stuck in D-state, not load (CPU
  is ~90% idle).

Suspend-to-RAM (`echo mem`) is the biggest remaining battery win but too risky to test
remotely — a bad resume hangs the tablet until a physical power-cycle. Saved for in-person.

## State at end of session

Every hardware subsystem on this $40 tablet is now reachable from our own OS: display,
touch, Wi-Fi, Tailscale, **audio**, and the **camera sensor** (clocked + identified; frame
capture built, one handshake away). The UI is a real Rust app platform with power
management. Open threads: camera CSI-2 lock, strudel live-coding input, suspend, and the
soak + native-boot swap to retire the bind-mount.
