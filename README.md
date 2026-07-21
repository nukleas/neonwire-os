# NEONWIRE OS

Reverse-engineer a **$40 white-label Android tablet** — a DigiLand **DL7006-KB**
(MediaTek **MT8127**) — and turn it into **NEONWIRE OS**: a self-built cyberpunk
Linux that boots from its own neon splash straight into a touch framebuffer UI,
joins Wi-Fi, is reachable over SSH from anywhere, and **synthesizes a full
soundtrack in real time on its own CPU**.

📟 **The build story (with live screen captures + audio):**
https://claude.ai/code/artifact/53bcb58a-8865-4016-8648-25d2aad089af

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

## Repo layout

```text
experiments/
  neonui/       the NEONWIRE shell — Rust workspace (neon-gfx, neonwire, neon-songs)
  fbui/         original C framebuffer UI (now recovery/reference) + the dossier build
  camera/       camgrab (raw MIPI capture) + host debayer / CCM tooling
  net/          Wi-Fi / CONSYS / SSH / Tailscale bring-up scripts
  consys-pwr/   CONSYS power-on reverse-engineering (patch + repack tooling)
  audio/        codec bring-up notes
docs/           charter, runbooks, per-subsystem devlogs, living checklist
tools/          flashing helpers, ISP diff tooling
```

> **Not included:** reverse-engineered vendor material (firmware, extracted
> libraries, upstream kernel/vendor trees, captured device logs) lived under
> `reference/` and is **excluded from this repository** — it is proprietary
> vendor content, not ours to redistribute. The RE *writeups* and *tooling* are
> here; the vendor *binaries* are not.

## Building the shell

The shell is `armv7-unknown-linux-musleabihf`, static, built with a musl cross
toolchain. From `experiments/neonui/`:

```sh
cargo build --release            # -> target/armv7-.../release/neonwire
```

The music engine depends on **strudel-rs** (the Rust Strudel port) as a pinned
git dependency from Codeberg — no local checkout or crates.io publish needed; a
fresh clone resolves it automatically. Deploy is a straight copy to the SD card
over SSH.

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
