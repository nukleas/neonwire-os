#!/usr/bin/env python3
"""Minimal Android boot.img v0 unpacker (MTK-era page sizes)."""
from __future__ import annotations

import argparse
import json
import struct
from pathlib import Path


def page_align(n: int, page: int) -> int:
    return (n + page - 1) // page * page


def unpack(boot_path: Path, out_dir: Path) -> dict:
    data = boot_path.read_bytes()
    if data[:8] != b"ANDROID!":
        raise SystemExit(f"not a boot.img (magic={data[:8]!r}): {boot_path}")

    (
        kernel_size,
        kernel_addr,
        ramdisk_size,
        ramdisk_addr,
        second_size,
        second_addr,
        tags_addr,
        page_size,
    ) = struct.unpack_from("<IIIIIIII", data, 8)
    name = data[48:64].split(b"\0")[0].decode("ascii", "replace")
    cmdline = data[64 : 64 + 512].split(b"\0")[0].decode("ascii", "replace")

    if page_size not in (2048, 4096, 16384, 8192):
        raise SystemExit(f"unexpected page_size={page_size}")

    out_dir.mkdir(parents=True, exist_ok=True)
    pos = page_size
    kernel = data[pos : pos + kernel_size]
    pos = page_align(pos + kernel_size, page_size)
    ramdisk = data[pos : pos + ramdisk_size]
    pos = page_align(pos + ramdisk_size, page_size)
    second = data[pos : pos + second_size] if second_size else b""

    (out_dir / "kernel").write_bytes(kernel)
    (out_dir / "ramdisk.cpio").write_bytes(ramdisk)
    if second:
        (out_dir / "second").write_bytes(second)

    note: list[str] = []
    kmagic = kernel[:4].hex() if len(kernel) >= 4 else ""

    def strip_mtk(blob: bytes) -> tuple[bytes, str | None]:
        """MediaTek 512-byte header: magic + size + name @8, payload @0x200."""
        if len(blob) < 0x200 or blob[:4] != bytes([0x88, 0x16, 0x88, 0x58]):
            return blob, None
        sz = struct.unpack_from("<I", blob, 4)[0]
        nm = blob[8:16].split(b"\0")[0].decode("ascii", "replace")
        payload = blob[0x200 : 0x200 + sz] if sz and 0x200 + sz <= len(blob) else blob[0x200:]
        return payload, nm

    kernel_s, kname = strip_mtk(kernel)
    ramdisk_s, rname = strip_mtk(ramdisk)
    (out_dir / "kernel.stripped").write_bytes(kernel_s)
    (out_dir / "ramdisk.stripped").write_bytes(ramdisk_s)
    if kname:
        note.append(f"MTK kernel header name={kname}")
    if rname:
        note.append(f"MTK ramdisk header name={rname}")
    if kernel_s[:2] == b"\x1f\x8b":
        note.append("kernel:gzip")
    if ramdisk_s[:2] == b"\x1f\x8b":
        note.append("ramdisk:gzip")

    meta = {
        "source": str(boot_path),
        "name": name,
        "cmdline": cmdline,
        "page_size": page_size,
        "kernel_size": kernel_size,
        "kernel_addr": hex(kernel_addr),
        "ramdisk_size": ramdisk_size,
        "ramdisk_addr": hex(ramdisk_addr),
        "second_size": second_size,
        "tags_addr": hex(tags_addr),
        "kernel_magic_hex": kmagic,
        "notes": note,
    }
    (out_dir / "header.json").write_text(json.dumps(meta, indent=2) + "\n")

    # best-effort ramdisk extract (gzip + newc cpio, pure Python)
    import gzip
    import shutil

    rd_dir = out_dir / "ramdisk"
    if rd_dir.exists():
        shutil.rmtree(rd_dir)
    rd_dir.mkdir(exist_ok=True)

    payload = ramdisk_s
    if payload[:2] == b"\x1f\x8b":
        try:
            payload = gzip.decompress(payload)
            (out_dir / "ramdisk.cpio.unc").write_bytes(payload)
        except OSError:
            note.append("ramdisk gzip decompress failed")

    def extract_cpio_newc(data: bytes, out: Path) -> int:
        pos = 0
        nfiles = 0
        while pos + 110 <= len(data):
            if data[pos : pos + 6] not in (b"070701", b"070702"):
                nxt = data.find(b"07070", pos + 1)
                if nxt < 0:
                    break
                pos = nxt
                continue
            header = data[pos : pos + 110]

            def h(a: int, b: int) -> int:
                return int(header[a:b], 16)

            namesize = h(94, 102)
            filesize = h(54, 62)
            mode = h(14, 22)
            pos += 110
            fname = data[pos : pos + namesize - 1].decode("utf-8", "replace")
            pos += namesize
            pos = (pos + 3) & ~3
            if fname == "TRAILER!!!":
                break
            file_payload = data[pos : pos + filesize]
            pos += filesize
            pos = (pos + 3) & ~3
            target = out / fname
            ftype = mode & 0o170000
            if ftype == 0o040000:
                target.mkdir(parents=True, exist_ok=True)
            elif ftype == 0o120000:
                target.parent.mkdir(parents=True, exist_ok=True)
                try:
                    target.symlink_to(file_payload.decode())
                except OSError:
                    target.write_bytes(file_payload)
            else:
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(file_payload)
            nfiles += 1
        return nfiles

    n = extract_cpio_newc(payload, rd_dir)
    meta["ramdisk_files"] = n
    meta["notes"] = note
    (out_dir / "header.json").write_text(json.dumps(meta, indent=2) + "\n")

    return meta


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("bootimg", type=Path)
    ap.add_argument("outdir", type=Path)
    args = ap.parse_args()
    meta = unpack(args.bootimg, args.outdir)
    print(json.dumps(meta, indent=2))
    print(f"wrote {args.outdir}")


if __name__ == "__main__":
    main()
