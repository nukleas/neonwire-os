# Handoff — Wi‑Fi / CONSYS bisect (Android works → L1 fails)

**Date:** 2026-07-19  
**Workspace:** this repository  
**Audience:** next agent (Claude or human) continuing Wi‑Fi on DL7006  
**Read first:** this file, then [18-wifi-consys-plan.md](18-wifi-consys-plan.md), [15-consys-power-path.md](15-consys-power-path.md), [16-handoff-linux-consys.md](16-handoff-linux-consys.md)

---

## 0. Paste-this status line

```text
Digiland DL7006-KB (KB SKU; FCC XMF-MID7006 / Lightcomm MID7006): Wi-Fi WORKED on
stock Android (user-locked only; browsed web). Same unit/kernel under L1:
chipId(0x00000000) after perfect host power (CON=0x10d, TOPAXI open). Doc 18's
"silicon dead / beyond software" is WRONG as final conclusion. Next: Android-vs-L1
bisect + fuller WMT userspace. Handoff: docs/19-handoff-wifi-bisect.md
```

---

## 1. Reframe (do not re-litigate power)

| Fact | Evidence |
|------|----------|
| **Hardware is fine** | Owner used stock Digiland Android: Wi‑Fi connected, logins, websites. Not a dead board. |
| **Vendor stack is fine on Android** | Live stock props: `persist.mtk.wcn.combo.chipid=0x8127`, `wlan.driver.status=ok`, `service.wcn.driver.ready=yes` (`reference/probe/live-20260719/getprop-full.txt`) |
| **L1 fails at chipId** | Same stock 3.18.35 kernel + DTB: `Read CONSYS chipId(0x00000000)` |
| **Host power path is NOT the bug** | Instrument (doc 18): `SPM_CONN_PWR_CON=0x10d`, `PWR_STATUS` CONN bit set, TOPAXI CONN protect open, direct read of chipId still 0 |
| **Firmware is NOT the chipId bug** | `WIFI_RAM_CODE_8127` / `ROMv2_*` load *after* chipId succeeds |
| **Naming is correct** | Device `18070000.consys` / `mediatek,mt8127-consys`; control `/dev/wmtWifi`; chipId phys `0x18070008` expect `0x8127` |

**Correct problem statement:**

> Same SoC + same kernel blob: consys comes up under Digiland Android userspace/init, and does not under L1 busybox + hand-started WMT. Find the **runtime delta** (registers and/or userspace sequence), not rewrite MTCMOS from scratch.

Doc 18’s instrument results remain valid. Its **final conclusion** (“silent core / beyond software reach / park Wi‑Fi”) is **superseded** by the working-Android fact.

---

## 2. Device identity (quick)

**SKU: Digiland `DL7006-KB` (the KB version).** Not a random “DL7006” sibling unless labels match. FCC/OEM family is MID7006; retail firmware packs that match this unit are labeled **DL7006-KB** + **MT8127**.

| Item | Value |
|------|--------|
| Retail SKU | Digiland **DL7006-KB** (KB variant — this unit) |
| FCC / OEM model | **XMF-MID7006** / Lightcomm **MID7006** (see [01-device-identity.md](01-device-identity.md)) |
| Also sold as | Everest Digiland **DL7006-KB** (SPFT pack naming) |
| Live product strings | `MID7006AL`, flavor `mid7006al`, project `mid721l_96e_mipi` |
| SoC | MediaTek **MT8127** (integrated CONSYS Wi‑Fi/BT/FM/GPS) |
| Stock | Android 7.0 ALPS `mid7006al` / kernel **3.18.35** |
| Preloader | Unprotected — **mtkclient** is the flash path (fastboot locked) |
| Boot write offset | **`0x1d80000`** |
| Wi‑Fi firmware | `WIFI_RAM_CODE_8127`, `ROMv2_patch_1_{0,1}_hdr.bin`, `WMT_SOC.cfg` (`co_clock_flag=0`) |
| L1 known-good image | `experiments/linux-initramfs/out/boot-linux-l1.img` (or neonos face) |
| Stock boot restore | `./tools/flash-linux-l1.sh restore` → stock `boot.img` from dump |

---

## 3. What is already done (do not redo)

### Ruled out

- [x] Missing DTB / wrong compatible / power-domains absent  
- [x] Userspace “forgot VCN33_WIFI before chipId” (VCN33 is later; chipId needs VCN18/28 + domain + bus clk)  
- [x] Fixed virtual SPM map `0xF000xxxx` shellcode (kernel never uses that map; **do not flash consys-v2**)  
- [x] Host MTCMOS / genpd bookkeeping incomplete (CON=`0x10d` live)  
- [x] TOPAXI AP↔CONN firewall stuck closed  
- [x] Wrong chipId address (instrument used same ctx+0x28 ioremap as driver)  
- [x] Separate BT path (same core; BT also fails with chipId 0)  
- [x] zImage repack breaking patches (`repack_boot_fpad.py` / GNU gzip same-size piggy — solved)

### Proven tools / images

| Asset | Path / note |
|-------|-------------|
| Instrument image (v2) | `experiments/linux-initramfs/out/boot-linux-wifi-instrument.img` — dumps CDBG/CTPX |
| Flash helper | `./tools/flash-wifi.sh {control,instrument,fix,restore,restore-l1,restore-stock}` |
| L1 Wi‑Fi diag script | `experiments/net/wifi-diag.sh` → push to `/mnt/sd/linux-lab/` |
| RNDIS cockpit | `experiments/net/net-up.sh` / `net-down.sh` (cable QoL; not Wi‑Fi) |
| Repack for patches | `experiments/consys-pwr/repack_boot_fpad.py` |
| Stock vmlinux | `experiments/consys-pwr/vmlinux.bin` (kallsyms: `mtk_wcn_consys_power_on` @ `c059c144`, `mtk_wcn_consys_hw_reg_ctrl` @ `c059c234`, link base **`0xc0008000`**) |
| GPL reference (older API) | `reference/upstream/kernel_amazon_mt8127-common/.../mt8127/mtk_wcn_consys_hw.c` |
| Stock firmware pull | `reference/blobs/firmware/` |

### Stock Android bring-up recipe (what works on device)

From ramdisk `init.connectivity.rc` + `init.mt8127.rc`:

1. `wmt_loader` (class core, oneshot)  
2. `wmt_launcher -p /vendor/firmware/` (long-running)  
3. On `wlan.driver.status=ok` → `write /dev/wmtWifi "1"`  
4. `wpa_supplicant -Dnl80211 -iwlan0 ...`

L1 approx recipe: bind-mount `/system`+`/vendor`, copy firmware to `/tmp/fw` + `WMT.cfg`, run loader/launcher, `echo 1 > /dev/wmtWifi`. **Do not** `echo on > .../consys/power/control` while debugging stock WMT (`pm_runtime_get_sync` returns 1 and aborts).

---

## 4. Mission for next agent

**Goal:** get `chipId=0x8127` then `wlan0` under L1 (or document the exact Android-only dependency).

**Success criteria:**

```text
dmesg | grep -i chipId          # 0x8127 (not 0)
ls /sys/class/net                # includes wlan0
# optional: wpa_supplicant + udhcpc → SSH over Wi-Fi
```

**Non-goals:** full custom kernel port; pmOS-style open driver rewrite; more SPM force-on patches (already proven moot).

---

## 5. Bisect plan (do in order)

### Phase A — Android success capture (highest value)

Boot **stock Android** (restore stock boot if currently on L1/NeonOS).

```bash
# Host — when tablet is stock + adb (or serial if available)
./tools/flash-linux-l1.sh restore   # only if currently on L1 boot; needs Preloader
# Power on stock; unlock / skip wizard as already documented (docs/10, 11)
```

On device (adb shell or equivalent), with Wi‑Fi **connected** (or at least after toggle on):

```sh
# Save these under reference/probe/android-wifi-ok/ on the host
getprop | grep -iE 'wlan|wcn|wifi|chipid|wmt'
dmesg | grep -iE 'WMT-CONSYS|WMT-CORE|chipId|EMI|reserve|consys|scpsys|ROMv2|WIFI_RAM' | tee /sdcard/android-wifi-dmesg.txt
ls -l /sys/devices/soc/18070000.consys/driver
cat /sys/devices/soc/18070000.consys/power/runtime_status
cat /sys/kernel/debug/clk/connsys_bus/clk_enable_count 2>/dev/null
# If root + debugfs /dev/mem available on stock (may not be):
# dump same regs as instrument-v3 below
```

Pull artifacts into:

```text
reference/probe/android-wifi-ok/
  getprop-wifi.txt
  dmesg-wifi.txt
  NOTES.md          # SSID connected? IP? time of capture
```

**What to look for in Android dmesg:**

- `reserve_memory_consys_fn` / `consys-reserve-memory` base/size at early boot  
- `Read CONSYS chipId` **absent** or non-zero  
- EMI mapping OK / gConEmiPhyBase  
- Any TOPRGU / OSC / reset lines  
- Exact order: loader → launcher → chipId → patch → `wlan0`

### Phase B — L1 failure capture (clean, comparable)

1. Flash known-good L1: `./tools/flash-wifi.sh restore-l1` or `./tools/flash-linux-l1.sh`  
2. Serial: `./tools/serial-cmd.py 'uname -a'`  
3. Mount SD + system; push `wifi-diag.sh`; run **without** power/control pokes:

```sh
mount -t ext4 -o ro,noload /dev/mmcblk0p6 /mnt/system
mount -t vfat /dev/mmcblk1p1 /mnt/sd
sh /mnt/sd/linux-lab/wifi-diag.sh 2>&1 | tee /mnt/sd/linux-lab/wifi-diag.log
```

4. Also capture **full early dmesg** before ring wraps (or `dmesg -c` only after saving):

```sh
dmesg | grep -iE 'consys|reserve|EMI|WMT-CONSYS|PHY layout|gConEmi|scpsys' | tee /mnt/sd/linux-lab/l1-consys-boot.txt
```

Prior cold capture started at ~0.9s — **early reserved-memory lines may have been lost**. Prefer capturing immediately after boot.

### Phase C — Instrument-v3 (undumped regs)

Doc 18 instrument-v2 proved power + TOPAXI. **Still never dumped:**

| Phys (typical MT8127) | Symbol / meaning |
|----------------------|------------------|
| `0x10007018` | `CONSYS_CPU_SW_RST` (AP_RGU+0x18), bit12 + key `0x88<<24` |
| `0x10001800` | `AP2CONN_OSC_EN` (+ wakeup bit17) — 26M path to CONN |
| `0x10000018` | `CONSYS_WD_SYS_RST` area (TOPCKGEN+0x18), bit9 |
| `0x10001310` | `CONSYS_EMI_MAPPING` |
| `0x18070000` + `0,4,8,0x110,0x114,0x160` | CONN_MCU block (id, ACR, CPUPCR) |

Implement as a **same-size** patch via existing `patch_consys_instrument*.py` + `repack_boot_fpad.py` pattern. Log one line e.g.:

```text
CV3 rst=<rgu> osc=<osc> emi=<map> id0=<r0> id8=<chipId> acr=<110> pcr=<160>
```

Flash: `./tools/flash-wifi.sh` — add a new mode or reuse instrument after rebuild.  
**Do not** reintroduce fixed `0xF000xxxx` SPM writes.

Compare CV3 dump **Android (if possible)** vs **L1** → the delta is the fix target.

### Phase D — Userspace parity experiments (if regs match)

If L1 regs already look like Android after power-on but chipId still 0, or if Android shows extra early init:

1. Run launcher from real path: `wmt_launcher -p /vendor/firmware/` (bind mounts), not only `/tmp/fw`  
2. Longer wait after launcher before `echo 1 > /dev/wmtWifi`  
3. Minimal property stubs if binaries require them (`service.wcn.driver.ready`, etc.)  
4. One-shot test: `co_clock_flag=1` in `WMT.cfg` (board ships `0`; contrast only)  
5. Empty boot.img cmdline like stock (L1 currently sets console/root=ram) — low probability, free test  

### Phase E — Fallback comfort (not the goal)

- RNDIS: `net-up.sh` on L1 → `telnet 192.168.42.1`  
- USB Wi‑Fi: needs kernel with `CONFIG_MODULES=y` or built-in driver — no Digiland 3.18 sources; far path  

---

## 6. Register / code anchors (for patches)

Stock disassembly (vmlinux linked at **`0xc0008000`**):

| Symbol | VA |
|--------|-----|
| `mtk_wcn_consys_power_on` | `c059c144` |
| `mtk_wcn_consys_hw_reg_ctrl` | `c059c234` |
| Global consys ctx | `c0f59f30` (ctx+0x28 = CONN_MCU ioremap; +0x4 → dev) |
| `regulator_enable` | `c03fb840` |
| `scpsys_power_on` | `c03f7ba4` |

`reg_ctrl(on=1)` for **`co_clock_en=0`** (our `WMT_SOC.cfg`): VCN18 → PMIC 0x41c HW-mode VCN28 → VCN28 enable → `power_on` → bus clk → poll chipId in `{0x8127,0x8163,0x335,0x321,0x337}` with retries + `msleep(20)` style delays.

GPL (Amazon 3.10, older API names): assert CPU SW reset → `conn_power_on` → delay 10µs → enable CONNMCU clk → poll chipId → (later FW) deassert reset / AFE / MBIST. Some steps `#if 0`’d into FW patch for ALPS bugs — stock 3.18 may differ; **compare live Android**.

---

## 7. Constraints / landmines

| Constraint | Action |
|------------|--------|
| `CONFIG_MODULES=n` | No `insmod`; binary patch or full kernel rebuild only |
| No Digiland ALPS 3.18 sources | Cannot clean rebuild matching modules |
| Fastboot locked | Preloader/mtkclient only for flash |
| `/dev/mem` off on L1 | Need kernel patch or stock Android debug path for raw regs |
| Piggy size | Patches must compress ≤ original piggy (`gzip -9 -n` + pad); else zopfli / shrink shellcode |
| Serial ACM | One opener; WMT spam can briefly drop USB — re-enum or power cycle |
| Host must not mount tablet eMMC nodes | All mounts on-device over serial |

**Do not:**

- Flash `boot-linux-l1.4-consys-v2.img` / fixed-addr SPM shellcode  
- Leave `18070000.consys/power/control=on` while using stock WMT  
- Conclude “hardware dead” without Android Wi‑Fi regression proof  

---

## 8. Suggested work split (if multi-agent)

| Agent A | Agent B |
|---------|---------|
| Android restore + probe → `reference/probe/android-wifi-ok/` | Instrument-v3 patch + repack offline |
| Diff Android vs L1 dmesg | L1 clean wifi-diag after A’s artifacts land |
| Flash only with user present | Doc/checklist updates |

Single-writer: one `/dev/ttyACM0`, one mtkclient session, one repack of `boot-linux-*.img`.

---

## 9. Repo map (Wi‑Fi signal)

```text
docs/
  15-consys-power-path.md     early diagnosis (partially outdated on genpd)
  16-handoff-linux-consys.md  L1 ops + old Wi-Fi state
  18-wifi-consys-plan.md      instrument results; FINAL "parked" conclusion SUPERSEDED
  19-handoff-wifi-bisect.md   ★ this file
experiments/consys-pwr/       patches, vmlinux, repack_boot_fpad.py
experiments/net/              wifi-diag.sh, net-up.sh
experiments/linux-initramfs/out/
  boot-linux-l1.img
  boot-linux-wifi-instrument.img
  boot-linux-wifi-control.img
tools/flash-wifi.sh
reference/blobs/firmware/
reference/probe/live-20260719/   stock Android props (chipid 0x8127, wlan ok)
reference/probe/live-coldboot/   L1 cold probes
```

---

## 10. Definition of done for this handoff

Next agent has succeeded when **either**:

1. L1 shows `chipId=0x8127` and `wlan0`, with steps documented; or  
2. A side-by-side Android vs L1 reg/dmesg diff names the **exact** missing step (e.g. OSC bit, RGU reset, EMI base, launcher env), with a concrete patch or userspace fix proposed.

Until then, prefer bisect over more SPM force-on experiments.

---

## 11. Digiland / OEM external sources (2026-07-19 research)

**SKU note:** this lab unit is **`DL7006-KB`** (KB). When hunting packs/manuals, prefer **DL7006-KB** + **MT8127** + Android 7.0. Generic “DL7006” without KB may be a different board/revision — do not flash blindly.

**Expectation management:** Digiland is a retail brand; the **OEM is Lightcomm Technology Co., Ltd.** (FCC grantee **XMF**). They do **not** publish kernel trees, Wi‑Fi driver sources, or board schematics. Nothing found that unblocks CONSYS chipId on L1 better than our on-device dump + GPL MT8127 trees.

### Official / semi-official

| Source | What you get | Wi‑Fi value |
|--------|----------------|-------------|
| Old brand site `digi-land.net` | Often dead/unreachable; Support/Download was generic tablet PC marketing | Near zero |
| Best Buy Q&A (DL7006-KB) | Manuals by email: **service.digi-land.net** (historical tip) | User guide only |
| Amazon Digiland store | Current tablets only (newer SoCs) | Irrelevant |

### FCC (best public hardware docs) — **XMF-MID7006**

- Hub: https://fccid.io/XMF-MID7006  
- Mirror: https://fcc.report/FCC-ID/XMF-MID7006  
- Certified **2017-06-23**, model **MID7006**, applicant Lightcomm (HK). Contact on filing: `sunshuhui@hcn2000.com`.

**Public PDFs (download from FCC exhibits):**

| Exhibit | Use |
|---------|-----|
| **User Manual** | Consumer setup (not engineering) |
| **Internal / external photos** | Antenna, shield cans, board silkscreen — RF layout hints |
| **Test Report WLAN / BT LE / NII** | Confirms integrated **802.11 a/b/g/n** (+ 5 GHz bands on grant), BT classic + BLE — RF is real silicon |
| **SAR / RF exposure** | Antenna positions / body SAR |
| **ID label** | FCC ID placement |

**Long-term confidential (not public):** block diagram, schematics, operational description, software security / tune-up. Do not expect pin-level CONSYS power nets from Digiland’s site.

**Note vs props:** FCC grant lists 5 GHz WLAN; stock prop `ro.wlan.mtk.wifi.5g=0` may mean software-disabled 5 GHz on this SKU — not a contradiction with “Wi‑Fi worked.”

### Third-party “stock firmware” (SPFT packs)

| Listing | Package name | Notes |
|---------|--------------|--------|
| https://firmwarefile.com/everest-digiland-dl7006-kb | **`Everest_Digiland_DL7006-KB_MT8127_20170605_7.0.zip`** (~908 MB) | **KB-labeled** pack (matches our SKU); SPFT + USB driver claims; Google Drive mirror on that page |
| firmwaredrive.com Digiland folder | Same ecosystem | Often paid/gated mirrors |

**Value for us:** optional **scatter + factory images** to compare against our mtkclient dump (`reference/dumps/session-20260718/`). **Not** kernel source. Prefer our dump as ground truth; treat random firmware sites as malware-risk — scan before use. Only use packs clearly marked **DL7006-KB** + **MT8127**. We already have working vendor firmware blobs under `reference/blobs/firmware/`. See also [03-flashing.md](03-flashing.md) / [07-option-c-stock-reflash.md](07-option-c-stock-reflash.md).

### USB drivers

Windows “Digiland DL7006 USB driver” pages = generic **MediaTek preloader/VCOM** installers (same as mtkclient needs). No Linux Wi‑Fi driver package.

### What is *not* available from Digiland

- No ALPS/`mid7006al` / `MT8127_N0.MP102` kernel tree  
- No out-of-tree `wlan` / WMT sources beyond what is already in upstream Amazon/narnia trees  
- No public schematics for VCN / crystal / CONSYS routing  

**Still the highest-signal path:** Phase A Android dmesg vs L1 + instrument-v3 (this doc §5), not OEM support tickets.
