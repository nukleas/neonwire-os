#!/usr/bin/env python3
"""Repack Android boot.img with MediaTek KERNEL/ROOTFS headers (MT8127-style)."""
from __future__ import annotations

import argparse
import os
import struct
import zlib
from pathlib import Path


def page_align(n: int, page: int) -> int:
    return (n + page - 1) // page * page


def make_mtk_header(name: str, payload: bytes, template: bytes | None = None) -> bytes:
    """512-byte MTK image header; magic LE 0x58881688, size, 8-char name."""
    if template is not None and len(template) >= 512:
        hdr = bytearray(template[:512])
    else:
        hdr = bytearray(512)
        # 0xff padding used by this device's stock headers
        for i in range(40, 512):
            hdr[i] = 0xFF
        for i in range(16, 40):
            hdr[i] = 0x00
    struct.pack_into("<I", hdr, 0, 0x58881688)
    struct.pack_into("<I", hdr, 4, len(payload))
    nb = name.encode("ascii")[:8]
    hdr[8:16] = nb.ljust(8, b"\x00")
    return bytes(hdr) + payload


def pack_newc(ramdisk_dir: Path) -> bytes:
    """Pack directory into newc cpio (070701)."""
    entries: list[tuple[str, Path, int, bytes]] = []

    for root, dirs, files in os.walk(ramdisk_dir, followlinks=False):
        rel_root = os.path.relpath(root, ramdisk_dir)
        if rel_root == ".":
            rel_root = ""
        # directory entries
        if rel_root:
            p = Path(root)
            mode = p.stat().st_mode
            entries.append((rel_root.replace("\\", "/"), p, mode, b""))
        for name in sorted(dirs):
            pass  # dirs visited via walk
        for name in sorted(files):
            p = Path(root) / name
            rel = str(Path(rel_root) / name) if rel_root else name
            rel = rel.replace("\\", "/")
            st = p.lstat()
            if p.is_symlink():
                target = os.readlink(p).encode()
                mode = st.st_mode
                entries.append((rel, p, mode, target))
            else:
                entries.append((rel, p, st.st_mode, p.read_bytes()))

    # ensure root dir "."
    out = bytearray()

    def emit(name: str, mode: int, data: bytes, ino: int):
        name_b = name.encode() + b"\x00"
        namesize = len(name_b)
        filesize = len(data)
        # newc header
        hdr = (
            f"070701"
            f"{ino:08X}"
            f"{mode:08X}"
            f"{0:08X}"  # uid
            f"{0:08X}"  # gid
            f"{1:08X}"  # nlink
            f"{0:08X}"  # mtime
            f"{filesize:08X}"
            f"{0:08X}"  # dev major
            f"{0:08X}"  # dev minor
            f"{0:08X}"  # rdev major
            f"{0:08X}"  # rdev minor
            f"{namesize:08X}"
            f"{0:08X}"  # check
        ).encode("ascii")
        assert len(hdr) == 110
        out.extend(hdr)
        out.extend(name_b)
        while len(out) % 4:
            out.append(0)
        out.extend(data)
        while len(out) % 4:
            out.append(0)

    # root
    emit(".", 0o040755, b"", 1)
    ino = 2
    # collect all paths including dirs
    all_paths: list[Path] = []
    for dirpath, dirnames, filenames in os.walk(ramdisk_dir, topdown=True, followlinks=False):
        dirnames.sort()
        filenames.sort()
        rel = os.path.relpath(dirpath, ramdisk_dir)
        if rel != ".":
            all_paths.append(Path(dirpath))
        for fn in filenames:
            all_paths.append(Path(dirpath) / fn)

    for p in all_paths:
        rel = str(p.relative_to(ramdisk_dir)).replace("\\", "/")
        st = p.lstat()
        if p.is_dir() and not p.is_symlink():
            emit(rel, st.st_mode, b"", ino)
        elif p.is_symlink():
            emit(rel, st.st_mode, os.readlink(p).encode(), ino)
        else:
            emit(rel, st.st_mode, p.read_bytes(), ino)
        ino += 1

    emit("TRAILER!!!", 0, b"", ino)
    return bytes(out)


def gzip_compress(data: bytes) -> bytes:
    # gzip with mtime 0 for reproducibility-ish
    import gzip
    import io

    buf = io.BytesIO()
    with gzip.GzipFile(fileobj=buf, mode="wb", mtime=0, compresslevel=9) as gz:
        gz.write(data)
    return buf.getvalue()


def build_boot(
    *,
    stock_boot: Path,
    kernel_mtk: bytes,
    ramdisk_mtk: bytes,
    page_size: int = 2048,
) -> bytes:
    stock = stock_boot.read_bytes()
    assert stock[:8] == b"ANDROID!"
    # keep load addresses / tags / name from stock
    kernel_addr = struct.unpack_from("<I", stock, 12)[0]
    ramdisk_addr = struct.unpack_from("<I", stock, 20)[0]
    second_addr = struct.unpack_from("<I", stock, 28)[0]
    tags_addr = struct.unpack_from("<I", stock, 32)[0]
    name = stock[48:64]
    cmdline = stock[64:64 + 512]

    kernel_size = len(kernel_mtk)
    ramdisk_size = len(ramdisk_mtk)
    second_size = 0

    hdr = bytearray(page_size)
    hdr[0:8] = b"ANDROID!"
    struct.pack_into(
        "<IIIIIIII",
        hdr,
        8,
        kernel_size,
        kernel_addr,
        ramdisk_size,
        ramdisk_addr,
        second_size,
        second_addr,
        tags_addr,
        page_size,
    )
    # unused/id fields after page_size in stock may matter little; zero
    hdr[48:64] = name
    hdr[64 : 64 + 512] = cmdline[:512]

    out = bytearray(hdr)
    out.extend(kernel_mtk)
    out.extend(b"\x00" * (page_align(len(out), page_size) - len(out)))
    out.extend(ramdisk_mtk)
    out.extend(b"\x00" * (page_align(len(out), page_size) - len(out)))
    return bytes(out)


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--stock-boot", type=Path, required=True)
    ap.add_argument("--ramdisk-dir", type=Path, required=True, help="Modified ramdisk root")
    ap.add_argument("--output", type=Path, required=True)
    ap.add_argument(
        "--kernel-from-stock",
        action="store_true",
        default=True,
        help="Reuse stock KERNEL (MTK-wrapped) from stock boot",
    )
    args = ap.parse_args()

    stock = args.stock_boot.read_bytes()
    page_size = struct.unpack_from("<I", stock, 36)[0]
    kernel_size = struct.unpack_from("<I", stock, 8)[0]
    koff = page_size
    roff = page_align(koff + kernel_size, page_size)
    kernel_mtk = stock[koff : koff + kernel_size]
    # keep stock KERNEL header template from first 512 of kernel_mtk
    k_template = kernel_mtk[:512]
    # actually kernel_mtk already includes header; keep as-is
    assert kernel_mtk[:4] == bytes([0x88, 0x16, 0x88, 0x58])

    r_template = stock[roff : roff + 512]
    assert r_template[:4] == bytes([0x88, 0x16, 0x88, 0x58])

    cpio = pack_newc(args.ramdisk_dir)
    gz = gzip_compress(cpio)
    ramdisk_mtk = make_mtk_header("ROOTFS", gz, template=r_template)

    boot = build_boot(
        stock_boot=args.stock_boot,
        kernel_mtk=kernel_mtk,
        ramdisk_mtk=ramdisk_mtk,
        page_size=page_size,
    )
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_bytes(boot)
    print(f"wrote {args.output} ({len(boot)} bytes)")
    print(f"  kernel_mtk={len(kernel_mtk)} ramdisk_mtk={len(ramdisk_mtk)} cpio={len(cpio)} gzip={len(gz)}")
    # safety: must fit before recovery
    if len(boot) > 0x1000000:
        raise SystemExit("boot image larger than 16MiB gap before recovery — abort")


if __name__ == "__main__":
    main()
