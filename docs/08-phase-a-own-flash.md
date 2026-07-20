# Phase A — Own flash

**Status:** Active  
**Charter:** [00-charter.md](00-charter.md)

Goal: attach via Preloader, read the flash map, dump images we trust, prove we can write them back.

## A1 — Host tooling

Already largely done in this repo:

```bash
cd $REPO
source tools/venv/bin/activate
cd tools/mtkclient
python mtk.py --help
```

Install udev (once, needs sudo):

```bash
sudo $REPO/tools/install-udev.sh
```

Optional convenience:

```bash
source $REPO/tools/mtk-env.sh
mtk --help
```

## A2 — Preloader on demand

1. Unplug USB.  
2. Power tablet **fully off** (long Power or pinhole reset).  
3. Start the tool **first** (it waits for the device).  
4. Plug USB; if needed hold **Volume Up** (try **Volume Down** if Up fails).  
5. Confirm host sees `0e8d:2000` / `MT65xx Preloader` briefly:

```bash
journalctl -kf | rg -i '0e8d|preloader|mediatek'
```

Notes from this host:

- Preloader has appeared as `0e8d:2000` then dropped in ~2–3 s when nothing claimed it.  
- When Android is partially up: `0e8d:2008` MTP (not useful for Phase A dumps).

## A3 — GPT / partition table

```bash
source $REPO/tools/venv/bin/activate
cd $REPO/tools/mtkclient

python mtk.py printgpt | tee $REPO/reference/dumps/printgpt-$(date +%Y%m%d).txt
```

Commit or keep that text file. It becomes the partition name source of truth.

## A4 — Dump

Create a dated dump directory:

```bash
DUMP=$REPO/reference/dumps/$(date +%Y%m%d-%H%M%S)
mkdir -p "$DUMP"
cd $REPO/tools/mtkclient
source ../venv/bin/activate
```

**Important (this unit):** legacy DA only exposes partition names
`0`, `1`, `2`, `3`, `4`, `cache`, `data` — **not** `boot` / `recovery` / `user`.

Also: **one output file per partition** (comma-separated filenames).

```bash
# Prefer sudo if non-root handshake fails
sudo $(which python) mtk.py r 0,1,2,3,4 \
  "$DUMP/0.bin,$DUMP/1.bin,$DUMP/2.bin,$DUMP/3.bin,$DUMP/4.bin"

# optional large ones (cache 256MiB; data multi-GiB — skip data until needed)
sudo $(which python) mtk.py r cache "$DUMP/cache.bin"
```

**All named partitions into a folder:**

```bash
sudo $(which python) mtk.py rl "$DUMP/parts/"
```

**By absolute flash offset** (use `ro`, not `r user`):

```bash
sudo $(which python) mtk.py ro 0x900000 0xa00000 "$DUMP/off_0x900000.bin"
sudo $(which python) mtk.py ro 0x1300000 0xa00000 "$DUMP/off_0x1300000.bin"
```

**Whole eMMC user area** (~7 GiB, slow):

```bash
sudo $(which python) mtk.py rf "$DUMP/flash-user.bin"
```

Write a short `README.md` in `$DUMP` with:

- date  
- mtkclient git revision (`git -C tools/mtkclient rev-parse --short HEAD`)  
- commands used  
- whether boot still worked after  

### What to prioritize

| Partition (typical names) | Why |
|---------------------------|-----|
| `boot` | Kernel + ramdisk — Phase B |
| `recovery` | Recovery experiments |
| `lk` / `uboot` | Early bootloader stage |
| `seccfg` | Lock state (read; don’t casual-write) |
| `nvram` / `proinfo` / `protect*` | Calibration — **read only** until expert |
| `system` / `userdata` | Large; dump if space allows |

## A5 — Integrity

```bash
cd "$DUMP"
sha256sum * 2>/dev/null | tee SHA256SUMS
```

Store sums next to images. Re-hash before any restore.

## A6 — Restore proof

**Do this only after A4–A5.**

Safest proof patterns:

1. **Read-only confidence:** dump twice, compare hashes (same image twice → same SHA256).  
2. **Write-back same bytes:** `python mtk.py w boot "$DUMP/boot.bin"` (or whatever filename mtk wrote) using the dump you just took — should change nothing functionally.  
3. **Safe damage + repair:** wipe `userdata` / `cache`, confirm mess, restore from dump or factory-reset behavior.

Example write-back (names must match your dump files):

```bash
python mtk.py w boot /path/to/dumps/.../boot.bin
```

Avoid first experiments on `nvram`, `proinfo`, `preloader`, `protect*`.

### Third-party stock ZIP

Only if self-dump is incomplete:

- See [07-option-c-stock-reflash.md](07-option-c-stock-reflash.md)  
- Google Drive pack often **quota-blocked**  
- Always compare scatter layout to `printgpt` before mass write  

## A7 — Runbook capture

When something works, update:

- this file (commands that actually attached)  
- [checklist.md](checklist.md) log  
- `reference/dumps/<date>/README.md`  

## Failure cheat sheet

| Symptom | Try |
|---------|-----|
| Tool waits forever | Off tablet, better cable, hold Vol±, run tool as root once to rule out udev |
| `0e8d:2008` / `0e8d:20ff` only | OS is up — power fully off; don’t use MTP for dumps |
| Preloader seen, **Handshake failed** | See lab note below; retry timing; `printgpt --stock`; try as root; try Vol Up = BROM |
| Attach then error | Retry; try `--debugmode`; note DA/preloader messages |
| Wrong partition names | Re-run `printgpt`; don’t invent names from random guides |
| Drive ZIP is HTML/2 KB | Quota page — not a firmware file |

### Lab note (2026-07-18) — handshake failed, then **success with sudo**

Early attempts:

1. Cold plug → kernel: `MT65xx Preloader` / `0e8d:2000` (~3 s).  
2. mtkclient without proper USB claim: **Handshake failed**.  
3. Device auto-boots Android as `0e8d:20ff` — USB 5 V wakes the tablet.  

**What worked:** run mtkclient as **root** (`sudo python mtk.py printgpt …`), tablet fully off, plug when waiting.

Success details archived in `reference/dumps/20260718-success/`:

- CPU **MT8127**, HW code `0x8127`, **unprotected** (SBC/SLA/DAA all false)  
- Legacy DA stage1/2 (`MTK_DA_V5.bin`)  
- RAM **1 GiB**, eMMC user **~7.1 GiB**  
- MBR slots + `cache` / `data` printed  

Helper: `tools/phase-a-printgpt.sh` (still prefer `sudo` if non-root handshake fails).

## Exit to Phase B

Phase A is done when:

- [ ] GPT saved  
- [ ] boot (+ recovery) dumped and hashed  
- [ ] At least one restore or write-back proof  
- [ ] You trust `reference/dumps/` more than internet ZIPs  

Then: unpack `boot.img` and start [charter Phase B](00-charter.md).
