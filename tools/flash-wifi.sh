#!/usr/bin/env bash
# Flash the Wi-Fi/CONSYS experiment boot images, or restore.
#
#   ./tools/flash-wifi.sh control        # UNPATCHED repack (isolates repack vs patch) — flash FIRST
#   ./tools/flash-wifi.sh instrument     # printk live SPM_CONN_PWR_CON/PWR_STATUS (dmesg "CDBG ...")
#   ./tools/flash-wifi.sh fix            # force scpsys_power_on(conn) (dmesg "CFIX ...")
#   ./tools/flash-wifi.sh patch          # OLD fixed-addr shellcode — WRONG, refuses
#   ./tools/flash-wifi.sh restore        # back to NEONWIRE OS boot face
#   ./tools/flash-wifi.sh restore-l1     # plain L1 (serial shell, no UI)
#   ./tools/flash-wifi.sh restore-stock  # stock Android boot
#
# Method: fully power off, run script, plug USB no-buttons, wait for write OK.
# All images use the same corrected repack (repack_boot.py). Boot partition @0x1d80000.
# Recovery if anything bootloops: restore  ->  restore-l1  ->  restore-stock.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/experiments/linux-initramfs/out"
SESSION="$ROOT/reference/dumps/session-20260718"
OFFSET=0x1d80000

mode="${1:-}"
case "$mode" in
  control)       IMG="$OUT/boot-linux-l1.4-control.img";     echo "=== FLASH repack CONTROL (unpatched) — clean L1 test vehicle ===" ;;
  instrument)    IMG="$OUT/boot-linux-wifi-instrument.img";  echo "=== FLASH CONSYS INSTRUMENT (SPM CONN power dump via printk) ===" ;;
  fix)           IMG="$OUT/boot-linux-wifi-fix.img";         echo "=== FLASH CONSYS FIX (force scpsys_power_on for CONN) ===" ;;
  patch)         echo "!! consys-v2 uses FIXED 0xF000_xxxx SPM addrs — CONFIRMED WRONG for this"
                 echo "!! kernel (it uses ioremap). It would fault. Do the wifi-diag.sh run on the"
                 echo "!! 'control' image instead; see docs/18. Refusing to flash."; exit 1 ;;
  restore)       IMG="$OUT/boot-linux-l1-neonos.img";        echo "=== RESTORE NEONWIRE OS boot face ===" ;;
  restore-l1)    IMG="$OUT/boot-linux-l1.img";               echo "=== RESTORE plain L1 (serial shell) ===" ;;
  restore-stock) IMG="$SESSION/images/boot.img";             echo "=== RESTORE stock Android boot ===" ;;
  *) echo "usage: $0 [control|patch|restore|restore-l1|restore-stock]"; exit 1 ;;
esac

[[ -f "$IMG" ]] || { echo "missing $IMG"; exit 1; }
LEN=$(stat -c%s "$IMG"); LEN_HEX=$(printf '0x%x' "$LEN")
echo "image:  $IMG"
echo "size:   $LEN ($LEN_HEX)   offset: $OFFSET"
echo
echo "Power OFF fully, unplug. Press Enter, then plug USB (no buttons)."
case "$mode" in
  control) echo "Expect: boots to L1 ACM shell (0e8d:2007). If it BOOTLOOPS -> the repack"
           echo "        pipeline is still wrong; do NOT flash 'patch'. Restore and report." ;;
  instrument|fix)
           echo "Expect: boots to L1 ACM shell (same as control). Then run the WMT bring-up:"
           echo "        sh /mnt/sd/linux-lab/wifi-diag.sh 2>&1 | tee /mnt/sd/linux-lab/wifi-diag.log"
           echo "        Read back:  dmesg | grep -E 'CDBG|CFIX|chipId'"
           echo "        CDBG/CFIX <SPMbase> <CONN_PWR_CON> <PWR_STATUS>  (all hex)."
           echo "        CON=0x0d & (STATUS&0x2)=0x2  => CONN MTCMOS powered+un-isolated."
           echo "        CON bit1(ISO)=1 or bit2(PWR_ON)=0 or (STATUS&0x2)=0 => domain NOT up." ;;
  patch)   echo "Expect: boots to L1 ACM shell. Then bring up WMT and check chipId:"
           echo "        (mount system, wmt_loader, wmt_launcher, echo 1 > /dev/wmtWifi, dmesg|grep chipId)" ;;
esac
echo "Bootloop recovery ladder: $0 restore  ->  restore-l1  ->  restore-stock"
echo
read -r -p "Press Enter to start mtkclient (needs sudo)..." _
source "$ROOT/tools/venv/bin/activate"
cd "$ROOT/tools/mtkclient"
sudo "$(which python)" mtk.py wo "$OFFSET" "$LEN_HEX" "$IMG"
echo
echo "Done. Unplug USB, power on."
