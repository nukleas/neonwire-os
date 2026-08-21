# Boot ADB patch (Path 1 — keep features)

Patched stock **boot.img** ramdisk for insecure ADB while keeping the stock kernel (Wi‑Fi / touch / display unchanged).

## Artifacts

| File | Role |
|------|------|
| `reference/dumps/session-20260718/images/boot.img` | **Stock** — never delete; restore source |
| `reference/dumps/session-20260718/work/boot-adb/boot-adb.img` | **Patched** |
| `work/boot-adb/ramdisk/` | Working tree used for repack |
| `work/boot-adb/default.prop.stock` | Original props backup |

### Property changes (`default.prop`)

| Prop | Stock | Patched |
|------|-------|---------|
| `ro.secure` | 1 | **0** |
| `ro.adb.secure` | 1 | **0** |
| `ro.debuggable` | 0 | **1** |
| `persist.sys.usb.config` | mtp | **mtp,adb** |

Kernel payload is **byte-identical** to stock (MTK `KERNEL` block reused).

## Flash (device)

`mtk.py wo` needs **three** args: `offset` `length` `filename` (length = file size in hex or decimal).

```bash
# $REPO = clone of this repository
cd $REPO

# Interactive helper (sudo + Preloader wait):
./tools/flash-boot-adb.sh

# Or manual — patched boot (~0x743800 bytes):
source tools/venv/bin/activate
cd tools/mtkclient
# tablet OFF + unplugged, then:
sudo $(which python) mtk.py wo 0x1d80000 0x743800 \
  ../../reference/dumps/session-20260718/work/boot-adb/boot-adb.img
# when Waiting… plug USB (no buttons)
```

Exact length from file:

```bash
IMG=../../reference/dumps/session-20260718/work/boot-adb/boot-adb.img
sudo $(which python) mtk.py wo 0x1d80000 $(stat -c%s "$IMG") "$IMG"
```

### Restore stock

```bash
./tools/flash-boot-adb.sh restore
# or:
IMG=../../reference/dumps/session-20260718/images/boot.img
sudo $(which python) mtk.py wo 0x1d80000 $(stat -c%s "$IMG") "$IMG"
```

## After flash

1. Unplug, power on, wait for Android.  
2. USB cable to PC; accept any prompts if UI works.  
3. ```bash
   adb kill-server; adb start-server; adb devices
   adb shell getprop ro.debuggable   # expect 1
   adb shell getprop ro.adb.secure   # expect 0
   ```
4. If no adb: try USB mode MTP, reboot once, check cable.  
5. If no boot: restore stock boot immediately.

## Rebuild after further ramdisk edits

```bash
python3 tools/repack_bootimg.py \
  --stock-boot reference/dumps/session-20260718/images/boot.img \
  --ramdisk-dir reference/dumps/session-20260718/work/boot-adb/ramdisk \
  --output reference/dumps/session-20260718/work/boot-adb/boot-adb.img
```

## SHA256 (generate after build)

```bash
sha256sum reference/dumps/session-20260718/images/boot.img \
  reference/dumps/session-20260718/work/boot-adb/boot-adb.img
```

Stock: `df7db881be31484a32d43eda8d328669f976edb1903b67002c77d21375d55157`  

Patched builds:
- **Broken** (bootloop): `1c48dbdf…` — ramdisk files lost execute bits (`init` was 0644)
- **Fixed modes** (still may need RSA popup): `085f8320…`
- **Fixed + embedded host `adb_keys`** (no popup): `2132c8b8212bb0dd03b1f87e1e42ce9eafd7e878896eba2c78412e088b636d4e`

Rebuild always from stock cpio so modes stay correct. Embed `~/.android/adbkey.pub` as ramdisk `/adb_keys` when the Allow dialog never appears.
