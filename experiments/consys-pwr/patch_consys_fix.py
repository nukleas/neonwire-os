#!/usr/bin/env python3
"""Fix-attempt patch for DL7006 stock 3.18.35 (same size, in place).

Replaces mtk_wcn_consys_power_on (@0xC059C144, 120 bytes) with:
  1. pm_runtime_get_sync(dev)                 -- the stock genpd path, and then
  2. genpd = pm_genpd_lookup_name("conn")
  3. scpsys_power_on(genpd)                    -- FORCE the kernel's own CONN
     MTCMOS un-isolation directly (clear PWR_ISO, set PWR_RST_B, set PWR_ON/
     PWR_ON_2ND, clear SRAM_PDN, clear TOPAXI CONN bus protect 0x104), using
     the driver's *live* ioremap'd SPM base -- NOT any fixed 0xF000_xxxx addr.

scpsys_power_on already contains the exact, correct MTCMOS sequence (verified by
disassembly), so re-invoking it guarantees CONN is really un-isolated even if the
pm_runtime bookkeeping reported "active" without doing the electrical work. It is
idempotent (re-asserts already-set bits + polls PWR_STATUS), so it is safe if the
domain was already on.

Also prints the post-fix SPM_CONN_PWR_CON / PWR_STATUS so the same dmesg line
("CDBG ...") confirms the result.

Output: experiments/consys-pwr/vmlinux.fix.bin
"""
from __future__ import annotations
import struct
from pathlib import Path

ROOT = Path(__file__).resolve().parent
VIRT_BASE = 0xC0008000
POWER_ON = 0xC059C144
POWER_ON_END = 0xC059C1BC
FUNC_SIZE = POWER_ON_END - POWER_ON            # 120

PM_RUNTIME_RESUME    = 0xC042726C
PM_GENPD_LOOKUP_NAME = 0xC042CB6C
SCPSYS_POWER_ON      = 0xC03F7BA4
PRINTK               = 0xC0981DB4
CONSYS_GLOBAL        = 0xC0F59F30
CONN_STR             = 0xC0B8ECF0
FMT = b"CFIX %x %x %x\n\0"


def le(w): return struct.pack("<I", w & 0xFFFFFFFF)
def movw(rd, imm): return le(0xE3000000 | ((imm >> 12) & 0xF) << 16 | rd << 12 | (imm & 0xFFF))
def movt(rd, imm): return le(0xE3400000 | ((imm >> 12) & 0xF) << 16 | rd << 12 | (imm & 0xFFF))
def mov_imm(rd, imm8): return le(0xE3A00000 | rd << 12 | (imm8 & 0xFF))
def mov_reg(rd, rm): return le(0xE1A00000 | rd << 12 | rm)
def add_imm(rd, rn, imm8): return le(0xE2800000 | rn << 16 | rd << 12 | (imm8 & 0xFF))
def ldr_imm(rt, rn, imm12): return le(0xE5900000 | rn << 16 | rt << 12 | (imm12 & 0xFFF))
def cmp_imm(rn, imm8): return le(0xE3500000 | rn << 16 | (imm8 & 0xFF))
def bl(pc, tgt): return le(0xEB000000 | (((tgt - (pc + 8)) >> 2) & 0xFFFFFF))
def beq(pc, tgt): return le(0x0A000000 | (((tgt - (pc + 8)) >> 2) & 0xFFFFFF))


def build():
    def emit(fmt_addr, skip_addr):
        c = bytearray()
        A = lambda: POWER_ON + len(c)
        c += le(0xE92D40F8)                    # push {r3,r4,r5,r6,r7,lr}  (r3 keeps sp 8-aligned)
        c += movw(7, CONSYS_GLOBAL & 0xFFFF)
        c += movt(7, CONSYS_GLOBAL >> 16)
        c += ldr_imm(0, 7, 4)                  # r0 = driver ctx
        c += add_imm(0, 0, 0x10)               # r0 = &dev
        c += mov_imm(1, 4)
        c += bl(A(), PM_RUNTIME_RESUME)        # pm_runtime_get_sync(dev)
        c += movw(0, CONN_STR & 0xFFFF)
        c += movt(0, CONN_STR >> 16)
        c += bl(A(), PM_GENPD_LOOKUP_NAME)     # r0 = conn genpd
        c += cmp_imm(0, 0)
        c += beq(A(), skip_addr)               # NULL -> bail
        c += mov_reg(7, 0)                     # r7 = genpd (save)
        c += bl(A(), SCPSYS_POWER_ON)          # scpsys_power_on(genpd) FORCE un-iso
        c += ldr_imm(1, 7, 0x134)              # r1 = scp
        c += ldr_imm(4, 1, 0x64C)              # r4 = SPM base
        c += ldr_imm(5, 4, 0x280)              # con
        c += ldr_imm(6, 4, 0x60C)              # sta
        c += mov_reg(1, 4)
        c += mov_reg(2, 5)
        c += mov_reg(3, 6)
        c += movw(0, fmt_addr & 0xFFFF)
        c += movt(0, fmt_addr >> 16)
        c += bl(A(), PRINTK)
        skip_here = POWER_ON + len(c)
        c += mov_imm(0, 0)
        c += le(0xE8BD80F8)                    # pop {r3,r4,r5,r6,r7,pc}
        return bytes(c), skip_here, POWER_ON + len(c)

    code, skip_here, end = emit(POWER_ON, POWER_ON)
    fmt_addr = end
    code, skip_here, end = emit(fmt_addr, skip_here)
    assert end == fmt_addr
    blob = bytearray(code + FMT)
    if len(blob) > FUNC_SIZE:
        raise SystemExit(f"fix {len(blob)} > {FUNC_SIZE}")
    blob += b"\x00" * (FUNC_SIZE - len(blob))
    return bytes(blob), fmt_addr


def main():
    src = ROOT / "vmlinux.bin"
    dst = ROOT / "vmlinux.fix.bin"
    data = bytearray(src.read_bytes())
    off = POWER_ON - VIRT_BASE
    blob, fmt_addr = build()
    data[off:off + FUNC_SIZE] = blob
    dst.write_bytes(data)
    (ROOT / "fix.patch.bin").write_bytes(blob)
    print(f"fix @{POWER_ON:#x} size={FUNC_SIZE} fmt@{fmt_addr:#x} {FMT!r}")
    print(f"patch bytes ({len(blob)}):\n{blob.hex()}")
    print(f"wrote {dst} (delta {len(data)-len(src.read_bytes())})")


if __name__ == "__main__":
    main()
