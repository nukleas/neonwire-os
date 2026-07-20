# Option C — Preloader stock reflash (contingency)

> **Project role:** side quest / contingency when self-dump is incomplete.  
> Prefer Phase A dumps from *this* unit ([08-phase-a-own-flash.md](08-phase-a-own-flash.md)).  
> Charter: [00-charter.md](00-charter.md).

Use this when you need a **third-party** MT8127 DL7006 scatter pack because
Android is too broken for ADB and you do not yet have a full self-dump. The tablet
must still briefly enumerate as **MediaTek Preloader** on USB (`0e8d:2000`).

## Goal

Rewrite stock Android 7.0 (MT8127 scatter package) over eMMC so the tablet
boots a clean system again — or supply partition images you could not dump.

## Status on this machine (2026-07-18)

| Item | Status |
|------|--------|
| `android-tools` (adb) | Installed (not required for this path) |
| **mtkclient** | Cloned + venv ready under `tools/` |
| Preloader seen historically | Yes — `MT65xx Preloader` / `0e8d:2000` |
| Stock firmware ZIP | **Blocked** — Google Drive quota (“too many users”); empty/failed downloads |
| udev rules | Present in repo; need `sudo` install once |

You can finish tooling + practice Preloader connect **before** the ZIP arrives.

---

## 1. One-time host setup

```bash
cd $REPO

# Python env already created; re-activate anytime:
source tools/venv/bin/activate
cd tools/mtkclient

# Verify CLI
python mtk.py --help | head
```

### udev rules (required so Preloader is usable without fighting permissions)

```bash
cd $REPO
sudo cp tools/mtkclient/Setup/Linux/52-mtk.rules /etc/udev/rules.d/
sudo cp tools/mtkclient/Setup/Linux/51-edl.rules /etc/udev/rules.d/
# optional Android rules:
# sudo cp tools/mtkclient/Setup/Linux/50-android.rules /etc/udev/rules.d/
sudo udevadm control --reload-rules
sudo udevadm trigger
```

You are already in group `uucp` / `wheel` on this host; after rules reload,
unplug/replug the tablet.

### Wrapper (optional)

```bash
source $REPO/tools/mtk-env.sh
mtk --help
```

---

## 2. Get the stock firmware package

### Target package

| Field | Value |
|-------|--------|
| Name | `Everest_Digiland_DL7006-KB_MT8127_20170605_7.0.zip` |
| Size | ~**908 MB** |
| SoC | **MT8127** |
| Android | **7.0** |
| Free mirror (quota-limited) | https://drive.google.com/file/d/1yJ6W9hfCqbDcaPCQODnn2ZB2fO-wicIW/view |
| Listing | https://firmwarefile.com/everest-digiland-dl7006-kb |
| Paid mirror | firmwaredrive.com (same filename) |

### Recommended download method **right now**

Automated CLI downloads from this machine hit Google’s **“Too many users have
viewed or downloaded this file”** limit. Use a **normal browser** on this PC
(or another machine + USB stick):

1. Open the Drive link above while logged into a Google account if needed.
2. Download the 908 MB ZIP.
3. Place it here:

```text
reference/firmware/Everest_Digiland_DL7006-KB_MT8127_20170605_7.0.zip
```

4. Record hash:

```bash
cd $REPO/reference/firmware
sha256sum Everest_Digiland_DL7006-KB_MT8127_20170605_7.0.zip | tee -a SHA256SUMS
file Everest_Digiland_DL7006-KB_MT8127_20170605_7.0.zip   # must say Zip archive
```

5. Extract:

```bash
cd $REPO/reference/firmware
mkdir -p extracted
unzip -d extracted Everest_Digiland_DL7006-KB_MT8127_20170605_7.0.zip
find extracted -iname '*scatter*' -o -iname '*.txt' | head
```

You need a scatter file similar to `MT8127_Android_scatter.txt` plus partition
images (`boot.img`, `system.img`, etc.). Exact names vary by pack.

**Do not flash** a pack that is not labeled **MT8127** + **DL7006**.

---

## 3. Enter Preloader (tablet steps)

1. **Unplug** USB.
2. Power tablet **fully off** (hold Power ~10–15 s, or pinhole reset).
3. On the PC, start the wait command (next section) **first**.
4. Hold **Volume Up** (sometimes Down; try Up first) and plug USB **or**
   plug USB then briefly press Power — depends on board.
5. Watch for a few seconds of `0e8d:2000` / `MT65xx Preloader` in `dmesg`.

Watch USB:

```bash
journalctl -kf | rg -i '0e8d|preloader|mediatek'
# or
watch -n0.5 lsusb
```

Expected flash window: Preloader appears ~2–3 s, then may disconnect if nothing
claims it. Tools must already be listening.

---

## 4. Safe first commands (no firmware rewrite yet)

With venv + mtkclient:

```bash
source $REPO/tools/venv/bin/activate
cd $REPO/tools/mtkclient

# Listen for Preloader, then print partition table
python mtk.py printgpt
```

If that works, dump a safety backup **before** any write:

```bash
mkdir -p $REPO/reference/dumps/$(date +%Y%m%d)
# dump individual partitions when GPT is known, e.g.:
# python mtk.py r boot,recovery,nvram $REPO/reference/dumps/$(date +%Y%m%d)/
python mtk.py rl $REPO/reference/dumps/$(date +%Y%m%d)/
```

Full `rl` can take a long time and needs disk space. At minimum dump
`boot` + `recovery` + anything named `nvram` / `proinfo` if listed.

### Optional: factory-reset **without** stock ZIP

If the mess is mostly apps/settings (userdata) and the system partition is OK:

```bash
# DANGER: wipes user data / internal “storage” apps
python mtk.py e userdata
# sometimes also:
# python mtk.py e cache
```

Then power on normally. This is **not** a full reflash, but often un-breaks a
mangled Android without downloading 900 MB.

Prefer this only after `printgpt` works and you accept data loss.

---

## 5. Full stock reflash

### Preferred on Linux: mtkclient write from extracted images

After you know partition names from `printgpt` and have matching images:

```bash
# Example shape only — names MUST match your GPT + scatter contents
python mtk.py w boot boot.img
python mtk.py w recovery recovery.img
python mtk.py w system system.img
# ... remaining partitions from the pack, except avoid casual NVRAM wipes
```

Some packs include a full flash script; use **Download Only** style writes
(partition by partition), not “format entire flash” unless you know you need it.

### Alternate: SP Flash Tool (Windows or Linux binary)

Many DigiLand packs bundle SP Flash Tool for Windows. Flow:

1. Install MTK VCOM drivers (Windows).
2. Open SP Flash Tool → **Scatter-loading** → select `*scatter*.txt`.
3. Mode: **Download Only** (not Format All + Download).
4. Click **Download**.
5. Connect powered-off tablet (Preloader window).
6. Wait for green OK.

Linux SPFT builds exist but are often flakier than mtkclient or a Windows VM
with USB passthrough.

---

## 6. After a successful flash

1. Unplug, power on with Power button.
2. First boot can take several minutes (optimizing apps).
3. Complete setup wizard enough to reach home screen.
4. Then optional: enable USB debugging for HA kiosk work
   ([02-live-probe.md](02-live-probe.md), [04-home-assistant-panel.md](04-home-assistant-panel.md)).

If bootloop persists after stock flash:

- Wrong firmware variant (same SoC, different panel/memory map)
- Bad cable / incomplete write
- Hardware fault (eMMC)

Try a second known DL7006 MT8127 pack only if labels still match.

---

## 7. Hard no’s

- Do **not** flash MT6580 / MT6735 / MT8163 packs “because MediaTek”.
- Do **not** use **Format All + Download** as first attempt.
- Do **not** erase `nvram` / `pro_info` / `protect*` unless you have a dump
  and a reason.
- Do **not** unplug mid-write.

---

## 8. Checklist for Option C

- [ ] mtkclient venv works (`python mtk.py --help`)
- [ ] udev rules installed + reloaded
- [ ] Preloader visible in `dmesg` / `lsusb` on demand
- [ ] `python mtk.py printgpt` succeeds
- [ ] Optional dump saved under `reference/dumps/`
- [ ] Stock ZIP downloaded in browser → `reference/firmware/`
- [ ] `file` says Zip archive; SHA256 recorded
- [ ] Scatter extracted; MT8127 confirmed in filenames/contents
- [ ] Flash with Download Only / explicit partition writes
- [ ] Clean boot to setup wizard / home

## Related

- [03-flashing.md](03-flashing.md) — general MediaTek notes  
- [01-device-identity.md](01-device-identity.md) — confirm device  
- [06-sources.md](06-sources.md) — firmware links  
