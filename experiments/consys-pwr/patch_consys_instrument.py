#!/usr/bin/env python3
"""Instrumentation patch for DL7006 stock 3.18.35.

Replaces mtk_wcn_consys_power_on (@0xC059C144, 120 bytes, in place / same size)
with a version that:
  1. does the real pm_runtime_get_sync (__pm_runtime_resume(dev, RPM_GET_PUT))
     -- so the genpd/scpsys CONN power-on still runs exactly as stock, then
  2. looks up the CONN power domain by name (pm_genpd_lookup_name("conn")),
     derives the kernel's *live* ioremap'd SPM base (scp->base @ scp+0x64c,
     exactly as scpsys_power_on does: scp = *(genpd+0x134)), and
  3. printk's the live SPM_CONN_PWR_CON (base+0x280) and SPM_PWR_STATUS
     (base+0x60c), plus the base itself.

This tells us whether the CONN MTCMOS domain actually asserted PWR_STATUS after
the pm_runtime path -- i.e. whether "runtime_status=active" is real un-isolation
or just RPM bookkeeping. NO fixed 0xF000_xxxx addresses are used; the SPM base is
the kernel's own runtime mapping.

dmesg line to grep:  "CDBG <base> <con> <sta>"  (all hex)
Interpretation:
  con (SPM_CONN_PWR_CON @0x10006280): bit0 RST_B, bit1 ISO, bit2 PWR_ON,
      bit3 PWR_ON_2ND, bit4 CLK_DIS.  Powered+un-isolated => 0x0D (RST_B|PWR_ON|
      PWR_ON_2ND, ISO/CLK_DIS clear). If ISO(bit1)=1 or PWR_ON(bit2)=0 => NOT on.
  sta (SPM_PWR_STATUS @0x1000660c): CONN bit = bit1 (mask 0x2). bit1 set =>
      CONN domain is powered & acked. bit1 clear => domain never came up.

Output: experiments/consys-pwr/vmlinux.instrument.bin
"""
from __future__ import annotations
import struct
from pathlib import Path

ROOT = Path(__file__).resolve().parent
VIRT_BASE = 0xC0008000
POWER_ON = 0xC059C144
POWER_ON_END = 0xC059C1BC
FUNC_SIZE = POWER_ON_END - POWER_ON          # 120 = 0x78

# --- symbol addresses (from decompressed piggy kallsyms, verified) ---
PM_RUNTIME_RESUME     = 0xC042726C  # __pm_runtime_resume(dev, rpmflags)
PM_GENPD_LOOKUP_NAME  = 0xC042CB6C  # generic_pm_domain *pm_genpd_lookup_name(char*)
PRINTK                = 0xC0981DB4
CONSYS_GLOBAL         = 0xC0F59F30  # struct at +4 = ctx; ctx+0x10 = &pdev->dev
CONN_STR              = 0xC0B8ECF0  # "conn\0"

FMT = b"CDBG %x %x %x\n\0"          # printk format, lives in this function's tail


def le(w): return struct.pack("<I", w & 0xFFFFFFFF)
def movw(rd, imm): return le(0xE3000000 | ((imm >> 12) & 0xF) << 16 | rd << 12 | (imm & 0xFFF))
def movt(rd, imm): return le(0xE3400000 | ((imm >> 12) & 0xF) << 16 | rd << 12 | (imm & 0xFFF))
def mov_imm(rd, imm8): return le(0xE3A00000 | rd << 12 | (imm8 & 0xFF))
def mov_reg(rd, rm): return le(0xE1A00000 | rd << 12 | rm)
def add_imm(rd, rn, imm8): return le(0xE2800000 | rn << 16 | rd << 12 | (imm8 & 0xFF))
def ldr_imm(rt, rn, imm12): return le(0xE5900000 | rn << 16 | rt << 12 | (imm12 & 0xFFF))
def cmp_imm(rn, imm8): return le(0xE3500000 | rn << 16 | (imm8 & 0xFF))
def bl(pc, tgt):
    off = (tgt - (pc + 8)) >> 2
    return le(0xEB000000 | (off & 0xFFFFFF))
def beq(pc, tgt):
    off = (tgt - (pc + 8)) >> 2
    return le(0x0A000000 | (off & 0xFFFFFF))


def build() -> bytes:
    # Two-pass so branch/format targets resolve. Encode with placeholders first to
    # fix instruction count, then re-encode with real addresses.
    def emit(fmt_addr, skip_addr):
        c = bytearray()
        A = lambda: POWER_ON + len(c)          # current vaddr
        c += le(0xE92D40F0)                    # push {r4,r5,r6,r7,lr}
        c += movw(7, CONSYS_GLOBAL & 0xFFFF)   # r7 = &consys ctx global
        c += movt(7, CONSYS_GLOBAL >> 16)
        c += ldr_imm(0, 7, 4)                  # r0 = *(ctx+4)  (driver ctx ptr)
        c += add_imm(0, 0, 0x10)               # r0 = &pdev->dev
        c += mov_imm(1, 4)                     # r1 = RPM_GET_PUT
        c += bl(A(), PM_RUNTIME_RESUME)        # pm_runtime_get_sync(dev)  -> powers CONN
        c += movw(0, CONN_STR & 0xFFFF)        # r0 = "conn"
        c += movt(0, CONN_STR >> 16)
        c += bl(A(), PM_GENPD_LOOKUP_NAME)     # r0 = conn genpd (or NULL)
        c += cmp_imm(0, 0)
        c += beq(A(), skip_addr)               # if NULL skip the dump
        c += ldr_imm(1, 0, 0x134)              # r1 = scp = *(genpd+0x134)
        c += ldr_imm(4, 1, 0x64C)              # r4 = scp->base (ioremap'd SPM)
        c += ldr_imm(5, 4, 0x280)              # r5 = SPM_CONN_PWR_CON
        c += ldr_imm(6, 4, 0x60C)              # r6 = SPM_PWR_STATUS
        c += mov_reg(1, 4)                     # printk arg1 = base
        c += mov_reg(2, 5)                     # arg2 = con
        c += mov_reg(3, 6)                     # arg3 = sta
        c += movw(0, fmt_addr & 0xFFFF)        # r0 = fmt
        c += movt(0, fmt_addr >> 16)
        c += bl(A(), PRINTK)
        # skip:
        skip_here = POWER_ON + len(c)
        c += mov_imm(0, 0)                     # return 0 (reg_ctrl ignores it)
        c += le(0xE8BD80F0)                    # pop {r4,r5,r6,r7,pc}
        return bytes(c), skip_here, POWER_ON + len(c)

    # pass 1: dummy targets to learn code length
    code, skip_here, code_end = emit(POWER_ON, POWER_ON)
    fmt_addr = code_end                        # format string right after code
    # pass 2: real targets
    code, skip_here, code_end = emit(fmt_addr, skip_here)
    assert code_end == fmt_addr, "code length shifted between passes"
    blob = bytearray(code + FMT)
    if len(blob) > FUNC_SIZE:
        raise SystemExit(f"instrument {len(blob)} > {FUNC_SIZE}")
    blob += b"\x00" * (FUNC_SIZE - len(blob))   # pad to exact original size
    return bytes(blob), fmt_addr


def main():
    src = ROOT / "vmlinux.bin"
    dst = ROOT / "vmlinux.instrument.bin"
    data = bytearray(src.read_bytes())
    off = POWER_ON - VIRT_BASE
    orig = bytes(data[off:off + FUNC_SIZE])
    blob, fmt_addr = build()
    data[off:off + FUNC_SIZE] = blob
    dst.write_bytes(data)
    (ROOT / "instrument.patch.bin").write_bytes(blob)
    print(f"mtk_wcn_consys_power_on @{POWER_ON:#x} off={off:#x} size={FUNC_SIZE}")
    print(f"fmt string @{fmt_addr:#x} = {FMT!r}")
    print(f"orig[:16]  = {orig[:16].hex()}")
    print(f"patch[:16] = {blob[:16].hex()}")
    print(f"patch bytes ({len(blob)}):\n{blob.hex()}")
    print(f"wrote {dst} ({len(data)} bytes; delta size {len(data)-len(src.read_bytes())})")


if __name__ == "__main__":
    main()
