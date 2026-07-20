# Home Assistant Companion (client only)

Tablet is a **dashboard client**, not the HA server.

## Installed

| | |
|--|--|
| Package | `io.homeassistant.companion.android` |
| Version | **2026.7.3-full** (minSdk 23 — OK for Android 7 / API 24) |
| APK | `reference/apks/homeassistant-full-2026.7.3.apk` |
| Install | `adb install -r …` → Success |

## Network notes (this unit)

- Wi‑Fi works (example: `192.168.0.241` on LAN).
- **`homeassistant.local` often does NOT resolve** on Android 7 (weak/no mDNS).  
  Prefer the server’s **LAN IP**, e.g. `http://192.168.0.x:8123`.

Find HA IP on the machine that runs HA:

```bash
# on HA host / router / HA Settings → System → Network
hostname -I
# or from another PC that resolves .local:
getent hosts homeassistant.local
ping -c1 homeassistant.local
```

## First-run on the tablet

1. Open **Home Assistant** app (already launched once via adb).  
2. When asked for server URL, use:  
   `http://<HA-LAN-IP>:8123`  
   or `https://…` if you use TLS.  
3. Log in with your HA user.  
4. Optional: enable sensors you want (battery, etc.) — keep few on 1 GB RAM.

## Panel-friendly settings (already applied via adb)

```bash
adb shell settings put global stay_on_while_plugged_in 3
adb shell settings put system screen_off_timeout 2147483647
```

In the HA app: Settings → Companion app → enable **Keep screen on** / fullscreen if available.

## Reinstall / update

```bash
adb install -r reference/apks/homeassistant-full-2026.7.3.apk
adb shell monkey -p io.homeassistant.companion.android -c android.intent.category.LAUNCHER 1
```

## If the app is too heavy

Fall back to a light browser kiosk (Fully Kiosk, older Chrome/Firefox APK) pointed at the same URL.
