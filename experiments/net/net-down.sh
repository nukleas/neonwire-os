#!/bin/sh
# net-down — stop the RNDIS network services and revert the gadget to ACM-only.
set +e
AUSB=/sys/class/android_usb/android0
killall telnetd 2>/dev/null
killall udhcpd 2>/dev/null
if [ -d "$AUSB" ]; then
  echo 0 > "$AUSB/enable" 2>/dev/null
  echo acm > "$AUSB/functions" 2>/dev/null
  echo 02 > "$AUSB/bDeviceClass" 2>/dev/null
  echo 1 > "$AUSB/enable" 2>/dev/null
fi
echo "[net-down] reverted to ACM serial only."
