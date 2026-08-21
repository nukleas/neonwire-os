# NEONWIRE OS

Reverse-engineer a **$40 white-label Android tablet** — a DigiLand **DL7006-KB**
(MediaTek **MT8127**) — and turn it into **NEONWIRE OS**: a self-built cyberpunk
Linux that boots from its own neon splash straight into a touch framebuffer UI,
joins Wi-Fi, is reachable over SSH from anywhere, and **synthesizes a full
soundtrack in real time on its own CPU**.

This repository is a **field notebook**, not a ROM. The notes keep the wrong
turns. The code is what actually runs on the glass. If you want the story with
live screen captures and audio first:

**[naderlabs.io/projects/neonwire](https://www.naderlabs.io/projects/neonwire)**

Then come back here and read [docs/README.md](docs/README.md) for the path
through the notebook.

## What it does now

The tablet's one weakness was an **unprotected bootloader**. That was enough to
take the whole device — dump it, boot our own Linux, and keep going until it did
things the vendor never shipped:

- **Own boot chain** — custom boot image + a neon splash flashed over the vendor
  logo; `init` launches the UI straight into NEONWIRE (crash-loop guard falls
  back to a recovery UI).
- **NEONWIRE shell** (`experiments/neonui/`) — a single static Rust binary that
  draws a neon dashboard, nav rail, and touch apps directly to the panel. No X,
  no Android, no toolkit. Reverse-engineers the command-mode MIPI panel and a
  type-A multitouch driver to put a real touch UI on glass.
- **Networking** — the on-die **CONSYS** Wi-Fi radio brought up from scratch,
  plus dropbear SSH over **Tailscale** — reachable from anywhere.
- **Camera** — a live viewfinder off a raw MIPI SuperPix SP2509 sensor (no V4L2
  on this kernel; the pipeline is programmed by hand).
- **Sound** — a bare ALSA device driven by a raw-ioctl PCM writer + native mixer
  ioctls, with an on-device **Strudel** engine that *synthesizes* songs (drum
  samples + synth voices) in real time — an event-rain / spectrum / scope
  visualizer included.
- **Mind** — a cross-compiled [ZeroClaw](https://github.com/zeroclaw-labs/zeroclaw)
  agent runtime on the tablet, with a touch/voice front-end and a supervised
  watchdog that reads the device's own logs.

## Who this is for

- **Readers.** Follow a complete cheap-SoC reverse-engineering arc: Preloader
  dump → own boot → self-built Linux → framebuffer OS → radio, camera, audio,
  agent. Start at [docs/README.md](docs/README.md).
- **Builders on the same hardware.** DigiLand **DL7006-KB** / FCC **XMF-MID7006**
  and close Lightcomm MID7006 siblings. Dump *your* unit first. This repo does
  not ship a flashable image or vendor firmware.
- **People stealing parts.** The framebuffer engine (`neon-gfx`), the CONSYS
  launcher (`wmtctl2.c`), the HAL-free camera path (`camgrab.c`), and the
  musl-static ALSA writer are meant to be lifted. Check [licensing](#license)
  before you ship a binary that includes the music path.

This is **not** a daily-driver OS, a Lineage port, or a "flash this ZIP"
project. Stock Android is still the only practical full OS for the board. NEONWIRE
is what happened after we stopped asking "what can I install?" and started
building.

## Repo layout

```text
docs/           field notebook — charter, runbooks, per-subsystem devlogs
experiments/
  neonui/       the NEONWIRE shell — Rust workspace (neon-gfx, neonwire, neon-songs)
  fbui/         original C framebuffer UI (now recovery/reference) + the dossier
  camera/       camgrab (raw MIPI capture) + host debayer / CCM tooling
  net/          Wi-Fi / CONSYS / SSH / Tailscale bring-up scripts
  consys-pwr/   CONSYS power-on reverse-engineering (patch + repack tooling)
  audio/        codec bring-up notes
tools/          flashing helpers, ISP diff tooling, publish/sync
```

> **Not included:** reverse-engineered vendor material (firmware, extracted
> libraries, upstream kernel/vendor trees, captured device logs) lived under
> `reference/` and is **excluded from this repository** — it is proprietary
> vendor content, not ours to redistribute. The RE *writeups* and *tooling* are
> here; the vendor *binaries* are not. Clone [mtkclient](https://github.com/bkerler/mtkclient)
> into `tools/mtkclient` yourself if you are going to talk to a Preloader.

## Building the shell

The shell is `armv7-unknown-linux-musleabihf`, static, built with a musl cross
toolchain. From `experiments/neonui/`:

```sh
# rustup target add armv7-unknown-linux-musleabihf
# point the linker at your musl cross-GCC if it isn't the path in .cargo/config.toml:
#   export CARGO_TARGET_ARMV7_UNKNOWN_LINUX_MUSLEABIHF_LINKER=/path/to/armv7l-linux-musleabihf-gcc
cargo build --release            # -> target/armv7-.../release/neonwire
```

More in [`experiments/neonui/README.md`](experiments/neonui/README.md). The music
engine depends on **strudel-rs** as a pinned git dependency from Codeberg — a
fresh clone resolves it. Deploy is a copy to the SD card over SSH.

The C recovery UI (`experiments/fbui/`) and the CONSYS/camera C tools use the
same musl cross-GCC (`CC` / `$HOME/toolchains/armv7l-linux-musleabihf-cross/...`
in the build scripts).

## Device-local secrets

Anything that authenticates — Home Assistant token, xAI key, GitHub PAT,
authorized_keys, Tailscale state, ZeroClaw pairing token — lives on the tablet
at `/mnt/sd/linux-lab/` and is gitignored. See [SECURITY.md](SECURITY.md).

## License

This repository is **MIT** ([`LICENSE`](LICENSE)) for the original
reverse-engineering, systems, and framebuffer work.

The **music path is AGPL-3.0-or-later**: `experiments/neonui/neon-songs` derives
from [Strudel](https://strudel.cc) (AGPL-3.0), and the `neonwire` binary links
it — so the assembled shell is AGPL as a whole. The per-crate split and the
Strudel / TidalCycles attribution are documented in
[`experiments/neonui/LICENSING.md`](experiments/neonui/LICENSING.md).

## Disclaimer

Educational reverse-engineering of hardware **you own** — right-to-repair
territory. No proprietary vendor blobs are redistributed here. Flashing an
embedded device can brick it; there is no warranty. Dump before you write.
See [SECURITY.md](SECURITY.md).
