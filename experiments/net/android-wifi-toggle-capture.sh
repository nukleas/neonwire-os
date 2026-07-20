#!/system/bin/sh
# android-wifi-toggle-capture.sh
#
# Run ON the tablet under stock Android (adb shell as root if possible).
# Captures what the system does when Wi-Fi is turned OFF then ON.
#
#   adb root   # if available
#   adb push experiments/net/android-wifi-toggle-capture.sh /data/local/tmp/
#   adb shell sh /data/local/tmp/android-wifi-toggle-capture.sh
#   adb pull /sdcard/wifi-toggle-capture ./reference/probe/android-wifi-toggle/
#
# No root? Still get logcat + dumpsys + getprop; dmesg/sysfs may be limited.

set +e
OUT="${OUT:-/sdcard/wifi-toggle-capture}"
mkdir -p "$OUT"
TS=$(date +%Y%m%d-%H%M%S 2>/dev/null || echo now)
LOG="$OUT/run-$TS"
mkdir -p "$LOG"

say() { echo "==== $* ===="; echo "==== $* ====" >> "$LOG/notes.txt"; }

# --- helpers ---
snap() {
  tag="$1"
  d="$LOG/$tag"
  mkdir -p "$d"
  getprop 2>/dev/null | grep -iE 'wlan|wifi|wcn|wmt|conn' > "$d/getprop-wifi.txt"
  dumpsys wifi 2>/dev/null > "$d/dumpsys-wifi.txt"
  dumpsys connectivity 2>/dev/null | head -200 > "$d/dumpsys-connectivity-head.txt"
  ip link 2>/dev/null > "$d/ip-link.txt"
  ip addr 2>/dev/null > "$d/ip-addr.txt"
  ls -l /sys/class/net 2>/dev/null > "$d/sys-class-net.txt"
  cat /sys/devices/soc/18070000.consys/power/runtime_status 2>/dev/null > "$d/consys-runtime_status.txt"
  cat /sys/devices/soc/18070000.consys/power/control 2>/dev/null > "$d/consys-control.txt"
  ls -l /sys/devices/soc/18070000.consys/driver 2>/dev/null > "$d/consys-driver.txt"
  for c in connsys_bus; do
    echo -n "$c enable=" > "$d/clk-$c.txt"
    cat /sys/kernel/debug/clk/$c/clk_enable_count 2>/dev/null >> "$d/clk-$c.txt"
    echo -n " rate=" >> "$d/clk-$c.txt"
    cat /sys/kernel/debug/clk/$c/clk_rate 2>/dev/null >> "$d/clk-$c.txt"
  done
  # VCN rails if debugfs present
  for r in vcn18 vcn28 vcn33_wifi vcn33_bt; do
    [ -d /sys/kernel/debug/regulator/$r ] || continue
    echo -n "$r " >> "$d/regulators.txt"
    cat /sys/kernel/debug/regulator/$r/enable 2>/dev/null >> "$d/regulators.txt"
    echo -n " users=" >> "$d/regulators.txt"
    cat /sys/kernel/debug/regulator/$r/num_users 2>/dev/null >> "$d/regulators.txt"
    echo >> "$d/regulators.txt"
  done
  # WMT debug nodes
  ls -la /dev/stpwmt /dev/wmtWifi /dev/wmtdetect 2>/dev/null > "$d/wmt-devs.txt"
  ls -la /proc/driver/wmt* 2>/dev/null > "$d/wmt-proc.txt"
  cat /proc/net/wireless 2>/dev/null > "$d/proc-net-wireless.txt"
  ps 2>/dev/null | grep -iE 'wmt|wifi|wpa|netd' > "$d/ps-wifi.txt"
  # dmesg tail for this moment
  dmesg 2>/dev/null | tail -80 > "$d/dmesg-tail.txt"
  echo "snap $tag done" >> "$LOG/notes.txt"
}

say "0. prep — clear dmesg if root, start logcat"
# root-only often
dmesg -C 2>/dev/null || true
logcat -c 2>/dev/null || true

# background logcat (filtered)
logcat -v time \
  WifiService:V WifiStateMachine:V WifiNative:V WifiMonitor:V \
  wpa_supplicant:V Netd:V ConnectivityService:V \
  WMT:V WMT-CORE:V WMT-CONSYS:V WMT-LIB:V WMT-FUNC:V \
  MTK-WIFI:V HIF-SDIO:V WLAN:V \
  *:S \
  > "$LOG/logcat-wifi.txt" 2>&1 &
LC_PID=$!
echo "logcat pid=$LC_PID" >> "$LOG/notes.txt"
sleep 1

snap "01-before"

say "1. turn Wi-Fi OFF (svc)"
svc wifi disable 2>&1 | tee -a "$LOG/notes.txt"
sleep 4
snap "02-after-off"

say "2. turn Wi-Fi ON (svc)"
# mark kernel log if possible
echo "WIFI_TOGGLE_ON_MARK" > /dev/kmsg 2>/dev/null || true
svc wifi enable 2>&1 | tee -a "$LOG/notes.txt"
# wait for driver / DHCP window
sleep 2
snap "03-two-sec-after-on"
sleep 6
snap "04-eight-sec-after-on"
sleep 8
snap "05-sixteen-sec-after-on"

say "3. stop logcat, dump full dmesg slice"
kill $LC_PID 2>/dev/null
wait $LC_PID 2>/dev/null
dmesg 2>/dev/null > "$LOG/dmesg-full.txt"
dmesg 2>/dev/null | grep -iE 'WMT|CONSYS|chipId|WIFI|wlan|HIF-SDIO|pm_runtime|scpsys|regulator|vcn|connsys' \
  > "$LOG/dmesg-wifi-filtered.txt"

say "4. optional: read /dev/wmtWifi state / driver status"
getprop wlan.driver.status > "$LOG/wlan.driver.status.txt" 2>/dev/null
getprop persist.mtk.wcn.combo.chipid > "$LOG/combo.chipid.txt" 2>/dev/null
getprop service.wcn.driver.ready > "$LOG/wcn.driver.ready.txt" 2>/dev/null

# summary for humans
{
  echo "timestamp=$TS"
  echo "wlan.driver.status=$(getprop wlan.driver.status 2>/dev/null)"
  echo "chipid=$(getprop persist.mtk.wcn.combo.chipid 2>/dev/null)"
  echo "wcn.ready=$(getprop service.wcn.driver.ready 2>/dev/null)"
  echo "ifaces=$(ls /sys/class/net 2>/dev/null | tr '\n' ' ')"
  echo "dmesg chipId lines:"
  grep -i chipId "$LOG/dmesg-wifi-filtered.txt" 2>/dev/null | tail -5
  echo "logcat path: $LOG/logcat-wifi.txt"
} | tee "$LOG/SUMMARY.txt"

echo
echo "DONE. Pull with:"
echo "  adb pull $OUT reference/probe/android-wifi-toggle/"
echo "Focus files: SUMMARY.txt logcat-wifi.txt dmesg-wifi-filtered.txt */getprop-wifi.txt */dumpsys-wifi.txt"
