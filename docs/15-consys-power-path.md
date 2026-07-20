# CONSYS / Wi‑Fi power path — DL7006 (MT8127)

Working notes from live L1.3 Linux + stock 3.18.35 kernel.

## Goal

Get `echo 1 > /dev/wmtWifi` to create **`wlan0`**.

## What already works

```text
wmt_loader
  → registers mtk_stp_wmt (190), mtk_wmt_wifi (153), wmtdetect
  → mtk_wmtd kernel thread
mknod /dev/stpwmt /dev/wmtWifi from /sys/class/*/dev
wmt_launcher -p /tmp/fw/   (firmware copy of vendor/firmware + WMT.cfg)
```

## Failure signature

```text
[WMT-CONSYS-HW] Read CONSYS chipId(0x00000000)   # want 0x8127
[WMT-CORE] wmt_core_stp_init fail
[MTK-WIFI] WIFI_write: WMT turn on WIFI fail!
```

Clocks:

```text
/sys/kernel/debug/clk/connsys_bus/clk_enable_count = 0
/sys/kernel/debug/clk/connsys_bus/clk_rate = 0
```

Regulators (debugfs):

```text
vcn18 / vcn28 / vcn33_wifi : open_count=0 use_count=0
```

No userspace `regulator/*/enable` nodes for these LDOs.

## Hardware map

| Resource | Path / note |
|----------|-------------|
| CONSYS | `18070000.consys` driver `mtk_wmt` |
| Wi‑Fi misc | `180f0000.wifi` |
| Supplies (DT) | `vcn18`, `vcn28`, `vcn33_wifi`, `vcn33_bt` via MT6323 |
| PMIC wrap | `1000d000.pwrap` / `mt6323-pmic` |
| Bus clock | `connsys_bus` (debugfs clk) — name **`bus`** in DT `clock-names` |
| Power domain | DT `power-domains = <&scpsys 0>` — phandle **0x35** = `scpsys@10006000` |
| Reset | DT `resets` / `reset-names=connsys` |
| Config | `/vendor/firmware/WMT_SOC.cfg` → `co_clock_flag=0` |
| Live VCN state | all **disabled / off**, `num_users=0` until kernel enables |
| `/dev/mem` | **not usable** (`CONFIG_DEVMEM` effectively off — mknod fails open) |
| regmap pwrap | debugfs present but **read-only**, range effectively useless for SW poke |

### DT (from live `/sys/firmware/devicetree/.../consys@18070000`)

```text
compatible     = mediatek,mt8127-consys
status         = okay
clock-names    = bus
power-domains  = <&scpsys 0>   # scpsys phandle 0x35, domain index 0
vcn18-supply / vcn28-supply / vcn33_bt-supply / vcn33_wifi-supply
reset-names    = connsys
```

`scpsys@10006000`: `compatible=mediatek,mt8127-scpsys`, `#power-domain-cells=1`.

**Gap:** `pm_genpd` debug is empty; consys often shows `runtime_status=unsupported` or never turns VCN on. So either genpd for domain 0 isn’t CONN, or the scpsys provider didn’t register domains the way this 3.18 WMT expects.

## Reference power-on order (GPL Amazon/mt8127 tree)

File: `.../conn_soc/common/mt8127/mtk_wcn_consys_hw.c` → `mtk_wcn_consys_hw_reg_ctrl(on=1)`:

1. `hwPowerOn(VCN_1V8)`  
2. If not co-clock: HW mode + `hwPowerOn(VCN28)`  
3. `conn_power_on()` (MTCMOS via scpsys)  
4. `enable_clock(MT_CG_INFRA_CONNMCU)`  
5. Poll chip ID == `0x8127`  
6. Later: GPIO / EINT (`mtk_wcn_consys_hw_gpio_ctrl`)

Stock 3.18 uses **regulator framework + CCF + pm_runtime** instead of `hwPowerOn`/`enable_clock` names, but the **same electrical order** applies.

## Pitfalls we hit

1. **`echo on > .../power/control`** then `pm_runtime_get_sync()` returns **1** (already active) → driver logs **fail(1)** and skips real bring-up.  
2. **Unbind/rebind** left `runtime_status=unsupported` once — avoid unless you reboot.  
3. **Heavy WMT spam** can drop USB ACM briefly; shell recovers after re-enum or power cycle.

## Cold-boot capture (DigiLand screen / L1 up — **not** powered off)

Captured 2026-07-19 after clean reboot, device on logo, USB ACM up.

| Before any `wmt_loader` | Value |
|-------------------------|--------|
| `18070000.consys` `power/runtime_status` | **`unsupported`** |
| `connsys_bus` clk enable/rate | **0 / 0** |
| VCN use/open counts | **0** |
| dmesg WMT/CONSYS/Regulator_get | **empty** until userspace runs loader |

Artifacts:

- Device: `/tmp/dmesg-cold.txt`, `/mnt/sd/linux-lab/dmesg-cold.txt` (~125 KB, 1472 lines)
- Host: `reference/probe/live-coldboot/` (`state.txt`, `dmesg-head.txt`, filters)

**Conclusion from cold boot:** Kernel does **not** power CONSYS at boot. No probe-time `Regulator_get` / `cannot get clk` noise either — power is entirely lazy via WMT. RPM status **`unsupported`** means `pm_runtime_get_sync()` cannot drive a genpd for this device in the stock config as we’re seeing it; that matches later `chipId(0x00000000)` when WMT tries anyway.

### Experiments still open

- [x] Cold-boot dmesg: no early Regulator_get/clk errors; no WMT until loader  
- [x] DT: power-domains **does** point at scpsys; supplies/clocks/resets present  
- [x] VCN sysfs: all **disabled** with no `enable` node (consumer-only)  
- [x] `/dev/mem` unavailable for SPM poke (`SPM_CONN_PWR_CON` @ `0x10006280` from GPL)  
- [x] **`CONFIG_MODULES=n`** on stock 3.18.35 — `insmod` → `Function not implemented`; no `/proc/modules`  
- [x] Disassembled stock `mtk_wcn_consys_hw_reg_ctrl` @ `c059c234` (kallsyms + vmlinux piggy)  
- [ ] Confirm scpsys domain **index 0** is actually CONN on this kernel  
- [ ] Why genpd not visible / RPM unsupported despite DT power-domains  
- [x] OOT module source written: `experiments/consys-pwr/` (cannot load until MODULES=y kernel)  
- [ ] Rebuild kernel with `CONFIG_MODULES=y` and/or `CONFIG_DEVMEM=y`, or built-in power fix  
- [ ] Optional: binary-patch stock zImage pm_runtime check + SPM sequence  

### Stock code path (disassembly, 2026-07-19)

`mtk_wcn_consys_hw_reg_ctrl` → regulators → `mtk_wcn_consys_power_on`  
(`__pm_runtime_resume` / `RPM_GET_PUT`) → `clk_prepare`/`clk_enable` → chipId.

`mtk_wcn_consys_power_on` logs `pm_runtime_get_sync() fail(%d)` on **any non-zero**
return (including `1` = already active). With `runtime_status=unsupported`, genpd
never turns CONN MTCMOS on → **chipId stays 0**.

Module (blocked by MODULES=n): `experiments/consys-pwr/consys_pwr.c` does VCN +
raw SPM MTCMOS (`SPM_CONN_PWR_CON`) + `connsys_bus` + chipId poll, bypassing
broken genpd/RPM.  

### GPL register anchors (Amazon mt8127 tree)

| Symbol | Address pattern |
|--------|-----------------|
| `SPM_CONN_PWR_CON` | `SPM_BASE+0x280` ≈ `0x10006280` |
| `SPM_PWR_STATUS` | used with `CONN_PWR_STA_MASK (1<<1)` |
| Power-on bits | PWR_ON, PWR_ON_S, clear ISO/CLK_DIS, set RST_B, clear SRAM_PDN |

Without `/dev/mem` or a kernel helper, we **cannot** drive that sequence from busybox alone.


### L1.4 stock kernel binary patch (2026-07-19)

True `CONFIG_MODULES=y` rebuild needs DigiLand ALPS sources we do not have
(Amazon tree is 3.10; stock is 3.18.35). Practical path:

- **Patched** `mtk_wcn_consys_power_on` @ `c059c144` to drive SPM MTCMOS
  (`0xF0006280`) + clear TOPAXI CONN protect, return 0.
- Boot image: `experiments/linux-initramfs/out/boot-linux-l1.4-consys.img`
- Flash: `./tools/flash-linux-l1.4-consys.sh` (Preloader; offset `0x1d80000`)
- Restore: `./tools/flash-linux-l1.4-consys.sh restore`
- OOT module still at `experiments/consys-pwr/consys_pwr.ko` (built vs Amazon
  3.10 — **not** loadable on stock 3.18; kept for a future MODULES kernel)

After L1.4 boot: `/mnt/sd/linux-lab/wifi-bringup-l14.sh`

## Success criteria

```text
dmesg | grep chipId     # non-zero, ideally 0x8127
cat .../connsys_bus/clk_enable_count   # >0
echo 1 > /dev/wmtWifi
ls /sys/class/net       # includes wlan0
```
