#!/bin/sh
# wifi-diag — clean CONSYS/Wi-Fi bring-up + full state capture on the L1 shell.
#
# The stock 3.18 kernel HAS working CONN power-on (scpsys_power_on, ioremap'd SPM).
# The blocker is that the genpd/pm_runtime chain doesn't reach it for 18070000.consys.
# This script reproduces the stock userspace recipe WITHOUT poking power/control
# (which self-corrupts pm_runtime), then dumps exactly where the chain breaks so we
# can craft the right fix. Reversible: touches no flash, pokes no power/control.
#
#   sh /mnt/sd/linux-lab/wifi-diag.sh 2>&1 | tee /mnt/sd/linux-lab/wifi-diag.log
set +e
SYS=/mnt/system
CONSYS=/sys/devices/soc/18070000.consys
say(){ echo; echo "==== $* ===="; }

say "0. kernel + cold state (BEFORE any bring-up)"
uname -a
echo "consys driver bound? :"; ls -l $CONSYS/driver 2>/dev/null || echo "  (unbound)"
echo "runtime_status: $(cat $CONSYS/power/runtime_status 2>/dev/null)"
echo "control:        $(cat $CONSYS/power/control 2>/dev/null)"
echo "genpd domains:"; cat /sys/kernel/debug/pm_genpd/pm_genpd_summary 2>/dev/null | head -30 || echo "  (no pm_genpd debugfs)"
echo "connsys_bus clk: $(cat /sys/kernel/debug/clk/connsys_bus/clk_enable_count 2>/dev/null)"
echo "VCN regulators:"; for r in /sys/class/regulator/*; do n=$(cat $r/name 2>/dev/null); case "$n" in *vcn*|*VCN*) echo "  $n use=$(cat $r/use_count 2>/dev/null) open=$(cat $r/open_count 2>/dev/null)";; esac; done

say "1. mount /system + bind bionic env"
mount -t ext4 -o ro,noload /dev/mmcblk0p6 $SYS 2>/dev/null
mkdir -p /system /vendor /system/etc/firmware /data /tmp/fw
mount --bind $SYS /system 2>/dev/null
mount --bind $SYS/vendor /vendor 2>/dev/null
export PATH=/system/bin:/system/xbin:/vendor/bin:$PATH
export LD_LIBRARY_PATH=/system/lib:/vendor/lib
cp /vendor/firmware/* /tmp/fw/ 2>/dev/null
cp /vendor/firmware/WMT_SOC.cfg /tmp/fw/WMT.cfg 2>/dev/null
echo "firmware in /tmp/fw:"; ls /tmp/fw 2>/dev/null | tr '\n' ' '; echo
grep -i co_clock /tmp/fw/WMT*.cfg 2>/dev/null

say "2. wmt_loader (registers WMT/char devices)"
/vendor/bin/wmt_loader 2>&1 | head -5
sleep 1
echo "char devs:"; ls -l /dev/stpwmt /dev/wmtWifi /dev/wmtdetect 2>/dev/null
# create nodes from sysfs if missing
for d in stpwmt wmtWifi wmtdetect; do
  s=/sys/class/$d/$d/dev; [ -e "$s" ] || s=$(ls /sys/class/$d/*/dev 2>/dev/null | head -1)
  if [ -n "$s" ] && [ ! -e /dev/$d ]; then
    mknod /dev/$d c "$(cut -d: -f1 $s)" "$(cut -d: -f2 $s)" 2>/dev/null && echo "  created /dev/$d"
  fi
done

say "3. IS the consys platform driver bound now? (try binding if not)"
if [ ! -e $CONSYS/driver ]; then
  echo "still unbound — trying manual bind:"
  for drv in /sys/bus/platform/drivers/*consys* /sys/bus/platform/drivers/*mtk_wcn* /sys/bus/platform/drivers/*wmt*; do
    [ -d "$drv" ] || continue
    echo "  echo 18070000.consys > $drv/bind"
    echo 18070000.consys > "$drv/bind" 2>&1
  done
fi
echo "driver: $(ls -l $CONSYS/driver 2>/dev/null | sed 's/.*-> //')"
echo "runtime_status now: $(cat $CONSYS/power/runtime_status 2>/dev/null)"

say "4. wmt_launcher + power on Wi-Fi (NO power/control poking)"
/vendor/bin/wmt_launcher -p /tmp/fw/ >/tmp/fw/launcher.log 2>&1 &
sleep 2
echo "mtk_wmtd thread: $(ps 2>/dev/null | grep -c '[m]tk_wmtd')"
echo 1 > /dev/wmtWifi 2>&1
sleep 2

say "5. RESULT — did CONSYS power on?"
echo "chipId in dmesg:"; dmesg 2>/dev/null | grep -i chipid | tail -3
echo "connsys_bus clk: $(cat /sys/kernel/debug/clk/connsys_bus/clk_enable_count 2>/dev/null)"
echo "runtime_status: $(cat $CONSYS/power/runtime_status 2>/dev/null)"
echo "VCN use_count:"; for r in /sys/class/regulator/*; do n=$(cat $r/name 2>/dev/null); case "$n" in *vcn*|*VCN*) echo "  $n use=$(cat $r/use_count 2>/dev/null)";; esac; done
echo "wlan0 present?:"; ls /sys/class/net | tr '\n' ' '; echo

say "6. failure fingerprint (for the fix)"
dmesg 2>/dev/null | grep -iE "consys|scpsys|genpd|pm_runtime|WMT|chipid|power on" | tail -25
echo
echo "SUMMARY: chipId $(dmesg 2>/dev/null | grep -io 'chipId(0x[0-9a-f]*)' | tail -1)   wlan0 $([ -e /sys/class/net/wlan0 ] && echo YES || echo no)"
echo "capture saved if you ran with: | tee /mnt/sd/linux-lab/wifi-diag.log"
