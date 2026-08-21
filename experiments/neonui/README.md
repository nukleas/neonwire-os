# neonui — the NEONWIRE shell

A static `armv7-unknown-linux-musleabihf` Rust workspace that paints a touch UI
straight onto the MT8127 command-mode panel. No X, no Android, no toolkit.

| Crate | Role | License |
|-------|------|---------|
| `neon-gfx` | framebuffer, font, touch, draw primitives | MIT |
| `neon-songs` | on-device Strudel engine | AGPL-3.0-or-later |
| `neonwire` | the shell that links both | AGPL-3.0-or-later |

The split and the Strudel / TidalCycles attribution:
[LICENSING.md](LICENSING.md).

## Build

```sh
rustup target add armv7-unknown-linux-musleabihf

# .cargo/config.toml points at one machine's musl cross-GCC. Override:
export CARGO_TARGET_ARMV7_UNKNOWN_LINUX_MUSLEABIHF_LINKER=/path/to/armv7l-linux-musleabihf-gcc

cargo build --release
# -> target/armv7-unknown-linux-musleabihf/release/neonwire
```

`rust-toolchain.toml` pins `stable` plus the armv7 musl target. The music path
pulls [strudel-rs](https://codeberg.org/nukleas/strudel-rs) at a pinned rev
(needs the arm32 `ContextKeyBitset` align(16) fix or the layout assert fires).

On the tablet the binary is bind-mounted over `/bin/neui` so the C HUD in the
initramfs stays as recovery: `umount` rolls back. Headless checks used in
bring-up: `--shot`, `--tap`, `--ticks` (see `neonwire/src/main.rs`).

## Device config (not in git)

The shell reads optional files from `/mnt/sd/linux-lab/` — Home Assistant URL +
token, xAI key for voice, ZeroClaw pairing token, wpa state. See
[SECURITY.md](../../SECURITY.md).
