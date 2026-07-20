#!/usr/bin/env python3
"""Patch stock DigiLand 3.18.35 vmlinux: replace mtk_wcn_consys_power_on
with a direct SPM MTCMOS bring-up (bypasses broken genpd/pm_runtime).

Uses fixed MTK virtual maps (same as GPL mt_reg_base.h):
  SPM_BASE       = 0xF0006000
  SPM_CONN_PWR   = 0xF0006280
  SPM_PWR_STATUS = 0xF000660C / 0xF0006610

Input:  experiments/consys-pwr/vmlinux.bin  (decompressed piggy)
Output: experiments/consys-pwr/vmlinux.patched.bin
"""
from __future__ import annotations

import struct
from pathlib import Path

ROOT = Path(__file__).resolve().parent
VIRT_BASE = 0xC0008000

# From live kallsyms on DL7006 stock 3.18.35
POWER_ON = 0xC059C144
POWER_ON_END = 0xC059C1BC  # next: mtk_wcn_consys_power_off
FUNC_SIZE = POWER_ON_END - POWER_ON  # 0x78 = 120 bytes


def enc_movw(rd: int, imm16: int) -> bytes:
    """A2 encoding MOVW Rd, #imm16."""
    imm4 = (imm16 >> 12) & 0xF
    imm12 = imm16 & 0xFFF
    # 1110 0011 0000 imm4 Rd imm12
    return struct.pack("<I", 0xE3000000 | (imm4 << 16) | (rd << 12) | imm12)


def enc_movt(rd: int, imm16: int) -> bytes:
    imm4 = (imm16 >> 12) & 0xF
    imm12 = imm16 & 0xFFF
    return struct.pack("<I", 0xE3400000 | (imm4 << 16) | (rd << 12) | imm12)


def enc_mov_imm(rd: int, imm8: int) -> bytes:
    return struct.pack("<I", 0xE3A00000 | (rd << 12) | (imm8 & 0xFF))


def enc_ldr_imm(rt: int, rn: int, imm12: int = 0) -> bytes:
    return struct.pack("<I", 0xE5900000 | (rn << 16) | (rt << 12) | (imm12 & 0xFFF))


def enc_str_imm(rt: int, rn: int, imm12: int = 0) -> bytes:
    return struct.pack("<I", 0xE5800000 | (rn << 16) | (rt << 12) | (imm12 & 0xFFF))


def enc_orr_imm(rd: int, rn: int, imm8: int) -> bytes:
    return struct.pack("<I", 0xE3800000 | (rn << 16) | (rd << 12) | (imm8 & 0xFF))


def enc_bic_imm(rd: int, rn: int, imm8: int) -> bytes:
    return struct.pack("<I", 0xE3C00000 | (rn << 16) | (rd << 12) | (imm8 & 0xFF))


def enc_add_imm(rd: int, rn: int, imm8: int) -> bytes:
    return struct.pack("<I", 0xE2800000 | (rn << 16) | (rd << 12) | (imm8 & 0xFF))


def enc_sub_imm(rd: int, rn: int, imm8: int) -> bytes:
    return struct.pack("<I", 0xE2400000 | (rn << 16) | (rd << 12) | (imm8 & 0xFF))


def enc_cmp_imm(rn: int, imm8: int) -> bytes:
    return struct.pack("<I", 0xE3500000 | (rn << 16) | (imm8 & 0xFF))


def enc_bne(pc: int, target: int) -> bytes:
    """BNE to target; pc is address of this instruction."""
    # offset = (target - (pc + 8)) / 4
    off = (target - (pc + 8)) // 4
    assert -0x800000 <= off <= 0x7FFFFF
    return struct.pack("<I", 0x1A000000 | (off & 0xFFFFFF))


def enc_b(pc: int, target: int) -> bytes:
    off = (target - (pc + 8)) // 4
    return struct.pack("<I", 0xEA000000 | (off & 0xFFFFFF))


def enc_bx_lr() -> bytes:
    return struct.pack("<I", 0xE12FFF1E)


def enc_nop() -> bytes:
    return struct.pack("<I", 0xE320F000)


def build_shellcode() -> bytes:
    """
    Compact power-on (ARM state), must fit in 120 bytes:
      unlock SPM; PWR_ON|PWR_ON_S; short delay; clear CLK_DIS/ISO;
      set RST_B; clear SRAM_PDN; clear TOPAXI CONN protect; return 0
    """
    code = bytearray()
    base = POWER_ON

    def here() -> int:
        return base + len(code)

    # r4 = SPM_BASE 0xF0006000
    code += enc_movw(4, 0x6000)
    code += enc_movt(4, 0xF000)
    # unlock
    code += enc_movw(1, 0x0001)
    code += enc_movt(1, 0x0B16)
    code += enc_str_imm(1, 4, 0)

    # r5 = SPM_CONN_PWR_CON 0xF0006280
    code += enc_movw(5, 0x6280)
    code += enc_movt(5, 0xF000)
    code += enc_ldr_imm(3, 5, 0)
    code += enc_orr_imm(3, 3, 0x0C)  # PWR_ON|PWR_ON_S
    code += enc_str_imm(3, 5, 0)

    # short spin (no long poll — saves space)
    code += enc_movw(0, 0x4000)
    spin = here()
    code += enc_sub_imm(0, 0, 1)
    code += enc_cmp_imm(0, 0)
    code += enc_bne(here(), spin)

    # r3 = *CONN_PWR_CON; clear CLK_DIS|ISO; set RST_B; clear SRAM_PDN
    code += enc_ldr_imm(3, 5, 0)
    code += enc_bic_imm(3, 3, 0x10)  # ~PWR_CLK_DIS
    code += enc_bic_imm(3, 3, 0x02)  # ~PWR_ISO
    code += enc_orr_imm(3, 3, 0x01)  # PWR_RST_B
    code += struct.pack("<I", 0xE3C33C01)  # bic r3, r3, #0x100 (SRAM_PDN)
    code += enc_str_imm(3, 5, 0)

    # TOPAXI_PROT_EN @ 0xF0001220 clear CONN_PROT 0x104
    code += enc_movw(1, 0x1220)
    code += enc_movt(1, 0xF000)
    code += enc_ldr_imm(2, 1, 0)
    code += struct.pack("<I", 0xE3C22C01)  # bic r2, r2, #0x100
    code += enc_bic_imm(2, 2, 0x04)
    code += enc_str_imm(2, 1, 0)

    code += enc_mov_imm(0, 0)
    code += enc_bx_lr()

    if len(code) > FUNC_SIZE:
        raise SystemExit(f"shellcode {len(code)} > func size {FUNC_SIZE}")

    while len(code) + 4 <= FUNC_SIZE:
        code += enc_nop()
    while len(code) < FUNC_SIZE:
        code.append(0)
    return bytes(code)


def main() -> None:
    src = ROOT / "vmlinux.bin"
    dst = ROOT / "vmlinux.patched.bin"
    data = bytearray(src.read_bytes())
    off = POWER_ON - VIRT_BASE
    orig = bytes(data[off : off + FUNC_SIZE])
    shell = build_shellcode()
    print(f"power_on @{POWER_ON:#x} off={off:#x} size={FUNC_SIZE}")
    print(f"original: {orig[:16].hex()}...")
    print(f"shellcode {len(shell)} bytes: {shell[:32].hex()}...")
    data[off : off + FUNC_SIZE] = shell
    dst.write_bytes(data)
    (ROOT / "power_on.orig.bin").write_bytes(orig)
    (ROOT / "power_on.shell.bin").write_bytes(shell)
    print(f"wrote {dst} ({len(data)} bytes)")


if __name__ == "__main__":
    main()
