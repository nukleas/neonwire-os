# Wi‑Fi / CONSYS — research findings + attack plan

**Date:** 2026-07-19
**Goal:** get `wlan0` up so the tablet is usable wirelessly (SSH in, drop the cable).
**Read with:** [15-consys-power-path.md](15-consys-power-path.md), [16-handoff-linux-consys.md](16-handoff-linux-consys.md)

## Reframe (what the research changed)

- **The kernel + DTB are byte-identical to working Android.** Our L1 boot reuses the
  stock kernel MTK blob (zImage + appended DTB). The DTB already defines
  `consys@18070000`, `mediatek,mt8127-consys`, `consys-reserve-memory`, `emi@10203000`,
  the VCN supplies, `power-domains=<&scpsys 0>`, `clock-names="bus"`. Stock boot.img
  cmdline is empty (MTK LK generates it). So **nothing hardware-described is missing.**
- **The userspace recipe is identical too.** Stock `init.connectivity.rc`:
  `wmt_loader` → `wmt_launcher -p /vendor/firmware/` → (`wlan.driver.status=ok`) →
  `write /dev/wmtWifi "1"` → `wlan0` → `wpa_supplicant -Dnl80211`. We already do this.
- **Therefore the blocker is runtime/probe *state*, not missing DT or a userspace bug.**
  Failure point (stock kernel error strings): `pm_runtime_get_sync() fail`, then
  `Read CONSYS chipId(0x00000000)`. `runtime_status=unsupported`, scpsys/genpd CONN
  domain empty in our earlier probe.
- **Prior art (honest):** postmarketOS on the same MT8127 (Amazon Fire ford/austin)
  **never got internal Wi‑Fi working** — with full custom kernels. Our edge over them:
  we run the *working vendor driver*; we only need to coax the power-on. Their fallback
  is USB networking (see RNDIS below).

## Exact power-on sequence (GPL cite + stock strings)

VCN18 → (VCN28 if `co_clock_flag=0`) → MTCMOS `conn_power_on` @ SPM `SPM_CONN_PWR_CON`
`0x10006280` → CONNMCU/`connsys_bus` clock → poll chipId @ `0x18070008` for `0x8127`.
`VCN33_WIFI` is a **later Wi‑Fi-function step**, *not* needed to read chipId — so its
`use_count=0` at the chipId-0 stage is **normal, not the fault** (corrects docs 15/16).
Canonical GPL: `reference/upstream/kernel_amazon_mt8127-common/.../mt8127/mtk_wcn_consys_hw.c`,
`arch/arm/mach-mt8127/mt_spm_mtcmos.c` (`spm_mtcmos_ctrl_connsys`). SPM writes need the
unlock `SPM_POWERON_CONFIG_SET = 0x0b160001` first.

Stock disassembly (`experiments/consys-pwr/`, live kallsyms):
- `mtk_wcn_consys_power_on` @ `c059c144` — calls pm_runtime; **logs the failure and
  continues** (does NOT abort at the `bne`). So the naive "`if(ret)`→`if(ret<0)`" flip
  is a no-op. Return value comes from a *second* PM call.
- `mtk_wcn_consys_hw_reg_ctrl` @ `c059c234` — `regulator_enable(vcn18)` (`c03fb840`),
  branches on `co_clock_en`; our board (`co_clock_flag=0`) takes the `c059c3e0` path.
  **Open item:** disassemble `c059c3e0` for the exact power_on call + confirm whether
  SPM is reached via genpd (needs patch) or is already attempted.

## ★ Key finding — the bootloop is the *repack* (recompression), not the patch

**Proven on-device 2026-07-19:** the *unpatched* control (stock kernel, only the gzip
piggy recompressed) **bootlooped** — exactly the isolation the control was for. So the
fault is the repack's recompression, not any patch.

Root cause: an ARM zImage decompressor stub has **baked-in length assumptions**
(`input_data_end`, reloc offsets) that assume the *original* piggy length. Any
different-sized piggy shifts them → decompress/relocate fails → loop. Two wrong repacks
hit this:
1. L1.4: `head + piggy + fresh_dtb`, dropped the ~1.6 KB reloc tail, set `edata` to
   total-incl-DTB.
2. First "fix": faithful in-place swap + `edata += delta` — but Python's zlib/gzip is
   **943 bytes larger** than the kernel's compressor, so the piggy size *still* changed.

**Real fix (verified):** the kernel build uses GNU **`gzip -9 -n`**, which on our exact
vmlinux reproduces the **byte-identical** original piggy (5,715,942 B). `repack_boot.py`
now recompresses with GNU gzip, pads the complete gzip member up to the exact original
length (inflate stops at the gzip end-of-stream; trailing bytes ignored), and splices
**same-size in place** — so total size, `edata`, `input_data_end`, reloc offsets, and
DTB position are all unchanged. Verified: the rebuilt **control zImage is byte-identical
to the stock kernel** (and to the booting `boot-linux-l1.img` kernel). A patch will use
the same path — same-size piggy (pad if it compresses smaller; error if larger → then
need zopfli).

## ★★ SPM addressing — the L1.4 fixed-address patch is CONFIRMED WRONG

Scanned the whole stock kernel: **zero** `movt Rd,#0xF000` instructions — it *never*
uses the fixed `0xF000_xxxx` virtual map the L1.4 shellcode assumes. `scpsys_power_on`
(`c03f7ba4`) reaches SPM via an **ioremap'd base loaded from a global struct**
(`ldr sl,[r5,#0x64c]; add ip,sl,r9; str r3,[ip]`), and you can see it set `PWR_ON`/
`PWR_ON_S` (`orr r3,lr,#4`, then `#0xC`) and poll `SPM_PWR_STATUS` at `+0x60c` — the
exact MTCMOS sequence. So:
- **The kernel ALREADY has working CONN power-on code.** Nothing needs to be written
  from scratch; the fault is that genpd/pm_runtime doesn't *reach* `scpsys_power_on`
  for the consys device.
- **Do NOT flash `consys-v2`** (fixed-addr shellcode) — it would fault on unmapped
  memory. Kept only for reference.

## Images built + ready (offline)

| Image | What | Flash |
|-------|------|-------|
| `out/boot-linux-l1.4-control.img` | **unpatched** vmlinux, corrected repack. Boots clean L1; the primary test vehicle. | `./tools/flash-wifi.sh control` |
| `out/boot-linux-l1.4-consys-v2.img` | ⚠ fixed-addr SPM patch — **addressing confirmed wrong, do not flash** | — |

## Plan (cable session; low-risk, all Preloader-reversible)

1. **`flash-wifi.sh control`** → boots to L1 ACM shell?
   - **Bootloops** → repack still wrong; fix framing (unlikely — verified offline).
   - **Boots** → the corrected repack is proven; L1.4's loop was the repack. 
2. **`net-up`** (comfort) → telnet in.
3. **`sh /mnt/sd/linux-lab/wifi-diag.sh | tee …/wifi-diag.log`** — clean WMT bring-up
   with NO power/control poking, then a full state dump. It answers the real questions:
   is `18070000.consys` driver **bound**? is the CONN **genpd domain** registered/attached?
   what does `runtime_status` read cold vs after bring-up? does binding the driver
   manually change it? what's the final `chipId`? Two outcomes:
   - `chipId=0x8127` / `wlan0` appears → **done, no patch needed** (the earlier failures
     were self-inflicted power/control pokes). Continue to `wpa_supplicant`.
   - `chipId=0` → the diag pins *where* the chain breaks. Likely fixes, in order:
     (a) manual driver **bind** enables pm_runtime; (b) a `pm_runtime` patch that, on
     get_sync failure, calls the kernel's own `scpsys_power_on` for the CONN domain
     using the **ioremap'd base** (not fixed addrs); (c) rebuild kernel (far).
4. On `wlan0`: `wpa_supplicant -Dnl80211` + `udhcpc` → wireless. Cable becomes optional.

**Ranked fallbacks:** kernel rebuild (`MODULES=y`) is far — no matching 3.18 source, and
even pmOS's kernels didn't get this Wi‑Fi up; userspace poke is dead (no `/dev/mem`).

## ★★★ LIVE DIAGNOSTIC RESULTS (2026-07-19, `wifi-diag.log`) — it's NOT a power problem

Ran `wifi-diag.sh` on the live NeonOS (no reflash). Findings, in order:
- **Cold:** consys driver *unbound*, `runtime_status=unsupported`, `connsys_bus` clk 0,
  VCN18/28/33 all `off` (correct voltages 1.8/2.8/3.6V). (`use_count` doesn't exist in
  3.18 — the field is `num_users`; `pm_genpd_summary` doesn't exist until 4.x — earlier
  "empty genpd" was a missing file, not empty domains.)
- **`wmt_loader` binds** the consys driver (`mtk_wmt`); RPM flips `unsupported→suspended`.
  So driver-bind is not the problem.
- **`mtk-scpsys` is bound** (`10006000.scpsys`). `echo on > .../consys/power/control`
  flips `runtime_status suspended→active` — the RPM/genpd resume path works.
- **During the actual bring-up (sampled live at t1):**
  `consys=active  vcn18=1:on  vcn28=1:on  connsys_bus clk=1` — **the ENTIRE power +
  clock sequence executes correctly.** Then it all powers back down when the chip-ID
  read fails.
- **The only driver error is `mtk_wcn_consys_hw_reg_ctrl(570): Read CONSYS chipId
  (0x00000000)`** — no VCN/clock/EMI/reset/TOPAXI/pinctrl error at all.

**Conclusion: the CONSYS chip is powered, clocked, and un-gated — but its digital core
stays silent (chipId 0).** This is *not* a power-sequencing bug.

Leading hypothesis: the consys device gets RPM `active` as bookkeeping, but the real
**MTCMOS CONN domain isn't actually powered** — i.e. `scpsys_power_on` either isn't
invoked for CONN (the `power-domains=<&scpsys 0>` genpd attach didn't take) or doesn't
finish the un-isolation (clear `PWR_ISO`, set `PWR_RST_B`, clear TOPAXI CONN protect).
VCN rails + `connsys_bus` are independent of that domain, so they come up regardless,
while the chip's core (inside the MTCMOS domain) stays in reset → chipId 0. This matches
pmOS's "works even with drivers, but the chip won't come up" wall.

**Next step (multi-session, now feasible with the fixed repack):** a small kernel
instrumentation patch that, after the power-up, dumps `SPM_CONN_PWR_CON` (0x10006280),
`SPM_PWR_STATUS`, and `TOPAXI_PROT_STA1` via the kernel's ioremap'd bases — to see
whether the CONN domain PWR_STATUS actually asserted and the bus is un-isolated. If not,
patch `mtk_wcn_consys_power_on` to call the kernel's own `scpsys_power_on` for CONN (via
the real ioremap base, NOT fixed addrs). The patch must compress ≤ the original piggy
(`gzip -9 -n` + pad); a tiny logging patch should fit.

**Honest status:** the power path is proven good; the remaining fault is deep silicon
un-isolation/reset state that needs kernel-side register visibility to resolve. Uncertain
payoff. The RNDIS cockpit below is the pragmatic comfort win regardless.

## ★★★★ INSTRUMENT RESULT (2026-07-19) — host power is PERFECT; ruled out

Flashed `boot-linux-wifi-instrument.img` (patched `mtk_wcn_consys_power_on` dumps live
SPM regs via the ioremap'd base from `pm_genpd_lookup_name("conn")`). During bring-up:

```
CDBG f0d6a000 10d 31cf     (SPMbase, SPM_CONN_PWR_CON, SPM_PWR_STATUS)
```
- `CON=0x10d`: RST_B=1, ISO=0, PWR_ON=1, PWR_ON_S=1, CLK_DIS=0, bit8=L1_PDN_ACK (ro).
  → domain powered, un-isolated, clocked, out of reset. `MD_SRAM_PDN` cleared.
- `PWR_STATUS 0x31cf & 0x2 = 0x2` → CONN domain powered & acked.

**Conclusion: the CONN MTCMOS domain is genuinely, fully powered — chipId=0 is NOT a
host-power problem.** Entire power/genpd/scpsys space RULED OUT. The `fix` image (force
scpsys_power_on) is moot. (Also: the fixed FNAME-padding repack booted cleanly — repack
is solved for good.)

**Remaining suspects (narrowed to two):**
1. **TOPAXI AP↔CONN bus protection** (`INFRACFG_AO+0x228`, mask `0x104`) still asserted →
   AP physically can't read CONN_MCU (`0x18070008`) → chipId 0 despite a powered domain.
   The instrument didn't capture this — it's the single leading suspect.
2. A **connsys-internal reset/clock** beyond the SPM domain (CONN_MCU's own reset/26M).

**Next: `instrument-v2`** — also dump `TOPAXI_PROT_STA1` (0x10001228) and do a direct read
of the chipId reg from inside the patch (confirm the bus, not the chip, is the wall).

## ★★★★★ INSTRUMENT-V2 RESULT — the definitive bottom (2026-07-19)

`boot-linux-wifi-instrument.img` v2 (adds INFRACFG_AO ioremap + direct chipId read). Dump:
```
CTPX 0 c0b8 0     (TOPAXI_PROT_EN, TOPAXI_PROT_STA1, chipId-direct)
```
- `PROT_EN=0` → no TOPAXI protection enabled at all.
- `PROT_STA1=0xc0b8 & 0x104 = 0` → CONN AP↔bus firewall is OPEN (AP can reach CONN).
- `chipId=0` → direct kernel read, full access, powered domain → STILL zero.

**CONCLUSION — everything host-controllable is RULED OUT** (rails, MTCMOS domain, bus clock,
TOPAXI firewall, direct AP access). The CONN_MCU core register reads 0 despite perfect
powered, un-firewalled AP access → the wall is INSIDE the connsys silicon core (its own
internal reset/clock/init not coming alive). This matches the pmOS "won't come up even with
driver+power / motherboard support issue" wall on this exact MT8127.

**Bluetooth confirmed same wall (live):** `echo 7 1 1 > /proc/driver/wmt_dbg` (WMT func_ctrl,
func 1 = BT) → `CTPX 0 c0b8 0` → `chipId 0` → `wmt_core_stp_init fail` → `opfunc_func_on(1171):
func(1) pwr_on fail(-2)`. Identical path to Wi-Fi (func 3). The combo block shares one connsys
core for WiFi/BT/FM/GPS — all four are blocked by the single silent core. No separate BT path.

**Status (instrument only): host power + TOPAXI ruled out.**  
**SUPERSEDED as final product conclusion (2026-07-19):** stock Digiland Android on **this
same unit** had working Wi‑Fi (owner used web/apps; live props `chipid=0x8127`,
`wlan.driver.status=ok`). So the silent core is **not** dead silicon — it is an **L1 vs
Android runtime delta**. Next work is the Android↔L1 bisect + undumped RGU/OSC/EMI regs,
not “park forever.” See **[19-handoff-wifi-bisect.md](19-handoff-wifi-bisect.md)**.

Wireless-adjacent still valid: RNDIS cockpit, USB Wi‑Fi dongle via rebuilt kernel. Repack
is fully solved (`repack_boot_fpad.py`) for any future kernel patching.

## RNDIS — comfortable cabled cockpit now (`experiments/net/`)

Kernel has RNDIS (`rndis_function_bind_config`); busybox has `telnetd`, `udhcpd`,
`ifconfig`, `tftp`/`ftpd`/`httpd`. `net-up.sh` reconfigures the gadget to `rndis,acm`
(keeps the ACM serial shell), gives the tablet `192.168.42.1`, serves DHCP, and starts
`telnetd` — so we `telnet 192.168.42.1` for a shell and move files at USB speed instead
of base64-over-serial. Run it **from the shell, not at boot**; a bad run is fixed by a
power-cycle (no reflash). `net-down.sh` reverts to ACM-only. Push both to
`/mnt/sd/linux-lab/` during a cable session.

**Reality check:** RNDIS is still cabled — it makes the Wi‑Fi *debugging* pleasant; the
wireless win is Wi‑Fi itself.
