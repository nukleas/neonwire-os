# consys_pwr — force CONSYS power on DL7006 (MT8127)

Tiny out-of-tree module that enables **VCN18 / VCN28 / VCN33_wifi**, runs the
**SPM MTCMOS** sequence for CONN, enables **`connsys_bus`**, and polls **chipId**.

## Blocker on stock L1 kernel

Live stock **3.18.35** (`liushen@midcompser`):

```text
insmod … → Function not implemented   # ENOSYS
/proc/modules → missing
```

**`CONFIG_MODULES=n`** — loadable modules are not supported. This `.ko` is
ready for a rebuilt kernel with `CONFIG_MODULES=y`, or the same logic can be
compiled **built-in**.

Also confirmed:

| Fact | Detail |
|------|--------|
| No `/dev/mem` | `mknod c 1 1` opens fail |
| No `/proc/kcore` | — |
| `runtime_status` | `unsupported` on `18070000.consys` |
| genpd debug | empty / missing |
| kallsyms | present (`mtk_wcn_consys_hw_reg_ctrl` @ `c059c234`) |

### Stock power path (from disassembly)

`mtk_wcn_consys_hw_reg_ctrl` (stock):

1. PMIC 0x512 + `regulator_enable(vcn18)`
2. PMIC 0x41c + `regulator_enable(vcn28)` when not co-clock
3. `mtk_wcn_consys_power_on` → `__pm_runtime_resume(..., RPM_GET_PUT)`
4. `clk_prepare` / `clk_enable` on stored clk
5. Read chip id from mapped CONN_MCU + 8

`mtk_wcn_consys_power_on` treats **any non-zero** `pm_runtime` return as
`pm_runtime_get_sync() fail(%d)` (classic MTK bug: return `1` = already
active). With `runtime_status=unsupported`, genpd never powers CONN, so
chipId stays **0** even if logs look busy.

## Build (once MODULES=y kernel exists)

Host cross-compiler (already downloaded, no root):

```bash
export CROSS_COMPILE=$HOME/toolchains/armv7l-linux-musleabihf-cross/bin/armv7l-linux-musleabihf-
export ARCH=arm
# Point KDIR at a 3.18 tree after: make modules_prepare
export KDIR=…/prepared-3.18-tree
make -C experiments/consys-pwr
```

Push `consys_pwr.ko` to SD, then on device:

```sh
insmod /mnt/sd/linux-lab/consys_pwr.ko
dmesg | grep consys_pwr
# expect chipId=0x8127
# then normal WMT path:
#   wmt_loader / wmt_launcher / echo 1 > /dev/wmtWifi
```

## Next paths (pick one)

1. **Rebuild kernel** with `CONFIG_MODULES=y` and optionally `CONFIG_DEVMEM=y`
   (best lab flexibility). Need a bootable 3.18 tree + DigiLand DT/boot.img.
2. **Built-in** the same `consys_pwr_on()` as `late_initcall` in a custom zImage.
3. **Binary-patch** stock zImage: fix `if (pm_runtime_get_sync())` → `< 0` and/or
   inject SPM sequence (addresses known; high risk).

## Addresses (physical)

| Resource | Address |
|----------|---------|
| SPM | `0x10006000` |
| `SPM_CONN_PWR_CON` | `0x10006280` |
| `SPM_PWR_STATUS` | `0x1000660c` |
| INFRACFG_AO | `0x10001000` |
| CONN_MCU / chipId | `0x18070000` / `+0x8` |
