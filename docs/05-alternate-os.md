# Alternate OS research (LineageOS, postmarketOS, etc.)

> **Project role:** Phase D moonshot only.  
> Ready-made alt OS is **out of scope** as a goal. See [00-charter.md](00-charter.md).

## Executive summary

**There is no ready-made modern OS image for DigiLand DL7006-KB / XMF-MID7006.**

A full OS replacement is a **device porting project**: kernel, device tree, bootloader quirks, Wi-Fi firmware, touch IC, display panel, audio, etc. For a 1 GB MT8127 white-label tablet, that is a deliberate moonshot — not a weekend flash.

**Recommendation:** complete Phase A (own flash) and ideally Phase B (own boot) before any port attempt. Stock Android remains the only practical full OS.

## What “supported” would look like

| Platform | Ready-made for DL7006? | Notes |
|----------|------------------------|-------|
| Official LineageOS | **No** | No device codename / builds found |
| Unofficial Lineage | **None known** for this exact SKU | Related MT8127 ports exist for *other* products |
| postmarketOS | **No device page** found for DL7006 | Old MT65xx tablet SoCs are hard mode |
| Ubuntu Touch | **No** | Needs active port maintainers |
| Generic GSI (Treble) | **Extremely unlikely** | Android 7 era, pre-Treble white-label |

## Related hardware ports (inspiration only)

These are **not** drop-in ROMs for DigiLand:

- **LeapFrog Epic / Quanta “narnia”** — community Lineage-oriented work on **MT8127** (GitHub orgs such as `mt8127-tadpole`). Useful as a *reference* for SoC bring-up patterns, not a flash package for DL7006.
- Other cheap MT8127 tablets may share kernel trees from MediaTek **ALPS** BSPs (`alps-mp-n0…` style tags appear in DL7006 firmware filenames online).

Shared SoC ≠ shared:

- GPIO / regulator maps  
- Display panel init  
- Touch controller (I2C ID)  
- Wi-Fi/BT combo module and NVRAM  
- Partition sizes / scatter  

## MediaTek mainline / pmOS context

postmarketOS and mainline efforts for MediaTek skew toward better-documented or more popular chips. Older MT8127 tablet platforms typically rely on **downstream Android 3.x/4.x kernels** from OEM BSPs.

Mainline trees named around **mt81xx** often target Chromebook-class parts (e.g. later MT81xx), not this exact budget tablet design. Do not assume Chromebook images will boot.

## If you still want to attempt a port

### Prerequisites

1. Full **readback** of eMMC / key partitions from a working unit.  
2. Stock **boot.img** + kernel modules extracted.  
3. Kernel **defconfig** / `proc/config.gz` if available.  
4. Working serial console (UART test points) — huge time saver; may need board tracing from FCC photos.  
5. Willingness to recover via **Preloader + SPFT** repeatedly.

### High-level path

1. Extract kernel from `boot.img` (`unpack_bootimg` / `split_boot`).  
2. Identify kernel version (`Linux version 3.x.x…`).  
3. Search for matching ALPS / MT8127 kernel sources.  
4. Build recovery (TWRP-class) for this partition layout.  
5. Only then consider Lineage 14.1-era (Android 7) ports — newer Android needs newer HALs you will not have.  
6. postmarketOS would mean either downstream kernel packaging or multi-year mainline bring-up.

### Realistic outcome matrix

| Outcome | Probability without UART |
|---------|--------------------------|
| Soft-brick + restore stock | High if you experiment |
| Rooted stock | Medium (classic Magisk/boot patch path) |
| Custom recovery | Medium–low |
| Daily-driver custom Android | Low |
| Usable mainline Linux + Wi-Fi + touch | Very low without sustained reverse engineering |

## Better “new OS” framing for this hardware

Think of **software roles**, not distro logos:

| Role | Implementation |
|------|----------------|
| Light UI shell | Fully Kiosk + debloated Android 7 |
| Automation brain | HA on another machine |
| Local sensors | Prefer ESPHome / Zigbee devices, not tablet sensors |
| Long-term OS updates | Not available; freeze and firewall the tablet |

A locked-down Android 7 kiosk that only talks to your HA instance on LAN is a **new purpose**, even if it is not a new kernel.

## Decision gate

Proceed with alternate OS work only if **all** are true:

- [ ] HA kiosk already works (or is rejected for a concrete software reason)  
- [ ] Stock firmware restore proven on this unit  
- [ ] Full dump archived  
- [ ] You want learning / porting more than a dashboard  

Otherwise stay on [04-home-assistant-panel.md](04-home-assistant-panel.md).
