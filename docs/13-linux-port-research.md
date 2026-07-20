# Linux / custom-OS port research — DL7006 (MT8127)

Research notes for turning this tablet into a **self-built Linux** experiment platform.  
Updated **2026-07-19** from dumps, live ADB probes, embedded DTB, firmware pull, and public repos.

---

## 1. Device / firmware identity (authoritative)

### From live device (`adb`)

| Field | Value |
|-------|--------|
| Kernel | **`Linux version 3.18.35`** `(liushen@midcompser)` **gcc 4.8** |
| Build stamp | `#2 SMP PREEMPT Tue May 9 16:02:08 CST 2017` |
| CPU | ARMv7 **Cortex-A7** ×4 (`0xc07`), SMP, NEON, VFPv4 |
| Hardware string | **`MT8127`** / `ro.hardware=mt8127` |
| RAM | **~1 GiB** (`MemTotal: 999176 kB`; DTB `memory@0x80000000` len `0x40000000`) |
| Display | Physical **1024×600** (landscape), density **160**; Android often reports portrait 600×1024 |
| Touch | **`mtk-tpd`** → **Chipone ICN85xx** (kernel strings) |
| Wi‑Fi | MTK WMT / CONSYS; iface `wlan0`; firmware **`WIFI_RAM_CODE_8127`** |
| FM | **MT6627** (`mt6627_fm_v1_*`) |
| BT | `blueangel` stack; WMT-tied |
| Accel | Live: **MC3410** (`/sys/module/mc3410`); HAL “MTK SENSORS” |
| Charger | **ETA6003** (`mediatek,eta6003`) |
| PMIC | **MT6323** (full regulator tree in DTB) |
| Camera | **SP0A09** + **SP2509** MIPI (kernel strings) |
| Carrier | `wifi-only` |

### From `build.prop` / MTK versioning

| Field | Value |
|-------|--------|
| Model / brand | DigiLand **DL7006** / **DL7006-KB** |
| Flavor | `full_mid7006al-user` |
| ALPS branch | **`alps-mp-n0.mp102`** (Android **N** / 7.0) |
| MTK release tag | **`alps-mp-n0.mp102-V2_hcn8127.tb.n_P16`** |
| Project path | **`device/hcn/mid721l_96e_mipi`** |
| Board platform | `mt8127` |
| Fingerprint | `digiland/DL7006/DL7006:7.0/NRD90M/1492498939:user/release-keys` |

Interpretation:

- OEM tree: **HCN / Lightcomm** white-label (`hcn8127`, `mid7006al`, `mid721l_96e_mipi`).
- Same board family appears in head-units and other tablets (INCAR mid7006al, MID721L boards).
- Kernel **3.18.35** is newer than most public MT8127 GPL dumps (**3.4 / 3.10**).

### From embedded DTB (stock zImage)

Extracted: `reference/blobs/devicetree/dtb-from-zImage-577fd8.dtb`

| Property | Value |
|----------|--------|
| Root compatible | `mediatek,mid7006al`, `mediatek,mt8127` |
| Model | `MediaTek tb8127 Development Board` |
| DRAM | `0x80000000` … size **1 GiB** |
| GPU | `arm,mali-450` / `arm,mali-utgard` @ `mali@13040000` |
| UARTs | **uart0–uart3** all `status = okay` (`mediatek,mt8127-uart`) |
| LCM | `mediatek,lcm`; power GPIO **0x53**, reset GPIO **0x54** |
| Touch DT | `mediatek,mt8127-touch` / `mediatek,cap_touch` |
| Connectivity | `consys@18070000` (`mediatek,mt8127-consys`) + 1 MiB reserved mem |
| Ram console | `0x83f00000` (64 KiB); pstore `0x83f10000` |
| Trustzone bootinfo | `0x80002000` |

**LK cmdline template** (from `lk.bin` strings):

```text
console=tty0 console=ttyS0,921600n1 console=ttyMT3,921600n1 root=/dev/ram androidboot.hardware=mt8127
```

→ serial console is designed at **921600** on **ttyMT3 / ttyS0**. UART pads on the PCB would unlock printk for Linux bring-up.

### From firmware dumps (this repo)

| Artifact | Path | Notes |
|----------|------|--------|
| Full user eMMC | `session-20260718/raw/flash-user.bin` | 7.125 GiB |
| Preloader | `images/preloader.bin` / boot1 | `preloader_mid7006al.bin` |
| LK / uboot | `images/lk.bin` @ `0x1d20000` | part size `0x60000` |
| boot.img | `images/boot.img` @ `0x1d80000` | page 2048, kernel `0x80008000`, ramdisk `0x84000000` |
| recovery | `images/recovery.img` @ `0x2d80000` | same kernel as boot |
| system | `images/system.img` | ext4 ~1.5 GiB |
| Map | `session-20260718/MAP.md` | flash offsets |

Boot chain:

```text
boot1: Preloader (mid7006al)
  → LK @ 0x1d20000
  → boot.img (zImage + ROOTFS) @ 0x1d80000
  → Android userspace
```

Security: Preloader **unprotected** (SBC/SLA/DAA off) — full restore always possible.

---

## 2. SoC reference (MT8127)

| Item | Spec (family / this device) |
|------|------------------------------|
| CPU | 4× Cortex-A7 ~1.3 GHz class |
| GPU | **Mali-450** Utgard (`mali@13040000`) |
| ISA | **armeabi-v7a** only (32-bit) |
| Process | ~28 nm tablet SoC (2014-era) |
| Display path | MTK DDP: OVL, RDMA, DSI/MIPI, BLS, COLOR, … |
| Storage | eMMC via **mtk-msdc** / `mediatek,mt8127-mmc` |
| USB | **mt_usb** / musb (`mediatek,mt8127-usb20`) |
| Connectivity | Integrated CONSYS (Wi‑Fi + BT + FM MT6627 path) |

Related public codenames: **narnia** (LeapFrog Epic), **ford/austin** (Amazon Fire 7 2015/2017), **ttab** (Alcatel/Telekom Puls), **tb8127** ALPS template.

---

## 3. Other people’s attempts (public)

### 3.1 LeapFrog Epic “narnia” (Quanta / MT8127) — **strong driver reference**

| Repo | What | Local |
|------|------|--------|
| [mt8127-tadpole/android_kernel_quanta_narnia](https://github.com/mt8127-tadpole/android_kernel_quanta_narnia) | GPL kernel from LeapFrog source CD | `reference/upstream/android_kernel_quanta_narnia` |
| [mt8127-tadpole/android_kernel_mediatek_mt8127-common](https://github.com/mt8127-tadpole/android_kernel_mediatek_mt8127-common) | Shared MT8127 kernel (**3.10.108**) | `…/android_kernel_mediatek_mt8127-common` |
| [mt8127-tadpole/android_device_quanta_narnia](https://github.com/mt8127-tadpole/android_device_quanta_narnia) | LineageOS device tree | cloned |

**Highlights:**

- `TARGET_BOARD_PLATFORM := mt8127`, GPU `mali-450mp4`
- `BOARD_KERNEL_BASE := 0x80000000`, pagesize **2048** (matches us)
- Wi‑Fi: `mt66xx` / `/dev/wmtWifi`
- **Touch: `icn85xx`** — same IC family as our stock kernel strings

Kernel version still **3.4 / 3.10**, not 3.18.

### 3.2 Amazon Fire 7 “ford” / “austin” — **best “Linux distro on MT8127” prior art**

| Repo | What | Local |
|------|------|--------|
| [cm14-mt8127/kernel_amazon_mt8127-common](https://github.com/cm14-mt8127/kernel_amazon_mt8127-common) | **3.10.108** Amazon common kernel | `kernel_amazon_mt8127-common` |
| [cm14-mt8127/device_amazon_mt8127-common](https://github.com/cm14-mt8127/device_amazon_mt8127-common) | BoardConfig, WMT, mkbootimg args | cloned |
| ford / austin device trees | 600×1024 Fire 7 boards | cloned |

**postmarketOS:**

- Wiki pages exist for **amazon-ford** and **amazon-austin** (downstream kernel ports; some archived).
- XDA: “postmarketOS for Amazon Fire 7 HD 2017 (Austin)” — **armv7, MT8127, Mali-450MP4**, Linux **3.10.54** class.
- Not mainline; proves **“real Linux userspace on MT8127 tablet”** is doable with stock-class kernels + LK boot path.

### 3.3 Alcatel / Telekom Puls “ttab”

| Repo | Notes |
|------|--------|
| [mt8127/android_kernel_alcatel_ttab](https://github.com/mt8127/android_kernel_alcatel_ttab) | **3.10.108**, local clone |

### 3.4 Gemini PDA 3.18 kernel

| Repo | Notes |
|------|--------|
| [lukefor/gemini-linux-kernel-3.18](https://github.com/lukefor/gemini-linux-kernel-3.18) | **3.18.x** (Debian/Sailfish for Gemini). Mentions MT8127 in some MediaTek shared code paths. Useful as **3.18 Mediatek layout** study; **not** our board. |

### 3.5 Other ALPS N0.mp102 siblings

- Head-units / white-label **mid7006al** share `alps-mp-n0.mp102` and kernel 3.18 (4PDA / firmware dump sites).
- Gem-flash listing: `MT8127 digiland DL7006 … alps-mp-n0.mp102-V2_hcn8127.tb.n_P22` — stock scatter packs exist in the wild (P16 vs P22 revisions).

### 3.6 Mainline status

| Source | Finding |
|--------|---------|
| pmOS MT8127 page | Devices exist under **downstream** kernels; **no solid mainline SoC bring-up** for 8127 |
| Nearby mainline | MT65xx / MT6735 / MT6768 communities (newer chips) |
| Implication | For “Linux we built,” plan on **stock 3.18 zImage + custom userspace** first; mainline is multi-month pioneer work |

### 3.7 DigiLand DL7006 specifically

| Source | Finding |
|--------|---------|
| XDA / forums | Root / SP Flash / boot.img only — no full custom ROM |
| Stock scatter | Everest Digiland DL7006-KB MT8127 7.0 packs (Drive often quota-blocked) |
| Our work | Full self-dump + ADB + no-wizard system + DTB + firmware — **beyond public DL7006 docs** |

**No public “Linux boots on DL7006” report found.** Closest working distro experience on same SoC: **Fire 7 pmOS/Lineage (downstream 3.10)**.

---

## 4. Firmware / software stack map

```text
┌─────────────────────────────────────────┐
│  Preloader  preloader_mid7006al.bin     │  boot1 eMMC
│  DRAM/PMIC6323, load LK                 │
├─────────────────────────────────────────┤
│  LK (uboot)  @ 0x1d20000                │  platform/mt8127/*
│  load bootimg, early LCM, optional FB   │  console 921600 ttyMT3
├─────────────────────────────────────────┤
│  boot.img                               │
│   ├─ KERNEL (MTK hdr) → zImage 3.18.35  │  + embedded DTB
│   └─ ROOTFS (MTK hdr) → Android ramdisk │
├─────────────────────────────────────────┤
│  system ext4 (ALPS N0.mp102 / mid7006al)│
│  /system/vendor/firmware (WMT/WiFi)     │
└─────────────────────────────────────────┘
```

### Partition names (fstab / PMT style)

`preloader`, `uboot`/`lk`, `bootimg`, `recovery`, `logo`, `sec_ro`, `seccfg`, `nvram`, `pro_info`, `misc`, `tee1/2`, `android`/`system`, `cache`, `usrdata`/`data`, `expdb`, `frp`.

Recovery block path style: `/dev/block/platform/mtk-msdc.0/by-name/*`.

---

## 5. Hardware bring-up table (for Linux)

| Subsystem | Evidence | Linux notes |
|-----------|----------|-------------|
| CPU/SMP | 4× A7, DTB smp method `mediatek,mt8127-smp` | armv7 multi_v7 |
| Memory | 1 GiB @ `0x80000000` | match DTB; watch reserved regions |
| eMMC | mtk-msdc, full dump OK | block names from fstab |
| Display | 1024×600 DSI/MIPI, `mediatek,lcm` | Hardest; panel IC name still opaque; LK already inits LCM — **may keep framebuffer if kernel/LK handoff works** |
| Touch | **ICN85xx** via mtk-tpd | narnia has full `icn85xx.c` source |
| Wi‑Fi | CONSYS + `WIFI_RAM_CODE_8127` + ROMv2 patches | Blobs in `reference/blobs/firmware/`; WMT `/dev/wmtWifi` |
| BT | blueangel + WMT | tied to same CONSYS |
| FM | MT6627 patches | optional |
| USB | mt_usb / musb | gadget for ADB-like serial if no UART |
| PMIC | MT6323 full DTB regulators | power domains |
| Charger | ETA6003 | |
| GPU | Mali-450 Utgard | blob or software render |
| Accel | MC3410 loaded | optional |
| Camera | SP0A09 / SP2509 | low priority |
| UART | 4× `mt8127-uart`, status okay | **hunt pads**; 921600 |

---

## 6. Strategy for “run Linux we built”

### Recommended order

| Phase | Approach | Why |
|-------|----------|-----|
| **L1** | Reuse **binary 3.18.35 kernel** + **custom initramfs** (busybox/static) as boot.img | Fastest path to “our userspace”; drivers already work |
| **L2** | Study/build **Amazon 3.10.108** or narnia tree for practice | Learn MTK build; not drop-in on our board without LCM/TP retune |
| **L3** | Hunt **3.18 ALPS N0** sources closer to `hcn8127` / mid7006al | Rebuild *this* generation |
| **L4** | Mainline / close-to-mainline | Long-term; needs UART |

### L1 detail (best near-term experiment)

1. Keep stock zImage (`work/boot/kernel` or stripped + DTB append as stock).  
2. Build **armhf static busybox** initramfs (init → shell / getty).  
3. Wrap as MTK `ROOTFS` + Android boot.img (same load addresses as stock).  
4. Flash experimental image: `mtk.py wo 0x1d80000 <len> <file>`.  
5. Observe: bootloop vs black screen vs (if UART) printk.  
6. Restore stock boot anytime from dump.

Success criterion: **init runs** (even if no display — need UART, USB gadget, or side-channel).

Optional L1 improvements:

- USB gadget serial if musb gadget works under stock kernel with custom init.  
- Mount eMMC partitions read-only and inspect.  
- Load Wi‑Fi by starting WMT the Android way (`wmt_loader` patterns from Amazon proprietary-files).

### Kernel source gap

| Tree | Version | Fit for DL7006 |
|------|---------|----------------|
| Running firmware | **3.18.35** | Ground truth |
| narnia GPL CD | **3.4.67** | Driver archaeology; **icn85xx** |
| mt8127-common / ttab / amazon | **3.10.108** | Best public full SoC drivers |
| Gemini | **3.18.x** | Layout study only |
| Ideal | **3.18.x ALPS N0.mp102** | Not found public for mid7006al |

Search keys still open:

- `alps-mp-n0.mp102` kernel source  
- `hcn8127` / `mid7006al` / `mid721l_96e_mipi`  
- GPL source CD requests to DigiLand/HCN (unlikely)

---

## 7. Local caches (what we pulled)

### Blobs — `reference/blobs/`

| Item | Status |
|------|--------|
| Wi‑Fi/BT/FM firmware | **Pulled** (`WIFI_RAM_CODE_8127`, ROMv2, MT6627 FM) |
| wpa / bluetooth conf | **Pulled** |
| vendor etc (audio, thermal, factory) | **Pulled** (~24 MB) |
| trustzone.bin | **Pulled** |
| Embedded DTB + text dump | **Extracted** from zImage |
| Uncompressed vmlinux | `work/boot/vmlinux.bin` (~15 MB) for strings |
| Live probe 2026-07-19 | `reference/probe/live-20260719/` |

### Upstream — `reference/upstream/` (~4.2 GiB)

See [reference/upstream/README.md](../reference/upstream/README.md).

---

## 8. L1 experiment status

**Built:** `experiments/linux-initramfs/out/boot-linux-l1.img` (~6.5 MiB) — **L1.1**

| Item | Detail |
|------|--------|
| Kernel | Stock 3.18.35 (unchanged MTK KERNEL blob) |
| Userspace | busybox 1.31 armv7l static + `/init` |
| USB | **`android_usb` ACM** (`0e8d:2007`) → host `/dev/ttyACM0`, shell on `ttyGS0` |
| Pack | ANDROID! + MTK ROOTFS gzip cpio; static `/dev/*` nodes |
| Cmdline | `androidboot.selinux=permissive` + `console=ttyMT3,921600n1` |
| Flash | `./tools/flash-linux-l1.sh` @ `0x1d80000` |
| Restore | `./tools/flash-linux-l1.sh restore` |

**L1.0 on device:** DIGILAND logo, no ADB, Preloader blip only (expected without gadget).  
**L1.1 goal:** stable `0e8d:2007` + serial shell.

See [experiments/linux-initramfs/README.md](../experiments/linux-initramfs/README.md).

### Still open

1. Flash L1.1 + confirm host `ttyACM0` shell.  
2. UART hunt only if ACM fails.  
3. Next: mount eMMC, larger userspace, Wi‑Fi blobs.

---

## 9. Bottom line

| Question | Answer |
|----------|--------|
| Public Linux for DL7006? | **No** ready image |
| Closest prior art? | **Amazon Fire 7 ford/austin** (pmOS/Lineage, kernel 3.10) + **narnia** (Lineage + GPL, ICN85xx) |
| Our kernel version? | **3.18.35** (confirmed live) |
| ALPS tag? | **alps-mp-n0.mp102** / mid7006al / mid721l_96e_mipi / hcn8127 |
| Mainline MT8127? | **Not practically available** |
| Touch IC? | **Chipone ICN85xx** |
| Wi‑Fi firmware? | **`WIFI_RAM_CODE_8127`** + ROMv2 + WMT (pulled) |
| RAM / display? | **1 GiB** / **1024×600** |
| Best first Linux bet? | **Stock 3.18 zImage + custom initramfs** via existing LK boot path |
| Sources pulled locally? | **Yes** — narnia, amazon, ttab, gemini 3.18, device trees, firmware, DTB |

This tablet is a **serious candidate for self-built Linux userspace** with a long road if full hardware (display/Wi‑Fi from a rebuilt kernel) is required. Research foundation is in-tree under this doc, `reference/blobs/`, and `reference/upstream/`.
