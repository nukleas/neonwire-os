# Live probe — fingerprint the unit on the desk

Before flashing anything, capture what *this* tablet actually runs. White-label boards share FCC IDs across SKUs; software builds differ.

## Host packages (Arch / Omarchy)

```bash
sudo pacman -S android-tools
# optional later:
# sudo pacman -S android-udev   # if udev rules needed
```

Confirm:

```bash
adb version
which adb
```

## Tablet: enable ADB

1. **Settings → About tablet**
2. Tap **Build number** 7 times → Developer options unlocked
3. **Settings → Developer options**
   - Enable **USB debugging**
   - Optionally **Stay awake** (while charging) for kiosk testing
4. Connect Micro-USB cable
5. Accept the RSA fingerprint prompt on the tablet if shown
6. Set USB mode to **File transfer / MTP** (not charge-only)

## Detect device

```bash
adb devices -l
lsusb | rg -i '0e8d|mediatek|android'
```

Expected when debugging works:

```text
List of devices attached
0123456789ABCDEF    device
```

If empty / `unauthorized`:

- Unplug/replug, unlock screen, re-accept prompt
- Check cable (some charge-only cables fail data)
- Confirm Developer options still enabled

If only MTP and no ADB interface, USB debugging is off or the build lacks adbd in the current USB config.

## Property dump (save to this repo)

From the project root:

```bash
mkdir -p reference/probe
OUT=reference/probe/$(date +%Y%m%d-%H%M%S)
mkdir -p "$OUT"

adb shell getprop > "$OUT/getprop.txt"
adb shell getprop ro.product.model >> "$OUT/summary.txt"
adb shell getprop ro.product.device >> "$OUT/summary.txt"
adb shell getprop ro.product.board >> "$OUT/summary.txt"
adb shell getprop ro.board.platform >> "$OUT/summary.txt"
adb shell getprop ro.hardware >> "$OUT/summary.txt"
adb shell getprop ro.mediatek.platform >> "$OUT/summary.txt"
adb shell getprop ro.build.display.id >> "$OUT/summary.txt"
adb shell getprop ro.build.version.release >> "$OUT/summary.txt"
adb shell getprop ro.build.version.sdk >> "$OUT/summary.txt"
adb shell getprop ro.build.fingerprint >> "$OUT/summary.txt"
adb shell getprop ro.serialno >> "$OUT/summary.txt"
adb shell getprop ro.boot.serialno >> "$OUT/summary.txt"

adb shell cat /proc/cpuinfo > "$OUT/cpuinfo.txt"
adb shell cat /proc/meminfo > "$OUT/meminfo.txt"
adb shell free -m > "$OUT/free.txt"
adb shell df -h > "$OUT/df.txt"
adb shell ls -la /dev/block/platform/ 2>/dev/null > "$OUT/block-platform.txt" || true
adb shell ls -la /dev/block/by-name/ 2>/dev/null > "$OUT/by-name.txt" || true
adb shell getenforce 2>/dev/null > "$OUT/selinux.txt" || true
adb shell uname -a > "$OUT/uname.txt"

# Optional: package list (large)
adb shell pm list packages -f > "$OUT/packages.txt" 2>/dev/null || true

echo "Wrote probe to $OUT"
cat "$OUT/summary.txt"
```

### Properties that matter for flash research

| Property | Why |
|----------|-----|
| `ro.board.platform` / `ro.mediatek.platform` | Confirms MTK family string |
| `ro.hardware` | Kernel board name |
| `ro.product.device` / `ro.product.model` | Scatter / firmware matching |
| `ro.build.display.id` | Exact stock build |
| `ro.build.fingerprint` | Unique software ID |
| `ro.build.version.release` + SDK | Android version vs FCC claim |

## Partition map (when ADB works)

```bash
adb shell "ls -l /dev/block/by-name 2>/dev/null || ls -l /dev/block/platform/*/by-name 2>/dev/null"
```

Typical MTK names (names vary by scatter):

- `preloader`, `lk` / `uboot`, `boot`, `recovery`, `logo`
- `system`, `cache`, `userdata`
- `nvram`, `proinfo`, `protect1` / `protect2` (calibration / NVRAM — **do not wipe casually**)

## Preloader observation (no ADB required)

With tablet powered off:

1. Watch host:

   ```bash
   # terminal 1
   sudo dmesg -w
   # or
   journalctl -kf
   ```

2. Plug USB while holding power / or just plug cold unit.

3. Look for `MT65xx Preloader` / `0e8d:2000` briefly.

This confirms the SP Flash / mtkclient path is reachable even if Android is broken.

## Fastboot?

Many cheap MTK tablets **do not** expose a useful unlocked fastboot for end users the way Pixels do. Prefer:

- Preloader + SP Flash Tool / mtkclient  
- Stock recovery (if any)  

```bash
# only if volume combo enters fastboot and adb reboot works:
adb reboot bootloader
fastboot devices
fastboot getvar all
```

Document whether fastboot appears at all on *this* unit.

## Soft info from the UI (no PC)

If ADB is blocked, photograph:

- Settings → About tablet (full screen)
- Storage usage
- Wi-Fi MAC (for inventory)
- Any “Legal / Kernel version” lines

## Success criteria for Phase 0

- [ ] `adb devices` shows `device`
- [ ] Probe folder written under `reference/probe/`
- [ ] Confirmed Android version + build ID
- [ ] Confirmed platform / hardware strings
- [ ] Confirmed free RAM and usable storage
- [ ] Preloader still appears on cold plug (dmesg)
