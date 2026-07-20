#!/usr/bin/env python3
"""Same as repack_boot.py but compresses the piggy with zopfli (stronger deflate)
so a patched vmlinux still fits the original piggy length in place.

The kernel decompressor inflates until the gzip end-of-stream and ignores trailing
bytes, so we splice the (smaller) zopfli gzip member and zero-pad up to the exact
original compressed length -> total zImage size, edata, input_data_end, reloc
offsets and DTB position are all unchanged (identical invariant to repack_boot.py).

  ./repack_boot_zopfli.py --vmlinux vmlinux.instrument.bin --out .../boot-...-instrument.img

Needs:  pip install zopfli   (already in tools/venv)
"""
from __future__ import annotations
import argparse, gzip, struct, sys, zlib
from pathlib import Path

raise SystemExit(
    "DEPRECATED / BUGGY: this script tail-pads the piggy with zeros, which "
    "overwrites the gzip ISIZE at input_data_end-4 that head.S reads for the "
    "inflated-size malloc -> BOOTLOOP. Use repack_boot_fpad.py (FNAME padding, "
    "keeps ISIZE at the tail).")

import zopfli.gzip as zg

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools"))
from repack_bootimg import build_boot, make_mtk_header  # noqa: E402

HERE = Path(__file__).resolve().parent
L1 = ROOT / "experiments/linux-initramfs"
STOCK_BOOT = ROOT / "reference/dumps/session-20260718/images/boot.img"
STOCK_Z = L1 / "out/verify/kernel.stripped"


def find_gzip_piggy(z: bytes):
    pos = 0
    while True:
        idx = z.find(b"\x1f\x8b\x08", pos)
        if idx < 0:
            raise SystemExit("no gzip piggy found")
        try:
            d = zlib.decompressobj(16 + zlib.MAX_WBITS)
            raw = d.decompress(z[idx:]) + d.flush()
            if len(raw) > 1_000_000:
                return idx, len(z[idx:]) - len(d.unused_data or b"")
        except Exception:
            pass
        pos = idx + 1


def rebuild_zimage(stock_z: bytes, new_vmlinux: bytes) -> bytes:
    assert struct.unpack_from("<I", stock_z, 0x24)[0] == 0x016F2818, "not an ARM zImage"
    piggy_off, old_comp_len = find_gzip_piggy(stock_z)
    comp = zg.compress(new_vmlinux)                 # zopfli gzip member (-n equivalent)
    assert gzip.decompress(comp) == new_vmlinux, "zopfli round-trip mismatch"
    if len(comp) > old_comp_len:
        raise SystemExit(f"zopfli piggy {len(comp)} > original {old_comp_len}")
    piggy = comp + b"\x00" * (old_comp_len - len(comp))
    body = stock_z[:piggy_off] + piggy + stock_z[piggy_off + old_comp_len:]
    assert len(body) == len(stock_z), "same-size invariant broken"
    print(f"piggy @ {piggy_off}: zopfli={len(comp)} pad={old_comp_len-len(comp)} -> {old_comp_len}")
    print(f"zImage size unchanged={len(body)}  identical-to-stock={body==stock_z}")
    return body


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--vmlinux", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--label", default="zopfli repack")
    args = ap.parse_args()

    vmlinux = Path(args.vmlinux).read_bytes()
    stock_z = STOCK_Z.read_bytes()
    stock_b = STOCK_BOOT.read_bytes()
    zimage = rebuild_zimage(stock_z, vmlinux)

    l1 = (L1 / "out/boot-linux-l1.img").read_bytes()
    page = struct.unpack_from("<I", l1, 36)[0]
    ksz = struct.unpack_from("<I", l1, 8)[0]
    rsz = struct.unpack_from("<I", l1, 16)[0]
    k_template = stock_b[page:page + 512]
    l1_roff = ((page + ksz + page - 1) // page) * page
    ramdisk_mtk = l1[l1_roff:l1_roff + rsz]

    kernel_mtk = make_mtk_header("KERNEL", zimage, template=k_template)
    boot = bytearray(build_boot(stock_boot=STOCK_BOOT, kernel_mtk=kernel_mtk,
                                ramdisk_mtk=ramdisk_mtk, page_size=page))
    boot[64:64 + 512] = l1[64:64 + 512]
    boot = bytes(boot)
    out = Path(args.out)
    out.write_bytes(boot)
    out.with_suffix(".meta.txt").write_text(
        f"label={args.label}\nvmlinux={args.vmlinux}\nsize={len(boot)}\nflash_offset=0x1d80000\n")
    print(f"wrote {out} size {len(boot)} (0x{len(boot):x})")
    if len(boot) > 0x1000000:
        raise SystemExit("boot > 16 MiB")


if __name__ == "__main__":
    main()
