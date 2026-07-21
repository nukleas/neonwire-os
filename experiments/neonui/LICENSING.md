# Licensing

This workspace is **dual-licensed by component**. The split is not cosmetic —
it reflects which code is original and which derives from
[Strudel](https://strudel.cc) (AGPL-3.0).

| Crate        | License              | Why                                                            |
|--------------|----------------------|----------------------------------------------------------------|
| `neon-gfx`   | **MIT**              | Original framebuffer/graphics engine. No Strudel lineage.      |
| `neon-songs` | **AGPL-3.0-or-later**| Derivative of Strudel — links strudel-rs, ports strudel-audio. |
| `neonwire`   | **AGPL-3.0-or-later**| The shipped binary links `neon-songs`, so the whole program is AGPL. |

License texts: [`LICENSE-MIT`](LICENSE-MIT), [`LICENSE-AGPL`](LICENSE-AGPL).

## What you can reuse, and under what terms

- **`neon-gfx` on its own is MIT.** Take the MT8127 command-mode-panel handling,
  the baked bitmap-font renderer, or the neon draw primitives into any project,
  including proprietary ones — just keep the copyright notice.
- **Anything that includes `neon-songs`, or the assembled `neonwire` binary, is
  AGPL-3.0-or-later.** If you distribute a build, ship the corresponding source.

In practice the AGPL costs almost nothing here: `neonwire` paints a local
framebuffer and serves nobody over a network, so AGPL's network clause never
triggers — it behaves like plain GPL-3 ("ship the source with the binary").

## Attribution — the Strudel lineage

`neon-songs` is a **derivative work of Strudel** (<https://strudel.cc>,
`AGPL-3.0-or-later`, © Strudel contributors), which itself descends from
[TidalCycles](https://tidalcycles.org). This crate:

- links the `strudel-rs` crates (the author's Rust port of Strudel), and
- ports `strudel-audio`'s `processor` / `mapper` / `channel` / `scheduler`
  modules (upstream hard-requires `cpal`, unusable on static musl).

Strudel's pattern language, mini-notation grammar, voicing dictionaries, and
soundfont catalogue originate with the Strudel project. All credit for that
design and data is theirs; this workspace only adapts it to run on the DL7006.

## Note on the wider repository

Other original work in this repository outside this workspace (the framebuffer
tooling, the reverse-engineering utilities, capture tools, and docs) is the
author's own and not Strudel-derived. Reverse-engineered vendor material
(firmware, extracted libraries, kernel/vendor trees under `reference/`) is
**not** covered by these licenses and is **not** redistributable — it is
excluded from any published release.
