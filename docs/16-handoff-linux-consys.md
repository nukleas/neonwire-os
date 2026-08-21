# Handoff — DL7006 live Linux + CONSYS / Wi‑Fi power

**Date:** 2026-07-19  
**Workspace:** this repository  
**Audience:** next human or agent continuing the L1 Linux lab  
**Read with:** [14-live-linux-lab.md](14-live-linux-lab.md), [15-consys-power-path.md](15-consys-power-path.md), [checklist.md](checklist.md)

---

## 1. Mission (current)

Primary goal shifted from “HA client on Android” to **run self-built Linux on the tablet** and bring up hardware, especially **Wi‑Fi**.

| Priority | Status |
|----------|--------|
| Stock kernel + busybox userspace (L1) | **Working** |
| USB serial root shell | **Working** |
| eMMC / SD / framebuffer | **Working** |
| WMT userspace stack (loader/launcher) | **Half-up** |
| CONSYS power → chipId / `wlan0` | **Blocked** |
| Loadable kernel modules | **Impossible on stock** (`CONFIG_MODULES=n`) |
| Custom kernel rebuild for DigiLand | **No board sources** |

---

## 2. Device snapshot

| Item | Value |
|------|--------|
| Product | DigiLand **DL7006-KB** (FCC **XMF-MID7006**) |
| SoC | MediaTek **MT8127** (4×A7, Mali-450) |
| Stock Android | 7.0 ALPS **`alps-mp-n0.mp102`** / flavor **`mid7006al`** / project **`mid721l_96e_mipi`** |
| Kernel | **3.18.35** `#2 SMP PREEMPT` `liushen@midcompser` (gcc 4.8) |
| Preloader | **Unprotected** — mtkclient works |
| Boot flash offset | **`0x1d80000`** |
| RAM | ~1 GiB |
| eMMC UA | ~7.1 GiB (`mmcblk0`) |
| SD | ~116 GiB (`mmcblk1p1`, vfat) |
| LCM | **ZS070BE3019B3H7II_713** (1024×600 class; fb virtual taller) |
| PMIC | MT6323 (VCN LDOs for connectivity) |
| Wi‑Fi path | Integrated CONSYS + firmware **`WIFI_RAM_CODE_8127`** |

### Partitions (user area)

| Node | Size (approx) | Role |
|------|----------------|------|
| `mmcblk0p6` | 1.5 GiB | **system** (ext4) |
| `mmcblk0p7` | 256 MiB | cache |
| `mmcblk0p8` | ~5.2 GiB | data |
| `mmcblk1p1` | ~116 GiB | SD lab workspace |

No `by-name` without Android `ueventd` — use `pN` numbers.

---

## 3. What is on the device right now (as of handoff)

**Restored known-good L1** after a failed L1.4 experiment. Confirmed live:

```text
USB:  0e8d:2007 MediaTek L1-Linux-ACM → host /dev/ttyACM0
Shell: dl7006#  root, busybox
Kernel: 3.18.35 stock (unpatched)
```

Mount recipe **on tablet shell** (not host):

```sh
mkdir -p /mnt/system /mnt/sd /mnt/data /mnt/cache
mount -t ext4 -o ro,noload /dev/mmcblk0p6 /mnt/system
mount -t vfat /dev/mmcblk1p1 /mnt/sd
# optional:
# mount -t ext4 -o ro,noload /dev/mmcblk0p7 /mnt/cache
# mount -t ext4 /dev/mmcblk0p8 /mnt/data
```

SD helpers already present:

```text
/mnt/sd/linux-lab/wifi-bringup.sh
/mnt/sd/linux-lab/wifi-bringup-l14.sh
/mnt/sd/linux-lab/dmesg-cold.txt
```

**Host mistake to avoid:** do **not** `mount /dev/mmcblk0p6` on the PC. That node is the tablet’s eMMC when the tablet is running Linux; the host does not see it as a usable filesystem for this workflow. All mounts and Wi‑Fi scripts run **over serial on the tablet**.

---

## 4. Day-to-day ops (agents)

### Serial console (primary)

```bash
# Host — only one opener at a time (close picocom/screen first)
./tools/serial-cmd.py 'uname -a'
./tools/serial-cmd.py -w 3 'dmesg | tail -30'
# Interactive-ish: picocom -b 115200 /dev/ttyACM0
```

### Flash boot (Preloader)

```bash
# Known-good L1 (busybox + stock kernel)
./tools/flash-linux-l1.sh          # interactive Enter + sudo
./tools/flash-linux-l1-now.sh      # waits for Preloader (used when looping)

# Stock Android boot restore
./tools/flash-linux-l1.sh restore

# FAILED experiment (do not re-flash until fixed):
# experiments/linux-initramfs/out/boot-linux-l1.4-consys.img
# tools/flash-linux-l1.4-consys.sh
```

**Flash procedure:** tablet fully off → unplug → start script → plug USB **no buttons** → write OK → unplug → power on.

**Bootloop signature:** USB shows `0e8d:2000 MT65xx Preloader` every ~15 s, no `L1-Linux-ACM`. Catch Preloader with `flash-linux-l1-now.sh` or full power-off cycle.

### Lab policy

- Stay on DigiLand/L1 for daily work when possible.  
- Power off only for Preloader flashes.  
- Prefer SD (`/mnt/sd/linux-lab`) for large artifacts over overwriting Android data.

---

## 5. Progress timeline (compressed)

1. Full flash dump, boot/recovery/LK mapped, ADB unlock / wizard skip paths documented.  
2. **L1** built: stock zImage + busybox initramfs + MTK boot framing.  
3. **L1.1–L1.3:** USB ACM gadget → interactive root shell; drop shell-poisoning heartbeat.  
4. Mounts, SD workspace, mtkfb framebuffer proven.  
5. Android **WMT** path: `wmt_loader` / `wmt_launcher` / `stpwmt` / `wmtWifi` can be started under bionic bind-mount; **fails at chip power**.  
6. Diagnosed CONSYS: chipId `0`, `connsys_bus` off, VCN use_count 0, genpd empty, RPM `unsupported`.  
7. Confirmed **`CONFIG_MODULES=n`** (`insmod` → ENOSYS; no `/proc/modules`).  
8. Disassembled stock `mtk_wcn_consys_hw_reg_ctrl` / `mtk_wcn_consys_power_on` via kallsyms + decompressed piggy.  
9. Built OOT `consys_pwr.ko` against Amazon 3.10 tree (not loadable on device).  
10. **L1.4** binary-patched stock kernel SPM power-on → **bootloop** → restored L1.

---

## 6. Wi‑Fi / CONSYS technical state

### Goal

```text
echo 1 > /dev/wmtWifi   →  wlan0 exists
chipId ideally 0x8127
```

### Failure signature (stock path)

```text
[WMT-CONSYS-HW] Read CONSYS chipId(0x00000000)
[WMT-CORE] wmt_core_stp_init fail
[MTK-WIFI] WIFI_write: WMT turn on WIFI fail!
connsys_bus enable_count=0 rate=0
vcn18/vcn28/vcn33_wifi: open_count=0 use_count=0
```

### Power order (electrical / GPL Amazon)

1. VCN 1V8 (`vcn18`)  
2. VCN28 if not co-clock (`WMT_SOC.cfg` has **`co_clock_flag=0`**)  
3. MTCMOS CONN (`SPM_CONN_PWR_CON` phys **`0x10006280`**, virt **`0xF0006280`**)  
4. Bus clock `connsys_bus`  
5. Poll chip ID @ CONN_MCU + 8  

### Stock code path (disassembly, live kallsyms)

| Symbol | Addr (stock) | Role |
|--------|----------------|------|
| `mtk_wcn_consys_hw_reg_ctrl` | `c059c234` | regulators + power_on + clk + chipId |
| `mtk_wcn_consys_power_on` | `c059c144` | `__pm_runtime_resume` / RPM_GET_PUT |
| `regulator_enable` | `c03fb840` | VCN rails |
| `scpsys_power_on` | `c03f7ba4` | genpd (not driving CONSYS usefully) |

`power_on` treats **any non-zero** `pm_runtime` return as failure (including `1` = already active). Live `18070000.consys` has **`runtime_status=unsupported`** (`disable_depth` path) and **no bound platform driver** at cold boot until WMT stack is exercised. genpd debugfs empty.

### Userspace limits

| Interface | Result |
|-----------|--------|
| `/dev/mem` | not usable (mknod exists or not; open fails) |
| `/proc/kcore` | missing |
| VCN sysfs `enable` | missing (consumer-only) |
| regmap pwrap | effectively read-only / useless for poke |
| `insmod` | **ENOSYS** — modules disabled in kernel |

### WMT userspace recipe (when shell is L1)

Android binaries need `/system` bind + bionic:

```sh
mount --bind /mnt/system /system
export LD_LIBRARY_PATH=/system/lib:/system/vendor/lib
export PATH=/system/bin:/system/xbin:/system/vendor/bin:$PATH
# firmware → /tmp/fw/ including WMT.cfg copy of WMT_SOC.cfg
/system/vendor/bin/wmt_loader
# mknod stpwmt / wmtWifi from /sys/class/*/dev
/system/vendor/bin/wmt_launcher -p /tmp/fw/ &
echo 1 > /dev/wmtWifi
```

See SD scripts under `/mnt/sd/linux-lab/`. Heavy WMT spam can briefly drop USB ACM; shell usually returns after re-enum.

---

## 7. L1.4 failure (do not re-flash as-is)

| Item | Path |
|------|------|
| Image | `experiments/linux-initramfs/out/boot-linux-l1.4-consys.img` |
| Flash helper | `tools/flash-linux-l1.4-consys.sh` |
| Intent | Binary-patch `mtk_wcn_consys_power_on` to raw SPM MTCMOS + TOPAXI clear |
| Patch tools | `experiments/consys-pwr/patch_stock_power_on.py`, `repack_patched_boot.py` |
| Result | **Preloader bootloop** — kernel never reached L1 ACM |
| Recovery | Flash `experiments/linux-initramfs/out/boot-linux-l1.img` |

### Likely causes to investigate

1. **zImage piggy recompression** — stock decompressor may not accept our gzip packaging (mtime/flags/layout).  
2. **zImage header `end` field** (`0x2c`) or alignment after larger piggy.  
3. **DTB splice** position relative to piggy.  
4. Less likely if never ACM: patch shellcode itself (would still usually get past early boot).  
5. **Fixed virt map assumption** `0xF0006xxx` is correct for this tree’s I/O map (matches GPL); still unused if kernel never boots.

### Safer re-test ideas (ordered)

1. **Repack control:** same pipeline with **unpatched** `vmlinux.bin` → if bootloops, repack is broken.  
2. If control boots: apply patch only; verify with `kallsyms` / memory that `c059c144` bytes match shellcode.  
3. Prefer **minimal instruction patch** (e.g. only `if (ret)` → `if (ret < 0)`) before full SPM shellcode.  
4. Avoid full Amazon 3.10 boot on this board without a long porting effort.

---

## 8. Repo map (high signal)

```text
docs/
  14-live-linux-lab.md       L1 shell, mounts, fb
  15-consys-power-path.md    Wi-Fi power diagnosis
  16-handoff-linux-consys.md this file
experiments/linux-initramfs/
  out/boot-linux-l1.img      ★ known-good flash target
  out/boot-linux-l1.4-consys.img  FAILED — keep for forensics
  rootfs/ init, busybox helpers
  pack_linux_boot.py
experiments/consys-pwr/
  consys_pwr.c / .ko         OOT module (Amazon 3.10; not for stock)
  patch_stock_power_on.py    ARM shellcode generator
  repack_patched_boot.py     zImage + boot.img rebuild
  vmlinux.bin                decompressed stock piggy
  stock-appended.dtb         FDT from stock zImage
  kbuild-amazon/             modules_prepare tree (3.10 + DEVMEM)
tools/
  serial-cmd.py              tablet shell over ACM
  flash-linux-l1.sh / -now.sh
  flash-linux-l1.4-consys.sh
  mtkclient/ + venv
reference/dumps/session-20260718/   full flash archive
reference/upstream/                 Amazon/narnia/gemini kernel trees (~4.2 GiB)
reference/blobs/firmware/           WIFI_RAM_CODE_8127, ROMv2 patches, …
```

### Host toolchain (already downloaded)

```text
~/toolchains/armv7l-linux-musleabihf-cross/bin/armv7l-linux-musleabihf-
```

Amazon kbuild: `experiments/consys-pwr/kbuild-amazon` with `CONFIG_MODULES=y` `CONFIG_DEVMEM=y`. Module build needs `-march=armv7-a` (see Makefile).

---

## 9. Constraints / non-goals for next agent

| Constraint | Implication |
|------------|-------------|
| No DigiLand ALPS 3.18 sources | Cannot cleanly rebuild matching kernel + modules |
| Amazon GPL is 3.10 | Modules/kernels from it ≠ stock vermagic; board DT differs |
| Gemini 3.18 is mt6797-class | Not a drop-in for MT8127 tablet |
| Stock `CONFIG_MODULES=n` | OOT `.ko` cannot load until different kernel boots |
| Preloader unprotected | Recovery always possible if boot.img flashes keep working |
| sudo for mtkclient | Agent often needs user-run flash scripts |

**Do not:** force-flash L1.4 again without a no-op repack control test.  
**Do not:** leave `18070000.consys` `power/control=on` while debugging stock WMT (makes `get_sync()` return 1 and aborts).  
**Do not:** assume host can mount tablet partitions.

---

## 10. Suggested next workstreams

### A — Unblock Wi‑Fi (highest value)

1. Fix zImage repack (control image unpatched).  
2. Minimal binary patch or corrected SPM injection.  
3. After boot: WMT bring-up; confirm chipId ≠ 0; `wlan0`.  
4. Document registers before/after (`SPM_CONN_PWR_CON`, VCN counts, `connsys_bus`).

### B — Lab quality of life

1. Optional: rebuild L1 with `CONFIG_DEVMEM` **requires kernel rebuild** — same source problem.  
2. Alpine rootfs on SD once networking works.  
3. Keep serial helpers robust if WMT floods ACM.

### C — Long-term kernel ownership

1. Hunt `MT8127_N0.MP102` / `mid7006al` kernel tree (path string in vmlinux: `.../BBY/MT8127_N0.MP102_V2/kernel-3.18/`).  
2. Or port DigiLand DTB + drivers onto closest GPL tree (multi-day).

---

## 11. Quick verification checklist (new session)

```bash
# Host
lsusb | grep 0e8d          # expect 2007 L1-Linux-ACM when up
ls -l /dev/ttyACM0
./tools/serial-cmd.py 'uname -a; echo ok'

# On tablet via serial
mount -t ext4 -o ro,noload /dev/mmcblk0p6 /mnt/system
mount -t vfat /dev/mmcblk1p1 /mnt/sd
cat /sys/devices/soc/18070000.consys/power/runtime_status
cat /sys/kernel/debug/clk/connsys_bus/clk_enable_count
# cold: unsupported / 0 until WMT
```

Success criteria for Wi‑Fi (from doc 15):

```text
dmesg | grep chipId     # non-zero, ideally 0x8127
cat .../connsys_bus/clk_enable_count   # >0
ls /sys/class/net       # includes wlan0
```

---

## 12. Contact surface for dual-agent work

| Agent A (example) | Agent B (example) |
|-------------------|-------------------|
| zImage repack forensics + control boot | Live WMT recipe polish + dmesg capture on good L1 |
| ARM shellcode size/safety | Serial tooling / initramfs helpers |
| Flash only with user present | Doc updates + checklist |

**Single-writer rules:** one process on `/dev/ttyACM0`; one mtkclient session; don’t both repack `boot-linux-l1.img` without coordinating.

**Handoff status line to paste:**

```text
DL7006 L1 OK (stock 3.18.35 + ACM shell). Wi-Fi blocked at CONSYS power
(chipId 0, MODULES=n). L1.4 SPM patch bootlooped; restored L1.img. Next:
fix zImage repack with unpatched control, then retry power patch.
Handoff: docs/16-handoff-linux-consys.md
```
