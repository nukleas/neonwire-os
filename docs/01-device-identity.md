# Device identity — DigiLand DL7006-KB / XMF-MID7006

## Marketing vs regulatory names

| Layer | Name |
|-------|------|
| What is printed / sold | DigiLand **DL7006-KB** |
| Alternate retail branding | **Everest** Digiland DL7006 / DL7006-KB (firmware sites) |
| FCC ID | **XMF-MID7006** |
| FCC equipment title | Tablet PC |
| Grantee | **Lightcomm Technology Co., Ltd.** |
| FCC product code | MID7006 |
| Grant / certification window | ~**2017-06-21 … 2017-06-23** |

Lightcomm produced many white-label “MID*” tablets. DigiLand (and Everest rebrands) are retail skins on that hardware.

## Specs (FCC user manual MID7006)

Extracted from FCC exhibit **User Manual** for XMF-MID7006 (manual labeled **DL7006**):

| Spec | Value |
|------|--------|
| Dimensions | 190.7 × 108.7 × 10.3 mm |
| OS | Android **7.0** |
| Processor | **1.3 GHz MTK8127**, Quad-Core |
| Memory | **1 GB RAM**, **8 GB** storage |
| Wi-Fi | 802.11 **a/b/g/n** |
| Connections | Micro-USB, Micro-SD (up to **32 GB**), headphone jack |
| Battery | 3.7 V **2100 mAh** Li-Poly |
| Claimed runtimes (manual) | ~17 h music / ~2.5 h video (marketing) |

### Related market listings (secondary)

Some marketplace/Facebook listings describe similar DigiLand 7″ units as Android 5.1 + 1 GB / 8 GB / Cortex-A7. Prefer the **FCC manual (Android 7.0, MT8127)** and **live `getprop`** from the unit in hand over random listings.

Similar DigiLand MT8127 family examples (for parts search only, not guaranteed identical PCB):

- DL701Q — often cited as MT8127 / Mali-450 (iFixit-style tech notes)

## Radio / FCC radio notes

From FCC grants for XMF-MID7006:

- WLAN 2.4 GHz and 5 GHz (U-NII bands listed)
- Classic Bluetooth + BLE
- Composite Part 15 device (computing device + intentional radiators)

Not a phone with cellular baseband in the FCC materials we reviewed (Wi-Fi tablet class).

## SoC: MediaTek MT8127

| Item | Notes |
|------|--------|
| Family | MediaTek tablet SoC (~2014-era designs still shipping in cheap 2016–2018 SKUs) |
| CPU | 4× ARM Cortex-A7 @ ~1.3 GHz |
| GPU | ARM Mali-450 class |
| Typical use | Budget 7–10″ Android tablets |
| Boot / flash | **Preloader** + Download Agent (DA) via USB |
| Tooling | SP Flash Tool, mtkclient, MTK VCOM drivers (Windows) |

Confirmed on **this host** when the tablet was connected: kernel logged product string **`MT65xx Preloader`** (`idVendor=0e8d`, `idProduct=2000`).

## USB identity (observed on host)

| Mode | VID:PID | Kernel / lsusb string | Meaning |
|------|---------|------------------------|---------|
| Preloader (brief) | `0e8d:2000` | MediaTek **MT65xx Preloader**, `cdc_acm` → `/dev/ttyACM0` | Window for SP Flash / mtkclient |
| Android running | `0e8d:2008` | Android / “Cyrus Technology CS 24” (lsusb quirk name) | MTP interface when booted |
| Serial (MTP) | — | `0123456789ABCDEF` | Generic white-label serial |

Preloader often appears for only **~2–3 seconds** after cable plug or power-on with USB connected, then disconnects when the OS boots. Flashing tools must be **armed first**, then cable/power applied.

### Example host log excerpt

```text
usb ... New USB device found, idVendor=0e8d, idProduct=2000
usb ... Product: MT65xx Preloader
usb ... Manufacturer: MediaTek
cdc_acm ... ttyACM0: USB ACM device
... disconnect ...
usb ... idVendor=0e8d, idProduct=2008
usb ... Product: Android
usb ... Manufacturer: Android
usb ... SerialNumber: 0123456789ABCDEF
```

## Physical ports & controls (manual)

- Power button (on/off, standby)
- Volume
- **Reset pinhole** — paperclip reset forces shutdown; power button to start again
- Micro-USB (charge + data)
- MicroSD
- Headphone

## Internal layout (FCC internal photos)

FCC internal photos show a compact single-board design with:

- Large EMI shield over main SoC / memory area
- Soft pouch Li-Po battery in chassis
- FPC connectors for display / digitizer
- Micro-USB and microSD on board edge
- Wi-Fi antenna cable(s)

Useful if probing test points later; not required for software HA use.

## What this device is *good* for

- **Lab reverse-engineering** (Preloader, dumps, boot chain) — project primary
- Learning MediaTek flash / Android boot on throwaway hardware
- Optional side quest: HA kiosk client on stock Android

## What this device is *not* good for

- GrapheneOS / modern custom ROMs out of the box
- Home Assistant **server**
- Modern Android daily driver
- Expecting official LineageOS / postmarketOS images

See [00-charter.md](00-charter.md), [08-phase-a-own-flash.md](08-phase-a-own-flash.md).
