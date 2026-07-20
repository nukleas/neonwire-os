# Home Assistant panel use (side quest)

> **Project role:** optional demo after Phase A (own flash) is solid.  
> **Not** the primary mission — see [00-charter.md](00-charter.md).  
> Primary path: reverse-engineering kit (dump / boot / hardware map).

## Design principle

Run **Home Assistant Core / HAOS on a real server** (Pi, mini PC, NAS, VM).  
Use the DL7006 only as a **touchscreen client**:

```text
[ HA server ]  <--- Wi-Fi --->  [ DL7006 kiosk browser / companion ]
```

This tablet: **1 GB RAM**, **1.3 GHz A7**, **8 GB** flash — fine for a simple dashboard, bad as a server.

## Why stock Android is the right “new OS”

For HA kiosk duty you mostly need:

- Working Wi-Fi  
- A full-screen browser or WebView  
- Stay-awake while plugged in  
- Optional remote maintenance  

Stock Android **7.0** already provides that. A custom ROM that breaks Wi-Fi or touch is worse than an old but working system.

## Performance budget

| Feature | On 1 GB MT8127 |
|---------|----------------|
| Lovelace with a few cards | OK |
| Graphs / history panels | Use sparingly |
| Live camera multi-view | Avoid or single low-res stream |
| Animated backgrounds | Off |
| Heavy custom cards / JS | Minimize |
| HA Companion full app | Possible but heavier than kiosk browser |

Prefer a **dedicated “wall” dashboard** with large buttons and few entities.

## Software options (client)

### 1. Fully Kiosk Browser (recommended class)

Commercial / freemium Android kiosk browser used heavily for HA walls:

- Fullscreen, hide status/nav  
- Keep screen on, load URL on boot  
- Motion / screensaver options (device-dependent)  
- Remote admin on LAN (paid features)

Point start URL at:

```text
http://HOME_ASSISTANT_HOST:8123/lovelace/wall
# or
https://HA_URL/lovelace/wall
```

Use a **long-lived access token** or kiosk user with limited rights if exposing auth is a concern.

### 2. Home Assistant Companion (Android)

Official app — notifications, sensors, better auth integration.  
On 1 GB RAM it can feel heavy; try if Fully is overkill or unavailable.

### 3. WallPanel / HomeDash / generic Chrome

Lighter community apps or a pinned Chrome tab. Chrome on Android 7 is ancient and may struggle with modern HA frontend — test current HA versions; if UI is too new, pin HA to a version that still works or use a simplified dashboard.

### 4. Fully-offline local HTML (fallback)

Host a tiny static page on the LAN that hits HA REST/WebSocket with minimal JS — last resort if stock browser cannot run modern Lovelace.

## Tablet setup checklist (stock)

1. Factory reset if previous owner garbage is present (optional).  
2. Skip as much Google setup as possible if offline/privacy preference.  
3. Connect **2.4 GHz Wi-Fi** if 5 GHz is flaky on cheap antennas (test both; FCC lists both bands).  
4. Insert **microSD** (A1 if possible) for APKs, cache, media.  
5. Set display sleep → long / never while charging (Developer: Stay awake).  
6. Install kiosk APK (Play if available, else sideload).  
7. Create HA user `wall-kitchen` (or similar) with limited access.  
8. Build Lovelace view **Wall** — large touch targets, low update rate.  
9. Mount tablet on wall / stand; USB power always on (prefer quality 5 V supply).  
10. Optional: disable Google Play Services updates thrashing, remove unused apps.

## HA server-side tips for weak clients

- Separate dashboard path: `/lovelace/wall`  
- Avoid camera cards by default; use snapshot buttons  
- Prefer entity buttons, lights, climate, scenes  
- Reduce history graph cards  
- Theme: high contrast, large fonts  
- If using SSL, ensure Android 7 trusts the cert chain (Let’s Encrypt usually OK; private CAs may need user cert install)

## Power & reliability

| Topic | Note |
|-------|------|
| Always plugged in | Battery will age; still better than deep cycling for wall use |
| 2100 mAh pack | Fine as UPS for brief outages only |
| Heat | A7 idle is mild; avoid direct sun / sealed hot boxes |
| Wi-Fi drop | Fully-style tools can auto-reload; static DHCP lease for the tablet |

## Optional hardening (later)

After probe + stable kiosk:

- Disable lock screen or use simple PIN if physical access is trusted  
- Restrict Settings via kiosk lockdown  
- Root only if you need deeper app freeze / hosts blocking / custom boot animation removal  

Root is **optional**, not required for a basic wall panel.

## Success criteria

- [ ] Tablet boots to dashboard without touches after power loss + plug-in  
- [ ] Critical lights/scenes controllable with fat UI targets  
- [ ] Survives overnight without OOM thrashing  
- [ ] Recoverable with stock flash if experiments fail  

## When to abandon the tablet

If Wi-Fi chip/drivers are dead, eMMC is failing, or Android 7 cannot render your HA version at all, spend time on a used Fire tablet / old iPad / ESP32 display instead of fighting MT8127 mainline Linux.
