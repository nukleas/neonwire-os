# Phase B — Own boot

**Status:** Active (Phase A dumps organized under `reference/dumps/session-20260718/`)  
**Charter:** [00-charter.md](00-charter.md)

## Inputs (do not modify in place)

| Artifact | Path |
|----------|------|
| boot.img | `reference/dumps/session-20260718/images/boot.img` |
| recovery.img | `reference/dumps/session-20260718/images/recovery.img` |
| preloader | `reference/dumps/session-20260718/images/preloader.bin` |
| system | `reference/dumps/session-20260718/images/system.img` |
| Full flash | `reference/dumps/session-20260718/raw/flash-user.bin` |

Flash offsets: [MAP.md](../reference/dumps/session-20260718/MAP.md).

## B1 — Unpack boot / recovery

Work directory: `reference/dumps/session-20260718/work/`

```bash
# helper (python, no extra packages)
python3 tools/unpack_bootimg.py \
  reference/dumps/session-20260718/images/boot.img \
  reference/dumps/session-20260718/work/boot

python3 tools/unpack_bootimg.py \
  reference/dumps/session-20260718/images/recovery.img \
  reference/dumps/session-20260718/work/recovery
```

Expect: `kernel`, `ramdisk.cpio*` / extracted root, `header.json` / cmdline notes.

## B2 — Document kernel

- Kernel version string (`strings kernel | grep Linux`)
- Compression (gzip/lz4/mtk header — note `0x88168858` MTK magic at kernel start)
- cmdline if present
- Same kernel in boot vs recovery?

## B3 — Safe experiment loop

1. Copy `images/boot.img` → `work/boot-stock.img`  
2. Modify only in `work/`  
3. Flash with **one** Preloader cycle:  
   `sudo python mtk.py wo 0x1d80000 work/boot-patched.img`  
4. Test boot; on failure restore stock:  
   `sudo python mtk.py wo 0x1d80000 images/boot.img`

## B4 — Root path (optional)

- Magisk on Android 7 / old boot may need older Magisk  
- Or classic `su` inject into ramdisk (learning exercise)  
- Document win/fail

## B5 — Recovery

- Boot recovery key combo (if any)  
- Or `adb reboot recovery` when UI works  
- Stock recovery.img already extracted

## Exit criteria

- [ ] boot + recovery unpacked under `work/`  
- [ ] Kernel version documented  
- [ ] At least one intentional boot write + restore **or** explicit decision not to write yet  
- [ ] Notes in checklist lab log  
