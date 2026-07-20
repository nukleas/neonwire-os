#!/usr/bin/env python3
"""Repack patched vmlinux into L1.4 boot.img (stock DTB + L1 ramdisk).

Patch: mtk_wcn_consys_power_on → direct SPM MTCMOS (see patch_stock_power_on.py).
"""
from __future__ import annotations

import gzip
import io
import struct
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
                unused = d.unused_data or b""
                comp_len = len(z[idx:]) - len(unused)
                return idx, comp_len
        except Exception:
            pass
        pos = idx + 1


def rebuild_zimage(stock_z: bytes, new_vmlinux: bytes, dtb: bytes) -> bytes:
    piggy_off, old_comp_len = find_gzip_piggy(stock_z)
    print(f"piggy @ {piggy_off} old_comp={old_comp_len}")

    buf = io.BytesIO()
    with gzip.GzipFile(fileobj=buf, mode="wb", compresslevel=9, mtime=0) as gz:
        gz.write(new_vmlinux)
    new_comp = buf.getvalue()
    print(f"new piggy compressed {len(new_comp)} (was {old_comp_len})")

    head = stock_z[:piggy_off]
    body = bytearray(head + new_comp + dtb)
    while len(body) % 4:
        body.append(0)

    # zImage end field @ 0x2c
    struct.pack_into("<I", body, 0x2C, len(body))
    print(f"zImage total {len(body)} (stock {len(stock_z)}) end={len(body):#x}")
    return bytes(body)


def main() -> None:
    vmlinux = (HERE / "vmlinux.patched.bin").read_bytes()
    dtb = (HERE / "stock-appended.dtb").read_bytes()
    stock_z = STOCK_Z.read_bytes()
    stock_boot = STOCK_BOOT

    zimage = rebuild_zimage(stock_z, vmlinux, dtb)
    (HERE / "zImage.patched").write_bytes(zimage)

    # L1 ramdisk from existing packed boot if present, else from verify
    l1_boot = L1 / "out/boot-linux-l1.img"
    if not l1_boot.is_file():
        raise SystemExit(f"missing {l1_boot}")

    l1 = l1_boot.read_bytes()
    page = struct.unpack_from("<I", l1, 36)[0]
    ksz = struct.unpack_from("<I", l1, 8)[0]
    rsz = struct.unpack_from("<I", l1, 16)[0]
    koff = page
    # extract stock MTK kernel header template from L1 / stock
    stock_b = stock_boot.read_bytes()
    sksz = struct.unpack_from("<I", stock_b, 8)[0]
    k_template = stock_b[page : page + 512]
    roff = ((page + sksz + page - 1) // page) * page
    r_template = stock_b[roff : roff + 512]

    # L1 ramdisk MTK blob
    l1_roff = ((page + ksz + page - 1) // page) * page
    ramdisk_mtk = l1[l1_roff : l1_roff + rsz]

    kernel_mtk = make_mtk_header("KERNEL", zimage, template=k_template)

    boot = build_boot(
        stock_boot=stock_boot,
        kernel_mtk=kernel_mtk,
        ramdisk_mtk=ramdisk_mtk,
        page_size=page,
    )
    # preserve L1 cmdline
    boot = bytearray(boot)
    boot[64 : 64 + 512] = l1[64 : 64 + 512]
    boot = bytes(boot)

    out = L1 / "out/boot-linux-l1.4-consys.img"
    out.write_bytes(boot)
    meta = out.with_suffix(".meta.txt")
    meta.write_text(
        f"boot={out}\n"
        f"size={len(boot)} (0x{len(boot):x})\n"
        f"zImage={len(zimage)}\n"
        f"kernel_mtk={len(kernel_mtk)}\n"
        f"ramdisk_mtk={len(ramdisk_mtk)}\n"
        f"patch=mtk_wcn_consys_power_on @ c059c144 SPM MTCMOS direct\n"
        f"flash_offset=0x1d80000\n"
    )
    print("wrote", out, "size", len(boot))
    print(meta.read_text())


if __name__ == "__main__":
    main()
