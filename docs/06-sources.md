# Sources & references

Scraped / consulted while identifying the DL7006-KB and planning flash / reuse.  
Links rot; keep local copies of critical PDFs and firmware under `reference/`.

## Regulatory / OEM identity

| Source | What we used |
|--------|----------------|
| [FCC ID XMF-MID7006 (fccid.io)](https://fccid.io/XMF-MID7006) | Grantee Lightcomm, product MID7006, dates, exhibits list, radio grants |
| FCC User Manual exhibit (XMF-MID7006 / doc 3436933) | **DL7006** title; specs: Android 7.0, **MTK8127** quad 1.3 GHz, 1 GB / 8 GB, Wi-Fi a/b/g/n, microSD 32 GB, 2100 mAh |
| FCC Internal Photos exhibit | Board layout, battery, shield can |
| Lightcomm / HCN contact patterns on grants | White-label tablet OEM context (`hcn2000`-class contact domains on related filings) |

Mirror entry points:

- https://fcc.report/FCC-ID/XMF-MID7006  
- https://fccid.io/XMF-MID7006  

## Retail / market confirmation

| Source | Notes |
|--------|-------|
| Best Buy product pages (archived listings) | Digiland 7″ 8 GB **DL7006-KB** |
| eBay / marketplace product IDs | DigiLand DL7006 7″ tablet |
| la-tronics / reseller blurbs | Android 7.0 DL7006-KB mentions |
| Reddit r/AndroidQuestions | Digiland DL7006-KB setup wizard issues (device exists in wild) |

## Firmware / flash ecosystem

| Source | Notes |
|--------|-------|
| [firmwarefile.com — Everest Digiland DL7006-KB](https://firmwarefile.com/everest-digiland-dl7006-kb) | Stock ZIP name `Everest_Digiland_DL7006-KB_MT8127_20170605_7.0.zip` (~908 MB); SPFT pointers |
| NeedROM Everest category | “EVEREST DL7006”, Android 7.0, **ONLY MT8127** |
| ROM provider / KurdishFirmware style indexes | Paths containing `MT8127`, `digiland`, `DL7006`, `hcn8127` |
| [androidmtk.com SP Flash tutorials](https://androidmtk.com/flash-stock-rom-using-smart-phone-flash-tool) | Generic SPFT procedure linked by firmware sites |
| SP Flash Tool distribution pages | Windows / Linux builds (third-party hosts — be careful) |

## Root / community

| Source | Notes |
|--------|-------|
| [XDA — rooting DigiLand DL7006](https://xdaforums.com/t/rooting-digiland-dl7006-how.3742343/) | 2018 discussion; SP Flash + boot.img root path mentioned in snippets |

(XDA may present bot challenges; archive locally if useful replies appear.)

## Related MT8127 development (not device-specific)

| Source | Notes |
|--------|-------|
| GitHub `mt8127-tadpole` / Quanta narnia (LeapFrog Epic) | Example Lineage-oriented MT8127 device trees |
| postmarketOS wiki (MediaTek categories, mainlining guides) | General MTK porting context; **no DL7006 device page found** |
| bkerler **mtkclient** | Modern BROM/preloader dump/flash tooling |

## Host observations (this project)

Recorded 2026-07-18 on the workstation used for this repo:

| Observation | Value |
|-------------|--------|
| Preloader VID:PID | `0e8d:2000` |
| Preloader product | `MT65xx Preloader` |
| Android VID:PID | `0e8d:2008` |
| Android serial | `0123456789ABCDEF` |
| Interface when booted | MTP (ADB not yet enabled) |
| Host `adb` | Not installed initially |

## Spec cross-checks

| Source | Notes |
|--------|-------|
| iFixit-style notes for DigiLand **DL701Q** | MT8127 / Mali-450 class sibling — use only as family context |
| Various “1 GB RAM DigiLand” marketplace filters | Consistent budget SKU positioning |

## What we did *not* find

- Official DigiLand developer site with GPL kernel drops for DL7006  
- Official LineageOS / pmOS device wiki pages for DL7006  
- Confirmed unlockable fastboot with public unlock tools  
- Verified clean SHA256 from DigiLand corporate (third-party ZIPs only)

## Local archive suggestions

When downloading anything important:

```text
reference/
  fcc/
    XMF-MID7006-user-manual.pdf
    XMF-MID7006-internal-photos.pdf
  firmware/
    SHA256SUMS
    *.zip
  probe/
    <timestamps>/
  dumps/
    <timestamps>/
```

Document provenance in each folder’s short README.
