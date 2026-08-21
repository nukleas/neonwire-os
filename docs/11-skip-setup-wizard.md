# Skip setup wizard offline (no ADB popup)

When `adb` stays **unauthorized** and the Allow dialog never appears, finish setup **without ADB** by rewriting **system** + wiping **userdata**.

## What we changed (offline)

Image: `reference/dumps/session-20260718/work/system-no-wizard/system.img`

| Change | Detail |
|--------|--------|
| Removed | `/priv-app/SetupWizard/SetupWizard.apk` |
| Removed | `/priv-app/Provision/Provision.apk` |
| Removed | `GooglePartnerSetup.apk`, `GoogleOneTimeInitializer.apk` |
| `build.prop` | `ro.setupwizard.mode=**DISABLED**` (was OPTIONAL) |
| Also set | `ro.setupwizard.require_network=no`, wifi_on_exit=false |

SHA256: `6099e51aa941bbfebcb24b54579c2de85d0b04ac61e232fa91f377c28065756f`

Original system remains: `raw/mbr-slots/4.bin`.

## Flash (recommended sequence)

System is mtk partition **`4`**. Data is partition **`data`**. Boot is still raw offset `0x1d80000`.

**Do one command per Preloader session.** If a step fails with USB Overflow / format error, power-cycle and continue with remaining steps — do not assume all three failed.

```bash
# $REPO = clone of this repository
cd $REPO
source tools/venv/bin/activate
cd tools/mtkclient

SYS=../../reference/dumps/session-20260718/work/system-no-wizard/system.img
BOOT=../../reference/dumps/session-20260718/work/boot-adb/boot-adb.img

# --- each step: tablet OFF, unplug, start command, plug on Waiting ---

# 1) New system without wizard (1.5 GiB — several minutes)
sudo $(which python) mtk.py w 4 "$SYS"

# 2) Wipe user data — legacy DA often returns 0xa5 (unsupported format).
#    If it fails, skip and boot anyway (wizard APKs already gone), or try:
#    sudo $(which python) mtk.py e cache
sudo $(which python) mtk.py e data

# 3) ADB-friendly boot — separate session after (1)/(2)
sudo $(which python) mtk.py wo 0x1d80000 $(stat -c%s "$BOOT") "$BOOT"
```

### Lab note (2026-07-19)

- `w 4 system-no-wizard` **succeeded** (full `0x60000000` write).  
- `e data` failed: `Error on sending emmc format command, response: 0xa5` (legacy DA format not supported for that part).  
- Follow-on `wo` boot failed with **USB Overflow** (session dead after format error).  
- **Recovery:** new Preloader cycle → flash boot only; boot and test; wipe data later via recovery/UI if needed.

Or interactive helper:

```bash
./tools/flash-no-wizard.sh        # all three
./tools/flash-no-wizard.sh system # system only
./tools/flash-no-wizard.sh data   # wipe only
```

## After flash

1. Unplug, power on (first boot can take a few minutes).  
2. You should get **Launcher3** (or a short residual screen), **not** Google account.  
3. ```bash
   adb devices
   ```
   Prefer `device`. If still `unauthorized`, reboot once more; keys are in ramdisk `/adb_keys`.

## Restore stock system

```bash
sudo $(which python) mtk.py w 4 \
  ../../reference/dumps/session-20260718/raw/mbr-slots/4.bin
```

## Why this is better than fighting the popup

| Approach | Needs UI popup | Reliability |
|----------|----------------|-------------|
| ADB skip settings | Yes (auth) | Fails if unauthorized |
| **System APK remove + data wipe** | **No** | High for lab units |
| Account skip in wizard | Sometimes | Fickle on GMS tablets |

## Note on Google apps

Play Store / GMS may still exist but setup wizard is gone. For a kiosk you can ignore or later debloat with ADB once authorized.
