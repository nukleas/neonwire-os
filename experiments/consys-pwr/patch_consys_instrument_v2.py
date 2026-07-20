#!/usr/bin/env python3
"""Instrumentation patch v2 for DL7006 stock 3.18.35 (same site, same size, in place).

v1 proved CONN MTCMOS is fully powered (CDBG con=0x10d, PWR_STATUS&0x2 set).
v2 tests the remaining host-side suspect: the AP<->CONN TOPAXI bus protection.
If TOPAXI CONN protection is still asserted while the domain is powered, the AP
cannot read CONN_MCU chipId (0x18070008) -> reads 0 with a healthy power domain.

Replaces mtk_wcn_consys_power_on (@0xC059C144, 120 B) with:
  1. pm_runtime_get_sync(dev)  -- stock genpd/scpsys CONN power-on (unchanged), then
  2. base = __arm_ioremap(0x10001000, 0x1000, MT_DEVICE)  -- INFRACFG_AO, the SAME
     block scpsys/mtk_infracfg_*bus_protection drive (NOT a fixed virt addr; a fresh
     kernel ioremap of the phys base), read TOPAXI_PROT_EN (+0x220) and
     TOPAXI_PROT_STA1 (+0x228), and
  3. chipId = *(*(0xC0F59F30+0x28) + 8)  -- the CONN_MCU chipId read the exact way
     mtk_wcn_consys_hw_reg_ctrl does it (consys ctx +0x28 = ioremap'd CONN_MCU base).
  4. printk one line:  CTPX <PROT_EN> <PROT_STA1> <chipId>   (all hex)

Interpretation:
  PROT_STA1 & 0x104 (CONN bus-protect bits) == 0  => bus un-isolated: AP CAN reach
      CONN_MCU. If chipId here is also 0, the wall is NOT the TOPAXI bus.
  PROT_STA1 & 0x104 != 0  => CONN bus protection still asserted: AP physically blocked
      from CONN_MCU -> chipId reads 0. THAT is the wall; fix = clear TOPAXI CONN prot.

Output: experiments/consys-pwr/vmlinux.instrument.bin  (v2 REPLACES v1)
"""
from __future__ import annotations
import struct
from pathlib import Path

ROOT = Path(__file__).resolve().parent
VIRT_BASE = 0xC0008000
POWER_ON = 0xC059C144
POWER_ON_END = 0xC059C1BC
FUNC_SIZE = POWER_ON_END - POWER_ON        # 120

# verified symbol addresses (kallsyms from decompressed piggy)
PM_RUNTIME_RESUME = 0xC042726C             # __pm_runtime_resume(dev, RPM_GET_PUT)
ARM_IOREMAP       = 0xC0114784             # __arm_ioremap(phys, size, mtype)  (32-bit phys ABI)
PRINTK            = 0xC0981DB4
CONSYS_GLOBAL     = 0xC0F59F30             # ctx: +4 -> drv ctx (+0x10=&dev); +0x28 -> CONN_MCU base
INFRACFG_PHYS     = 0x10001000             # INFRACFG_AO base (TOPAXI_PROT_EN +0x220 / STA1 +0x228)
MT_DEVICE         = 0                      # ioremap() default mtype

FMT = b"CTPX %x %x %x\n\0"                 # EN, STA1, chipId


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
        c += le(0xE92D40F8)                 # push {r3,r4,r5,r6,r7,lr}  (r3 = 8-byte align)
        c += movw(7, CONSYS_GLOBAL & 0xFFFF)
        c += movt(7, CONSYS_GLOBAL >> 16)   # r7 = consys ctx global
        c += ldr_imm(0, 7, 4)               # r0 = drv ctx
        c += add_imm(0, 0, 0x10)            # r0 = &dev
        c += mov_imm(1, 4)                  # RPM_GET_PUT
        c += bl(A(), PM_RUNTIME_RESUME)     # pm_runtime_get_sync(dev)
        c += movw(0, INFRACFG_PHYS & 0xFFFF)
        c += movt(0, INFRACFG_PHYS >> 16)   # r0 = 0x10001000
        c += movw(1, 0x1000)                # r1 = size 0x1000
        c += mov_imm(2, MT_DEVICE)          # r2 = MT_DEVICE (0)
        c += bl(A(), ARM_IOREMAP)           # r0 = INFRACFG_AO virt (or NULL)
        c += cmp_imm(0, 0)
        c += beq(A(), skip_addr)
        c += ldr_imm(4, 0, 0x220)           # r4 = TOPAXI_PROT_EN
        c += ldr_imm(5, 0, 0x228)           # r5 = TOPAXI_PROT_STA1
        c += ldr_imm(0, 7, 0x28)            # r0 = CONN_MCU base = *(ctx+0x28)
        c += ldr_imm(6, 0, 8)               # r6 = chipId = *(base+8)
        c += mov_reg(1, 4)                  # printk arg1 = PROT_EN
        c += mov_reg(2, 5)                  # arg2 = PROT_STA1
        c += mov_reg(3, 6)                  # arg3 = chipId
        c += movw(0, fmt_addr & 0xFFFF)
        c += movt(0, fmt_addr >> 16)
        c += bl(A(), PRINTK)
        skip_here = POWER_ON + len(c)
        c += mov_imm(0, 0)                  # return 0 (ignored by reg_ctrl)
        c += le(0xE8BD80F8)                 # pop {r3,r4,r5,r6,r7,pc}
        return bytes(c), skip_here, POWER_ON + len(c)

    code, skip_here, end = emit(POWER_ON, POWER_ON)
    fmt_addr = end
    code, skip_here, end = emit(fmt_addr, skip_here)
    assert end == fmt_addr
    blob = bytearray(code + FMT)
    if len(blob) > FUNC_SIZE:
        raise SystemExit(f"instrument-v2 {len(blob)} > {FUNC_SIZE}")
    blob += b"\x00" * (FUNC_SIZE - len(blob))
    return bytes(blob), fmt_addr


def main():
    src = ROOT / "vmlinux.bin"
    dst = ROOT / "vmlinux.instrument.bin"
    data = bytearray(src.read_bytes())
    off = POWER_ON - VIRT_BASE
    blob, fmt_addr = build()
    data[off:off + FUNC_SIZE] = blob
    dst.write_bytes(data)
    (ROOT / "instrument.patch.bin").write_bytes(blob)
    print(f"instrument-v2 @{POWER_ON:#x} size={FUNC_SIZE} fmt@{fmt_addr:#x} {FMT!r}")
    print(f"NEW bl targets: __arm_ioremap={ARM_IOREMAP:#x}  (pm_runtime_resume="
          f"{PM_RUNTIME_RESUME:#x}, printk={PRINTK:#x})")
    print(f"patch bytes ({len(blob)}):\n{blob.hex()}")
    print(f"wrote {dst} (delta {len(data)-len(src.read_bytes())})")


if __name__ == "__main__":
    main()
