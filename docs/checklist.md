# Lab checklist — DL7006 RE kit

Charter: [00-charter.md](00-charter.md)  
Archive: `reference/dumps/session-20260718/`

## Phase A — Own flash ✅ (complete enough)

See [08-phase-a-own-flash.md](08-phase-a-own-flash.md).

- [x] mtkclient + venv
- [x] Preloader attach (sudo)
- [x] Unprotected MT8127 confirmed
- [x] Full user dump `raw/flash-user.bin` (7.125 GiB)
- [x] boot1 preloader + boot2 empty
- [x] system (`4.bin` / `images/system.img`)
- [x] boot.img + recovery.img extracted @ `0x1d80000` / `0x2d80000`
- [x] SHA256SUMS
- [x] Session layout organized
- [ ] Optional: restore-proof write-back of stock boot (Phase B)

## Phase B — Own boot (active)

See [09-phase-b-own-boot.md](09-phase-b-own-boot.md).

- [x] Organize `images/` + `work/`
- [x] Unpack boot + recovery (`tools/unpack_bootimg.py`)
- [x] Strip MTK KERNEL/ROOTFS headers
- [x] Extract ramdisk (gzip + cpio)
- [x] Kernels identical (boot == recovery)
- [x] `fstab.mt8127` / `default.prop` notes
- [ ] Kernel version string (zImage still opaque / no easy banner)
- [x] **LK found** @ `0x1d20000` / `images/lk.bin` (also logo @ `0x4400000`)
- [x] **boot-adb.img** built (insecure ADB props; stock kernel)
- [x] Flash `boot-adb.img` via `wo 0x1d80000` (write OK 0x743800 bytes)
- [x] Verify `adb devices` + getprop after boot
- [ ] Restore stock boot proven if needed

## Phase C — Map hardware

- [x] Partition map from fstab + PMT + recovery.fstab
- [x] Touch **ICN85xx**, Wi‑Fi **WIFI_RAM_CODE_8127**, PMIC **MT6323**, charger **ETA6003**
- [x] Embedded DTB extracted from zImage (`reference/blobs/devicetree/`)
- [x] Display **1024×600**, RAM **1 GiB**
- [ ] UART pad hunt on PCB (console designed for 921600 ttyMT3)
- [ ] LCM panel IC string (still opaque; generic `mediatek,lcm` in DT)

## Phase D — Linux / custom OS (research solid)

See [13-linux-port-research.md](13-linux-port-research.md).

- [x] Live kernel version: **3.18.35** (liushen@midcompser)
- [x] ALPS tag: `alps-mp-n0.mp102` / project `mid721l_96e_mipi` / flavor `mid7006al`
- [x] Clone narnia + mt8127-common + **Amazon ford/austin** + **Gemini 3.18** → `reference/upstream/` (~4.2 GiB)
- [x] Pull vendor firmware blobs → `reference/blobs/firmware/`
- [x] Document pmOS/Lineage prior art on Fire 7 MT8127
- [x] Busybox initramfs + stock zImage image built (`experiments/linux-initramfs/out/boot-linux-l1.img`)
- [x] L1 flash observed: DigiLand logo, no ADB, Preloader blip only (kernel path likely)
- [x] **L1.1** USB ACM serial — **CONFIRMED on device** `0e8d:2007 L1-Linux-ACM` + `/dev/ttyACM0` stable
- [x] **L1.2 SUCCESS:** interactive root shell on `/dev/ttyACM0` — `uname` 3.18.35, uid=0
- [x] LCM name from cmdline: **ZS070BE3019B3H7II_713** (fps≈58)
- [x] **L1.3 built:** no heartbeat-on-shell (fixes wedge); helpers `make-block-nodes` / `mount-android`; host `tools/serial-cmd.py`
- [x] Flash L1.3 — live shell stable
- [x] Mount eMMC system/cache/data + SD; RW workspace on SD & data
- [x] Framebuffer `/dev/fb0` writable (mtkfb); zero-fill OK
- [x] Wi‑Fi stack half-up: `wmt_loader` + `stpwmt`/`wmtWifi` nodes + `wmt_launcher`/`mtk_wmtd`
- [ ] Wi‑Fi L1: chipId=0 was a RED HERRING (prints on working Android too). Driver stack comes up on L1 (chip **detects 0x8127**, `mtk_stp_wmt`+`mtk_wmtd` live, `SET_STP_MODE`=ok via native `wmtctl_min`). **Real blocker: connsys SDIO func never enumerates** (`hif_sdio_stp_on: no supported func probed` → `invalid Handle of WmtStp`). NOT silicon/power/firmware. Resume: docs/21 "▶ Resume here", `experiments/net/wifi-up.sh`
- [x] **Handoff doc** for multi-agent: [16-handoff-linux-consys.md](16-handoff-linux-consys.md)
- [x] Confirmed stock **`CONFIG_MODULES=n`** (insmod ENOSYS)
- [x] OOT `consys_pwr` source + Amazon 3.10 build attempt (`experiments/consys-pwr/`)
- [x] L1.4 binary SPM patch **bootlooped** → restored `boot-linux-l1.img` (do not reflash L1.4 yet)
- [ ] Alpine/armv7 rootfs on SD / larger userspace

## Phase E — Custom OS face (NEONWIRE fbui)

See [17-cyberpunk-fbui.md](17-cyberpunk-fbui.md). Aesthetic from `in-repo palette`.

- [x] Bake JetBrains Mono → AA bitmap font (`genfont.py` → `font_neon.h`)
- [x] `neofb.c` — static ARMv7 framebuffer HUD (ioctls, glyph blend, glow, panels, scanlines)
- [x] Serial deploy pipeline: `push.py` (gzip+base64+sha256) → SD; `pull_shot.py` (screenshot → PNG)
- [x] **Live on device:** neon dashboard painted on mtkfb — kernel/cpu/uptime/load/mem + subsystem status
- [x] Retuned to cyberdesign tokens (palette, glow, CRT scanlines, pixel-grid, corner brackets)
- [x] Shared engine `fbgfx.h` (palette, glyph/glow, panels, bar, scanlines, fb lifecycle)
- [x] **`neui` touch launcher** — nav rail + 5 panels (SYSTEM/PROCESS/STORAGE/KERNLOG/NETWORK)
- [x] Panels read real data: `/proc` stat, `statvfs`, `klogctl` ring buffer, `/proc/net/dev`
- [x] Touch on `mtk-tpd` `/dev/input/event4` (X[0..1024] Y[0..600], 1:1 map, tap-to-switch)
- [x] **Action bar:** SYNC / CLR LOG / REMOUNT / REBOOT(confirm-guarded) + toast; verified via `--tap` (dmesg 1759→2 on clear)
- [x] **Output tool tiles:** DF/MEM/MOUNTS/DMESG run commands (`popen`) into a scrollable neon overlay (tap-scroll, `[X]`/outside to close); SYNC/REBOOT kept as do-actions
- [x] Boot splash + autostart **CONFIRMED ON DEVICE**: flashed `out/boot-linux-l1-neonos.img` @ 0x1d80000; boots straight into the NeonOS UI (kernel unchanged, serial-shell recovery preserved). Flash: `./tools/flash-neonos.sh`
- [x] **DIGILAND LK logo replaced** (built + verified): decoded MTK logo.bin (39 zlib'd 1024x600 32bpp BGRA blobs), regenerated NEONWIRE splash into slots 0 & 38, kept battery frames byte-identical, fits partition (432<452 KB). `experiments/fbui/make_logo.py` → `out/logo-neonos.bin`. Flash: `./tools/flash-neonos.sh logo` (**pending on-device flash**)
- [ ] Output-capturing action tiles (shell cmd → overlay); scroll/gestures for KERNLOG/PROCESS
- [ ] Network panel goes live when Wi-Fi unblocks

## Lab log

| Date | Note |
|------|------|
| 2026-07-18 | Identity; Preloader; mtkclient; handshake issues until sudo |
| 2026-07-18 | Dumped mbr 0–4; system; prefix; boot1 preloader |
| 2026-07-18 | Soft-unresponsive then reset OK |
| 2026-07-19 | boot2 empty; **full flash-user.bin**; boot+recovery found in image |
| 2026-07-19 | **Reorganized** → `session-20260718/`; Phase B unpack started |
| | |
| 2026-07-19 | **LK found** @ `0x1d20000` size `0x60000` (payload 257KiB). logo @ `0x4400000`. Extracted `images/lk.bin`. Boot chain: preloader→LK→boot.img. |
| 2026-07-19 | Built **boot-adb.img** (insecure ADB props, stock kernel). Flash: `./tools/flash-boot-adb.sh` |
| 2026-07-19 | L1.3 live lab: ACM shell, mounts, fb, WMT half-up; CONSYS chipId=0. MODULES=n. |
| 2026-07-19 | L1.4 consys SPM binary patch bootlooped; restored L1. Handoff: docs/16-handoff-linux-consys.md |
| 2026-07-19 | **Flashed boot-adb** to `0x1d80000` len `0x743800` — write completed successfully. |
| 2026-07-19 | **Bootloop** confirmed (Preloader every ~15s). Cause: repack stripped execute bits on `init`. Rebuilt fixed image `085f8320…`. Restore stock or flash fixed. |
| 2026-07-19 | ADB **unauthorized**, no Allow dialog. Built system-no-wizard (SetupWizard/Provision removed, mode=DISABLED) + boot with embedded adb_keys. |
| 2026-07-19 | **Bootloop** after boot-adb flash: Preloader every ~15s, no Android. Restore stock boot required. Likely ramdisk repack issue. |
| 2026-07-19 | system-no-wizard **write OK**. `e data` failed 0xa5; boot write USB overflow. Need separate session for boot-adb flash. |
| 2026-07-19 | **Booted without wizard** (system-no-wizard). ADB still unauthorized — boot-adb flash may have been skipped after USB overflow. |
| 2026-07-19 | **ADB device** achieved. debuggable=1 adb.secure=0 setupwizard=DISABLED. shell uid=2000. Path 1 unlocked. |
| 2026-07-19 | Installed HA Companion **2026.7.3-full** via adb. WiFi OK; `homeassistant.local` does not resolve on tablet — use LAN IP. |
| 2026-07-19 | Linux research: kernel **3.18.35**, ALPS n0.mp102; narnia/mt8127-common upstream. |
| 2026-07-19 | **Deep research pull:** vendor firmware (WIFI_RAM_CODE_8127, MT6627 FM), DTB from zImage, Amazon Fire + Gemini clones (~4.2 GiB upstream). Touch=ICN85xx, RAM=1 GiB, panel 1024×600, UART console 921600. |
| 2026-07-19 | **L1 boot image built:** `experiments/linux-initramfs/out/boot-linux-l1.img` — stock 3.18.35 + busybox. Flash @ `0x1d80000`. |
| 2026-07-19 | L1 on device: DIGILAND logo stick, no ADB, Preloader `0e8d:2000` ~2s only — no USB gadget yet. |
| 2026-07-19 | **L1.1 built:** enable MediaTek `android_usb` ACM (`0e8d:2007`, shell on ttyGS0). |
| 2026-07-19 | **L1.1 SUCCESS:** flash OK; host sees stable `0e8d:2007 MediaTek Inc. L1-Linux-ACM` + `/dev/ttyACM0` (no ADB). Init + gadget proved. Interactive shell incomplete (local echo only). |
| 2026-07-19 | **L1.2 built:** heartbeat + proper ttyGS0 fd redirect / getty for interactive shell. |
| 2026-07-19 | **LINUX SHELL LIVE:** `0e8d:2007 L1-Linux-ACM`, `dl7006#`, root, kernel 3.18.35. LCM=`ZS070BE3019B3H7II_713`. |
| 2026-07-19 | **Lab work:** mounted system/data/cache + SD; RW `/mnt/sd/linux-lab`; fb0 draw; see docs/14-live-linux-lab.md |
| 2026-07-19 | **NEONWIRE fbui:** cyberpunk framebuffer HUD live on mtkfb (static ARM binary, JetBrains Mono, cyberdesign tokens). Deploy over serial to SD, no reflash. docs/17-cyberpunk-fbui.md |
| 2026-07-19 | **neui touch launcher** live: nav rail + 5 panels (system/proc/storage/kernlog/net) reading real kernel data; evdev touch on event4 (1:1 map). Shared `fbgfx.h`. |
| 2026-07-19 | **Action tiles** added to neui: SYNC/CLR LOG/REMOUNT/REBOOT(confirm) with toast; dispatch refactored + `--tap X Y` headless test hook. Verified (dmesg 1759→2 on CLR LOG). |
| 2026-07-19 | **Touch fixed:** type-A MT on event4 (codes 53/54), tap on BTN_TOUCH edge + SYN. **Display fixed:** panel is command-mode MIPI — `fb_present` must pan-flip 3 buffers or memcpy never reaches the glass. Live UI now smooth, taps switch panels. |
| 2026-07-19 | **BOOT FACE LIVE:** flashed boot-linux-l1-neonos.img; device boots straight into NeonOS UI (neui autostarts from initramfs). Next: replace DIGILAND LK logo @ 0x4400000. |
| 2026-07-19 | **Wi-Fi research + repack fix:** kernel/DT identical to Android → blocker is runtime state (docs/18). Disproved fixed-addr SPM patch (kernel uses ioremap). Control image bootlooped → isolated repack recompression as the cause → fixed with GNU `gzip -9 -n` same-size piggy swap (control zImage now byte-identical to stock). Ready: `flash-wifi.sh control` + `wifi-diag.sh`. |
| 2026-07-19 | **Wi-Fi instrument (docs/18):** host power PERFECT (CON=0x10d), TOPAXI OPEN, chipId still 0 on L1; BT same wall. Repack via FNAME-pad solved. |
| 2026-07-19 | **Wi-Fi reframed (docs/19):** Digiland **DL7006-KB** stock Android on this unit had working Wi‑Fi (`chipid=0x8127`, `wlan.driver.status=ok`). “Silicon dead” conclusion SUPERSEDED. Next: Android vs L1 bisect + instrument-v3 (RGU/OSC/EMI). Handoff for Claude: docs/19-handoff-wifi-bisect.md |
| 2026-07-19 | **Fastboot = info-only** (locked bootloader): `getvar` works (partition map: boot 16MB, logo 3MB, expdb 10MB, product MID7006AL, secure/locked), but `boot`/`flash`/`reboot` blocked ("not allowed in locked state"). Preloader (mtkclient) stays the only real flash path. |
| 2026-07-19 | **NeonOS UX: output tool tiles.** Action bar now DF/MEM/MOUNTS/DMESG (popen → scrollable neon overlay, tap-scroll + [X]) + SYNC/REBOOT. Live on device; deploy over serial. |
| 2026-07-19 | **NEONWIRE boot splash built:** reversed MTK logo.bin, replaced DIGILAND splash (slots 0 & 38) with neon splash, battery frames untouched, verified round-trip. Flash `./tools/flash-neonos.sh logo` @ 0x4400000 (restore-logo to revert). |
| 2026-07-19 | **Stock-Android recipe raid** (`reference/android-capture/`): read working kernel log via `adb logcat -b kernel` (logd CAP_SYSLOG bypasses shell dmesg block). Captured Wi-Fi (docs/21), audio (docs/23: MT6323, only `Speaker_Amp_Switch`+DAPM, `hw:0,5`/`hw:0,1`), camera (docs/22: **SP2509** 2MP i2c 0x7a). Board=`hcn/mid721l_96e_mipi`, nvram@0x400000 (MAC xx:xx:xx:xx:xx:xx). chipId=0 is a RED HERRING (prints on working Android). |
| 2026-07-19 | **Wi-Fi on-device (docs/21 "Resume here"):** driver stack comes up on L1 — chip **detects 0x8127**, `mtk_stp_wmt`+`mtk_wmtd` live, firmware staged @/etc/firmware, native `wmtctl_min` does `SET_STP_MODE(SDIO)=ok`. **Blocker relocated to SDIO transport:** `hif_sdio_stp_on: no supported func probed` — connsys SDIO func never enumerates (`/sys/bus/sdio/devices` empty). NOT silicon/power/firmware. NEXT: why `11230000.mmc` doesn't enumerate connsys func (msdc rescan / SDIO power / DTB). Tools: `experiments/net/wifi-up.sh`, `wmtctl_min.c`. |
