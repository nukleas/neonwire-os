# Flashing & recovery — MediaTek MT8127 path

## Mindset

| Goal | Tooling | Risk |
|------|---------|------|
| Restore stock after soft-brick | SP Flash Tool + scatter package | Medium |
| Dump partitions from *this* unit | mtkclient / SP Flash readback | Low–medium |
| Root (patched boot / Magisk) | Dump → patch → Download Only | Medium–high |
| Full wipe / Format All | SP Flash | **High** — can erase NVRAM / brick |

This board has **no meaningful custom-ROM marketplace**. Flashing is for **recovery and research**, not for “install Ubuntu tomorrow.”

## USB modes reminder

1. **Preloader** `0e8d:2000` — few seconds after connect/power; use for SP Flash / mtkclient.  
2. **Android** `0e8d:2008` — MTP when OS is up.  
3. Arm the tool **first**, then connect power-off tablet (or reboot into preloader window).

## Stock firmware packages (third-party)

Public “stock ROM” packs appear under both DigiLand and Everest branding for **MT8127**:

### Example package metadata (firmwarefile.com)

| Field | Value |
|-------|--------|
| Listing | Everest Digiland **DL7006-KB** |
| File name | `Everest_Digiland_DL7006-KB_MT8127_20170605_7.0.zip` |
| Approx size | ~908 MB |
| Contents (claimed) | Flash file + SP Flash Tool + USB driver + guide |
| Chip | **MT8127** |
| Android | **7.0** |
| Date in name | **20170605** (aligns with 2017 FCC era) |

### Other mirrors / indexes (unverified quality)

- NeedROM: “EVEREST DL7006” — notes **ONLY MT8127**, Android 7.0  
- KurdishFirmware / ROM provider style sites: `…MT8127…digiland…DL7006…7.0…hcn8127…`  
  - Build-path fragment `hcn8127` / `hcn2000` matches Lightcomm / HCN white-label lineage from FCC contact domains  

**Treat third-party ZIPs as untrusted.**

- Prefer scanning for malware  
- Prefer comparing partition layout to a **readback from your tablet**  
- Keep original ZIP + SHA256 in `reference/firmware/` if you download  

### Suggested local layout

```text
reference/firmware/
  README.md                 # where you got it, SHA256, date
  Everest_Digiland_DL7006-KB_MT8127_20170605_7.0.zip
  extracted/                # scatter + images
```

Record:

```bash
sha256sum reference/firmware/*.zip | tee reference/firmware/SHA256SUMS
```

## SP Flash Tool (classic MTK path)

### Concepts

| Term | Meaning |
|------|---------|
| Scatter file | Text map of partition names → file offsets / images |
| DA (Download Agent) | Small payload SPFT loads to talk to eMMC |
| Download Only | Write checked partitions; usual safe mode |
| Firmware Upgrade | Broader rewrite; more aggressive |
| Format All + Download | **Dangerous** — can destroy NVRAM / calibration |

### High-level Windows flow (most documented)

1. Install MTK USB / VCOM drivers from pack (or community pack).  
2. Extract firmware; locate `*_Android_scatter.txt` (name varies).  
3. Open SP Flash Tool → **Scatter-loading** → select scatter.  
4. Confirm only intended partitions checked (or full stock restore).  
5. Mode: **Download Only**.  
6. Click **Download**.  
7. Power off tablet; hold volume if required by guide; plug USB.  
8. Progress bar should run when preloader enumerates.  

### Linux notes

- SP Flash Tool Linux builds exist but are finicky.  
- Many people use **Windows VM with USB passthrough** or **mtkclient** natively.  
- Preloader device may need udev rules for user access to `/dev/ttyACM*` or `/dev/bus/usb/*`.

Example udev sketch (adjust group; untested for this exact unit):

```text
# /etc/udev/rules.d/51-mtk.rules
SUBSYSTEM=="usb", ATTR{idVendor}=="0e8d", MODE="0666", GROUP="uucp"
SUBSYSTEM=="tty", ATTRS{idVendor}=="0e8d", MODE="0666", GROUP="uucp"
```

Then: `sudo udevadm control --reload-rules && sudo udevadm trigger`.

### XDA-era note for DL7006

XDA thread *“rooting DigiLand DL7006 - how?”* (2018) points at **SP Flash Tool + boot.img** as the root path for this model class — i.e. community knowledge assumes **MediaTek SPFT**, not fastboot unlock theater.

## mtkclient (modern Linux-friendly dump/flash)

[bkerler/mtkclient](https://github.com/bkerler/mtkclient) speaks BROM/preloader protocols and can:

- Dump full flash / individual partitions  
- Flash partitions  
- Sometimes unlock / crash BROM on locked devices (SoC-dependent; **MT8127 support varies** — verify on a dump attempt before relying on it)

Typical dump workflow (illustrative — check current CLI):

```bash
# with tablet in preloader / BROM window
python mtk r boot boot.img
python mtk r recovery recovery.img
python mtk rl dumps/          # dump all known partitions to folder
```

Save dumps under:

```text
reference/dumps/<date>/
  boot.img
  recovery.img
  ...
  README.md   # mtkclient version, commands used, success/fail
```

## Safe order of operations

1. **ADB probe** ([02-live-probe.md](02-live-probe.md)) while device still boots.  
2. Optional: full `adb backup` / copy SD / export anything needed.  
3. **Download stock ZIP** and store hashes (do not flash yet).  
4. Attempt **readback / mtkclient dump** of at least `boot`, `recovery`, `nvram` if readable.  
5. Only then test **Download Only** of a single non-critical partition or full stock restore if already soft-bricked.  
6. After any successful restore, re-run ADB probe and archive `getprop`.

## Root research path (later)

If root is required for HA kiosk hardening:

1. Dump `boot.img` from device or stock pack.  
2. Patch with Magisk on a working Android (or host tools if available for this vintage).  
3. Flash **only** `boot` via SPFT Download Only.  
4. Keep original `boot.img` to reverse the change.

Old MT8127 / Android 7 may need older Magisk; newest Magisk may refuse ancient kernels.

## Things that brick white-label MTK tablets

- Flashing **wrong SoC** scatter (e.g. MT6580 / MT6735 packs onto MT8127)  
- **Format All** wiping NVRAM / PRO_INFO  
- Interrupting mid-write without a known-good DA path  
- Using “auth” / SLA locked DAs incorrectly  

If the unit becomes “dead” but still shows **Preloader** briefly, recovery is often still possible with the correct scatter + DA.

## Home lab host prep checklist

- [ ] `android-tools` installed  
- [ ] udev rules for `0e8d` (if flashing from Linux)  
- [ ] Stock firmware ZIP downloaded + SHA256 recorded  
- [ ] Firmware extracted; scatter file path noted  
- [ ] `reference/dumps/` ready for readback  
- [ ] Good cable + powered USB port (hubs sometimes fail preloader timing)  
- [ ] Written plan: which partitions will be written  

## Related docs

- [01-device-identity.md](01-device-identity.md) — confirm SoC before any scatter  
- [05-alternate-os.md](05-alternate-os.md) — why “new OS” is not step 1  
- [06-sources.md](06-sources.md) — firmware & tool links  
