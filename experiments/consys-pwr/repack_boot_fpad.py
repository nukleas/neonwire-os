#!/usr/bin/env python3
"""Correct same-size piggy repack for the ARM zImage.

WHY the previous repack bootlooped: arch/arm/boot/compressed/head.S loads the
kernel's *uncompressed* size from `input_data_end - 4` (the gzip ISIZE trailer),
i.e. the LAST 4 BYTES of the piggy region. Padding the piggy with trailing zeros
(to reach the original compressed length) overwrote that location with 0 -> the
decompressor mallocs a 0-byte output -> boot loop. TAIL PADDING IS FATAL.

CORRECT approach (verified against lib/decompress_inflate.c on this tree):
  * The decompressor checks bytes[0..2] == 1f 8b 08, uses a FIXED 10-byte header,
    then - and ONLY - skips an ASCIIZ filename if FLG.FNAME (bit3, 0x08) is set.
    It does NOT handle FEXTRA(0x04) or FCOMMENT(0x10). It then raw-inflates
    (-MAX_WBITS) and stops at the final DEFLATE block; it never reads the trailer.
  * So we pad by putting the slack in a big FNAME field BEFORE the deflate data.
    The deflate stream + 4B CRC + 4B ISIZE stay at the END, so ISIZE lands exactly
    at input_data_end-4, and total piggy length == original (all downstream
    offsets/edata/reloc/DTB unchanged).

Member layout we emit (total == original compressed length):
  1f 8b 08 08 00 00 00 00 XFL OS | 'A'*(pad-1) 00 | <raw deflate> | CRC32 | ISIZE

Compress with gzip -9 -n if it already fits, else zopfli (stronger). Both are
wrapped identically, so the boot-critical structure is compressor-independent.

Hard self-tests before writing:
  - len(member) == original compressed length
  - member[-4:] == (len(vmlinux) & 0xffffffff) little-endian   (the ISIZE)
  - gzip.decompress(member) == vmlinux    (full gzip compliance incl. CRC)
  - total zImage size == stock zImage size
"""
from __future__ import annotations
import argparse, gzip, struct, subprocess, sys, zlib
from pathlib import Path

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
            raise SystemExit("no gzip piggy found in zImage")
        try:
            d = zlib.decompressobj(16 + zlib.MAX_WBITS)
            raw = d.decompress(z[idx:]) + d.flush()
            if len(raw) > 1_000_000:
                return idx, len(z[idx:]) - len(d.unused_data or b"")
        except Exception:
            pass
        pos = idx + 1


def compress_member(vmlinux: bytes, budget: int) -> bytes:
    """Return a complete gzip member (header+deflate+crc+isize) <= budget bytes."""
    g = subprocess.run(["gzip", "-9", "-n", "-c"], input=vmlinux,
                       capture_output=True, check=True).stdout
    if len(g) <= budget:
        print(f"  gzip -9 -n member = {len(g)} (<= {budget}) [fast path]")
        return g
    import zopfli.gzip as zg
    z = zg.compress(vmlinux)
    print(f"  gzip too big ({len(g)}); zopfli member = {len(z)} (<= {budget}? {len(z)<=budget})")
    if len(z) > budget:
        raise SystemExit(f"even zopfli {len(z)} > budget {budget}")
    return z


def fname_pad_member(member: bytes, vmlinux: bytes, total: int) -> bytes:
    """Re-wrap a gzip member to EXACTLY `total` bytes using FNAME padding, keeping
    the deflate stream + CRC + ISIZE at the end (ISIZE == last 4 bytes)."""
    assert member[:3] == b"\x1f\x8b\x08"
    xfl, os_ = member[8], member[9]
    raw_deflate = member[10:-8]           # original header is 10 bytes, trailer 8
    crc = member[-8:-4]
    isize = member[-4:]
    # sanity: reconstruct with no pad and check it decompresses
    assert gzip.decompress(b"\x1f\x8b\x08\x00\x00\x00\x00\x00" + bytes([xfl, os_])
                           + raw_deflate + crc + isize) == vmlinux
    pad = total - (10 + len(raw_deflate) + 8)   # bytes to place in FNAME field
    if pad < 0:
        raise SystemExit(f"member too large for budget by {-pad} bytes")
    header = bytes([0x1f, 0x8b, 0x08, 0x08, 0, 0, 0, 0, xfl, os_])  # FLG.FNAME set
    if pad == 0:
        header = bytes([0x1f, 0x8b, 0x08, 0x00, 0, 0, 0, 0, xfl, os_])  # no fname
        fname = b""
    else:
        fname = b"A" * (pad - 1) + b"\x00"       # ASCIIZ, no interior NUL
    out = header + fname + raw_deflate + crc + isize
    # ---- hard self-tests ----
    assert len(out) == total, (len(out), total)
    assert out[-4:] == struct.pack("<I", len(vmlinux) & 0xFFFFFFFF), "ISIZE not at tail"
    assert gzip.decompress(out) == vmlinux, "member does not decompress to vmlinux"
    print(f"  member rewrapped: {len(out)} B, FNAME pad {pad}, ISIZE@tail "
          f"{out[-4:].hex()} == len(vmlinux)&ffffffff")
    return out


def rebuild_zimage(stock_z: bytes, new_vmlinux: bytes) -> bytes:
    assert struct.unpack_from("<I", stock_z, 0x24)[0] == 0x016F2818, "not an ARM zImage"
    piggy_off, old_comp_len = find_gzip_piggy(stock_z)
    print(f"piggy @ {piggy_off}, original compressed len {old_comp_len}")
    member = compress_member(new_vmlinux, old_comp_len)
    member = fname_pad_member(member, new_vmlinux, old_comp_len)
    body = stock_z[:piggy_off] + member + stock_z[piggy_off + old_comp_len:]
    assert len(body) == len(stock_z), "same-size invariant broken"
    print(f"zImage size {len(body)} == stock {len(stock_z)} : {len(body)==len(stock_z)}")
    # extra end-to-end check: the spliced piggy still round-trips in place
    d = zlib.decompressobj(16 + zlib.MAX_WBITS)
    got = d.decompress(body[piggy_off:piggy_off + old_comp_len]) + d.flush()
    assert got == new_vmlinux, "in-place piggy does not round-trip"
    print("in-place piggy round-trips to patched vmlinux : True")
    return body


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--vmlinux", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--label", default="fname-pad repack")
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
