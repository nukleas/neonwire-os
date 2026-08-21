# Project charter — DL7006 reverse-engineering kit

> **Where this actually went.** The charter below is the original mission:
> own flash, own boot, map hardware, optional Linux moonshot. All of that
> happened, then the moonshot became the project — a self-built Linux with a
> neon framebuffer OS, Wi-Fi, camera, live synthesis, and an on-device agent.
> Read this file as the starting intent. The living snapshot is the
> [root README](../README.md); the path through the notebook is
> [docs/README.md](README.md).

## One-sentence mission

Use a throwaway DigiLand **DL7006-KB** (MT8127) as a **lab platform** to learn MediaTek flash, Android boot chains, and board-level RE — shipping notes, dumps, and restore scripts, not a consumer product.

## Non-goals

| Non-goal | Why |
|----------|-----|
| GrapheneOS / Calyx / modern privacy OS | Pixel-class hardware only |
| Official Lineage / pmOS daily driver | No port exists; 1 GB is a poor target |
| Production Home Assistant wall panel | Possible later as a side quest; not the mission |
| “Find a simple OS to flash” | Stock Android is the only practical full OS |
| Keeping the unit pristine | Brick/restore is expected |

## Goals

1. **Own flash** — reliable Preloader access; dump; restore from our archive.  
2. **Own boot** — understand and modify boot/recovery path.  
3. **Map hardware** — document SoC peripherals, partitions, ICs.  
4. **Optional moonshot** — serial Linux or legacy custom Android bring-up.  
5. **Write it down** — this repo is the deliverable.

## Phases

### Phase A — Own flash (primary, now)

**Intent:** The unit is never “lost.” Anything we break, we restore.

| Milestone | Done when |
|-----------|-----------|
| A1 Host tooling | mtkclient venv + udev; `mtk.py --help` works |
| A2 Preloader on demand | `0e8d:2000` appears predictably; tool attaches |
| A3 GPT | `printgpt` (or equivalent) captured in repo |
| A4 Dump | At least `boot`, `recovery`, and full list of partitions; better: `rl` dump set |
| A5 Integrity | SHA256 for every dumped image |
| A6 Restore proof | Re-write a non-destructive or previously dumped partition and still boot **or** full restore from dump after intentional wipe of a safe partition |
| A7 Runbook | [08-phase-a-own-flash.md](08-phase-a-own-flash.md) matches what actually worked |

**Exit criteria:** “I can brick userdata/system experiments and get back using files under `reference/dumps/`.”

Third-party stock ZIPs are a **backup**, not the primary truth. Self-dump wins.

### Phase B — Own boot

**Intent:** Control early boot without OEM UI.

| Milestone | Done when |
|-----------|-----------|
| B1 Unpack | `boot.img` / recovery unpacked (kernel, ramdisk, cmdline) |
| B2 Document | Kernel version, cmdline, verified boot / encrypt flags noted |
| B3 Modified boot | Repacked boot or init tweak flashes and boots |
| B4 Recovery | Custom or stock recovery usable for sideload/wipe |
| B5 Root path | Magisk or older root strategy documented (success or failure with reasons) |

**Exit criteria:** Power-on path we understand and can alter safely.

### Phase C — Map hardware

**Intent:** Know what is on the board beyond “MT8127 tablet.”

| Milestone | Done when |
|-----------|-----------|
| C1 Partition map | Named partitions + sizes in repo |
| C2 Kernel surface | Modules/drivers list, `/sys` / dmesg captures when possible |
| C3 Peripherals | Touch, Wi-Fi/BT, panel IDs identified or “unknown” with evidence |
| C4 Physical | FCC photos annotated; optional UART pads if pursued |
| C5 Sibling research | Notes on related MT8127 devices (e.g. port trees) — comparison only |

**Exit criteria:** A hardware map page another engineer could use.

### Phase D — Moonshot (optional)

Pick **at most one** active moonshot:

- **D-Linux:** mainline or downstream kernel to **UART shell** (display optional later)  
- **D-Android:** Lineage 14.1–era / AOSP 7 bring-up from closest MT8127 trees + our dump  

**Exit criteria:** Defined tiny win (e.g. “kernel panic text on serial”) not “full desktop.”

## Side quests (explicitly optional)

| Quest | When it makes sense |
|-------|---------------------|
| Stock reflash from Everest/Digiland ZIP | Dump incomplete; need external images ([07](07-option-c-stock-reflash.md)) |
| HA kiosk on stock | Want a demo UI after Phase A is solid ([04](04-home-assistant-panel.md)) |
| Userdata-only wipe | Faster “clean slate” than full rewrite |

Side quests must not destroy the only good dump.

## Operating principles

1. **Dump → hash → experiment → restore.**  
2. **One variable per flash.**  
3. **Repo is truth** — if it isn’t written down, it didn’t happen.  
4. **Stock Android is the base OS**; custom work layers on top of understanding stock.  
5. **Throwaway hardware, not throwaway discipline** — still avoid casual NVRAM wipes.

## Roles of tools

| Tool | Role |
|------|------|
| mtkclient | Primary Linux Preloader/BROM client |
| SP Flash Tool | Optional alternate flasher (often Windows) |
| adb | Nice when UI works; **not** required for Phase A |
| Third-party ROM sites | Contingency images; untrusted until hashed + compared to dump |

## Definition of project success (overall)

The project succeeds if **all** of the following are true:

- [ ] Phase A exit criteria met  
- [ ] Phase B or C has tangible artifacts in `docs/` + `reference/`  
- [ ] A stranger could restore the device from this repo’s instructions + dumps metadata  
- [ ] We no longer ask “what OS can I install?” — we know the answer and chose deeper work instead  

## Current decision (2026-07-18)

- **Primary path:** Phase A (own flash)  
- **Deferred:** HA as primary goal  
- **Rejected as goals:** Graphene, ready-made alt OS  
- **Moonshot:** parked until A (+ preferably B) complete  
