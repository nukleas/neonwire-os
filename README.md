# NEONWIRE OS — DL7006 reverse-engineering + custom OS

Throwaway-lab project: reverse-engineer a **DigiLand DL7006-KB** (FCC **XMF-MID7006**, MediaTek **MT8127**) white-label tablet — and turn it into **NEONWIRE OS**, a self-built cyberpunk Linux that boots from its own neon splash straight into a touch framebuffer UI.

📟 **Build story / dossier:** https://claude.ai/code/artifact/53bcb58a-8865-4016-8648-25d2aad089af

## North star

```text
Phase A  Own flash     ✅ full eMMC dump + hashes (session-20260718)
Phase B  Own boot      ✅ boot/recovery/LK unpacked; custom boot images
Phase C  Map hardware  ✅ partitions, panel, touch, PMIC, DTB
Phase D  Self-built Linux (L1)  ✅ 3.18.35 + busybox + USB-ACM root shell
Phase E  NEONWIRE OS face       ✅ framebuffer UI + touch + boot-face + logo
         └─ Wi-Fi / CONSYS       ✗ blocked (no board sources, MODULES=n)
```

The device **boots straight into NeonOS**: a from-scratch framebuffer engine
(`experiments/fbui/`) draws a neon dashboard + touch launcher on the panel, and a
custom splash replaces the vendor logo. See [docs/17-cyberpunk-fbui.md](docs/17-cyberpunk-fbui.md)
and [docs/00-charter.md](docs/00-charter.md).

## Device (confirmed from dumps)

| Field | Value |
|-------|--------|
| Model | DigiLand **DL7006** |
| SoC | **MT8127** (unprotected Preloader) |
| Android | **7.0** NRD90M build `1492498939` |
| Fingerprint | `digiland/DL7006/DL7006:7.0/NRD90M/1492498939:user/release-keys` |
| RAM / eMMC UA | 1 GiB / 7.125 GiB |
| Preloader | `preloader_mid7006al.bin` (in eMMC boot1) |

## Repo layout

```text
docs/           charter, runbooks, checklist
tools/          mtkclient (git), venv, unpack/hash helpers
reference/dumps/session-20260718/   ← primary archive
  images/       boot.img recovery.img system.img preloader.bin
  raw/          flash-user.bin, boot1/2, mbr-slots, prefix
  meta/         build.prop, printgpt logs
  work/         Phase B unpack outputs
  MAP.md        byte offsets for write-back
```

## Quick paths

| Task | Command / path |
|------|----------------|
| Dump map | [reference/dumps/session-20260718/MAP.md](reference/dumps/session-20260718/MAP.md) |
| Unpack boot | `python3 tools/unpack_bootimg.py reference/dumps/session-20260718/images/boot.img reference/dumps/session-20260718/work/boot` |
| Phase B runbook | [docs/09-phase-b-own-boot.md](docs/09-phase-b-own-boot.md) |
| **Flash ADB boot** | [docs/10-boot-adb-patch.md](docs/10-boot-adb-patch.md) · `./tools/flash-boot-adb.sh` |
| Skip wizard | [docs/11-skip-setup-wizard.md](docs/11-skip-setup-wizard.md) |
| HA client | [docs/12-ha-companion-client.md](docs/12-ha-companion-client.md) |
| **Linux port research** | [docs/13-linux-port-research.md](docs/13-linux-port-research.md) · `reference/upstream/` |
| **Live Linux lab (L1.3)** | [docs/14-live-linux-lab.md](docs/14-live-linux-lab.md) · USB shell, mounts, fb |
| **CONSYS / Wi‑Fi power** | [docs/15-consys-power-path.md](docs/15-consys-power-path.md) |
| **Handoff (multi-agent)** | [docs/16-handoff-linux-consys.md](docs/16-handoff-linux-consys.md) · current L1 status + next steps |
| **NEONWIRE fbui (OS face)** | [docs/17-cyberpunk-fbui.md](docs/17-cyberpunk-fbui.md) · `cd experiments/fbui && ./build.sh && python3 push.py neofb.gz` |
| **Flash boot face** | `./tools/flash-neonos.sh` (restore: `restore` / `restore-stock`) |
| **Flash NEONWIRE logo** | `python3 experiments/fbui/make_logo.py --build …/logo-neonos.bin && ./tools/flash-neonos.sh logo` |
| **Build the dossier** | `python3 experiments/fbui/build_dossier.py <scratchpad>` |
| Preloader attach | `source tools/venv/bin/activate && cd tools/mtkclient && sudo $(which python) mtk.py printgpt` |
| Checklist | [docs/checklist.md](docs/checklist.md) |

## Documentation index

| Doc | Purpose |
|-----|---------|
| [docs/00-charter.md](docs/00-charter.md) | Mission & phases |
| [docs/01-device-identity.md](docs/01-device-identity.md) | FCC / USB IDs |
| [docs/08-phase-a-own-flash.md](docs/08-phase-a-own-flash.md) | Dump / Preloader runbook |
| [docs/09-phase-b-own-boot.md](docs/09-phase-b-own-boot.md) | **Active** boot work |
| [docs/03-flashing.md](docs/03-flashing.md) | SP Flash / mtk concepts |
| [docs/07-option-c-stock-reflash.md](docs/07-option-c-stock-reflash.md) | Third-party ZIP contingency |
| [docs/05-alternate-os.md](docs/05-alternate-os.md) | Why no Graphene/GSI |
| [docs/04-home-assistant-panel.md](docs/04-home-assistant-panel.md) | Side quest |
| [docs/checklist.md](docs/checklist.md) | Living checklist |

## Safety

1. Dump before write; restore from `session-20260718`.  
2. One `mtk.py` process at a time; fresh Preloader plug per command.  
3. Prefer `wo` at known offsets over full `wf`.  
4. Large bins are gitignored — keep `SHA256SUMS` + docs in git.

## License / disclaimer

Personal RE notes on hardware you own. Flashing can brick devices.
