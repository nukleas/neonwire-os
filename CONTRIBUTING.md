# Contributing

This is a personal lab notebook that happens to be public, not a product with a
roadmap. The most useful contributions are:

- **Factual fixes** — a register offset that is wrong, a command that drifted,
  a note that still describes a blocker we later solved.
- **"I tried this on sibling hardware"** — other MT8127 / Lightcomm MID* units.
  A short note beats a speculative port.
- **Build / docs clarity** — especially anything that would have saved you an
  hour on a fresh clone.

Please do **not** send:

- Vendor firmware, extracted libs, kernel trees, or eMMC dumps. They are
  gitignored for a reason. Point at a public source if you have one.
- Secrets, Tailscale IPs, SSIDs, Home Assistant URLs, or API keys.
- Drive-by refactors of working on-device code. The 3.18 / musl / command-mode
  panel combination is load-bearing and easy to "clean up" into a black screen.

## How to work

Read [docs/README.md](docs/README.md) first so you are not implementing against
a handoff doc whose blocker was already falsified.

Match the surrounding style: small C tools, musl-static ARM, notes written as
what actually happened. The assembled `neonwire` binary is AGPL because it
links the Strudel-derived music crate; `neon-gfx` on its own is MIT. See
[experiments/neonui/LICENSING.md](experiments/neonui/LICENSING.md).

Issues are welcome. A PR should be something you ran, or a docs change you
would have wanted on the way in.
