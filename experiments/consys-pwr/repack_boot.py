#!/usr/bin/env python3
"""Repack an (optionally patched) vmlinux into an L1 boot.img.

Same pipeline for the CONTROL (unpatched) and any patched build, so a bootloop
on the control isolates the repack itself vs. the patch.

  ./repack_boot.py --vmlinux vmlinux.bin          --out .../boot-linux-l1.4-control.img
  ./repack_boot.py --vmlinux vmlinux.patched.bin  --out .../boot-linux-l1.4-consys.img
"""
from __future__ import annotations

import argparse
import struct
import subprocess
import sys
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools"))
from repack_bootimg import build_boot, make_mtk_header  # noqa: E402

HERE = Path(__file__).resolve().parent
L1 = ROOT / "experiments/linux-initramfs"
STOCK_BOOT = ROOT / "reference/dumps/session-20260718/images/boot.img"
STOCK_Z = L1 / "out/verify/kernel.stripped"


def find_gzip_piggy(z: bytes) -> tuple[int, int]:
    pos = 0
    while True:
        idx = z.find(b"\x1f\x8b\x08", pos)
        if idx < 0:
            raise SystemExit("no gzip piggy found in zImage")
        try:
            d = zlib.decompressobj(16 + zlib.MAX_WBITS)
            raw = d.decompress(z[idx:]) + d.flush()
            if len(raw) > 1_000_000:
                comp_len = len(z[idx:]) - len(d.unused_data or b"")
                return idx, comp_len
        except Exception:
            pass
        pos = idx + 1


def rebuild_zimage(stock_z: bytes, new_vmlinux: bytes) -> bytes:
    """SAME-SIZE in-place piggy swap — the robust way to patch a prebuilt zImage.

    The decompressor stub has baked-in length assumptions (input_data_end, reloc
    offsets); a different-sized piggy shifts them and bootloops (that was L1.4 AND
    the Python-gzip control). Fix: the kernel build uses GNU `gzip -9 -n`, which on
    our exact vmlinux reproduces the *byte-identical* original piggy. So recompress
    with GNU gzip, pad the (complete) gzip member up to the exact original length
    (inflate stops at the gzip end-of-stream; trailing bytes are ignored), and splice
    it in place. Result: total size unchanged, every downstream offset/edata/DTB
    position identical to stock — for an unpatched kernel, byte-identical to stock."""
    assert struct.unpack_from("<I", stock_z, 0x24)[0] == 0x016F2818, "not an ARM zImage"
    piggy_off, old_comp_len = find_gzip_piggy(stock_z)
    comp = subprocess.run(["gzip", "-9", "-n", "-c"], input=new_vmlinux,
                          capture_output=True, check=True).stdout
    if len(comp) > old_comp_len:
        raise SystemExit(
            f"recompressed piggy {len(comp)} > original {old_comp_len} — cannot "
            f"pad down. Need a smaller patch or a stronger deflater (zopfli).")
    pad = old_comp_len - len(comp)
    piggy = comp + b"\x00" * pad                        # ignored past the gzip EOS
    body = stock_z[:piggy_off] + piggy + stock_z[piggy_off + old_comp_len:]
    assert len(body) == len(stock_z), "same-size swap invariant broken"
    print(f"piggy @ {piggy_off}: gzip -9 -n = {len(comp)}  (pad {pad} to hit {old_comp_len})")
    print(f"zImage size unchanged = {len(body)}; byte-identical-to-stock = {body == stock_z}")
    return body


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--vmlinux", default=str(HERE / "vmlinux.bin"))
    ap.add_argument("--dtb", default=str(HERE / "stock-appended.dtb"))
    ap.add_argument("--out", default=str(L1 / "out/boot-linux-l1.4-control.img"))
    ap.add_argument("--label", default="control (unpatched vmlinux — repack isolation)")
    args = ap.parse_args()

    vmlinux = Path(args.vmlinux).read_bytes()
    stock_z = STOCK_Z.read_bytes()
    stock_b = STOCK_BOOT.read_bytes()

    zimage = rebuild_zimage(stock_z, vmlinux)

    l1_boot = L1 / "out/boot-linux-l1.img"
    l1 = l1_boot.read_bytes()
    page = struct.unpack_from("<I", l1, 36)[0]
    ksz = struct.unpack_from("<I", l1, 8)[0]
    rsz = struct.unpack_from("<I", l1, 16)[0]
    sksz = struct.unpack_from("<I", stock_b, 8)[0]
    k_template = stock_b[page : page + 512]
    l1_roff = ((page + ksz + page - 1) // page) * page
    ramdisk_mtk = l1[l1_roff : l1_roff + rsz]

    kernel_mtk = make_mtk_header("KERNEL", zimage, template=k_template)
    boot = bytearray(build_boot(stock_boot=STOCK_BOOT, kernel_mtk=kernel_mtk,
                                ramdisk_mtk=ramdisk_mtk, page_size=page))
    boot[64 : 64 + 512] = l1[64 : 64 + 512]             # preserve L1 cmdline
    boot = bytes(boot)

    out = Path(args.out)
    out.write_bytes(boot)
    out.with_suffix(".meta.txt").write_text(
        f"label={args.label}\nvmlinux={args.vmlinux}\nsize={len(boot)} (0x{len(boot):x})\n"
        f"zImage={len(zimage)} kernel_mtk={len(kernel_mtk)} ramdisk_mtk={len(ramdisk_mtk)}\n"
        f"flash_offset=0x1d80000\n")
    print(f"wrote {out} size {len(boot)} (0x{len(boot):x})")
    if len(boot) > 0x1000000:
        raise SystemExit("boot image > 16 MiB — abort")


if __name__ == "__main__":
    main()
