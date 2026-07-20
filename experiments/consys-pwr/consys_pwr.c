/*
 * consys_pwr — force CONSYS rails + MTCMOS + bus clock on MT8127
 *
 * Order matches GPL Amazon mt8127 mtk_wcn_consys_hw_reg_ctrl /
 * spm_mtcmos_ctrl_connsys, and the stock DigiLand 3.18.35 sequence
 * recovered from kallsyms + disassembly:
 *
 *   1. regulator_enable(vcn18)
 *   2. regulator_enable(vcn28)   [co_clock_flag=0 on this board]
 *   3. optional vcn33_wifi
 *   4. SPM MTCMOS (SPM_CONN_PWR_CON @ 0x10006280)
 *   5. clk_prepare_enable(connsys_bus)
 *   6. read chip id @ CONN_MCU + 0x8  (expect 0x8127)
 *
 * Stock kernel ships CONFIG_MODULES=n, so this .ko cannot load until
 * a rebuilt kernel enables modules (or the same code is built-in).
 *
 * Module params:
 *   do_power=1   (default) run power-on at init
 *   co_clock=0   (default) match WMT_SOC.cfg co_clock_flag=0
 */

#include <linux/module.h>
#include <linux/kernel.h>
#include <linux/init.h>
#include <linux/delay.h>
#include <linux/io.h>
#include <linux/clk.h>
#include <linux/regulator/consumer.h>
#include <linux/of.h>
#include <linux/platform_device.h>
#include <linux/slab.h>

#define DRV "consys_pwr"

/* Physical addresses (MT8127 / DigiLand DT) */
#define SPM_PHYS		0x10006000
#define SPM_SIZE		0x1000
#define SPM_POWERON_CONFIG_SET	0x0000
#define SPM_CONN_PWR_CON	0x0280
#define SPM_PWR_STATUS		0x060c
#define SPM_PWR_STATUS_S	0x0610

#define INFRACFG_AO_PHYS	0x10001000
#define INFRACFG_AO_SIZE	0x1000
#define TOPAXI_PROT_EN		0x0220
#define TOPAXI_PROT_STA1	0x0228

#define CONN_MCU_PHYS		0x18070000
#define CONN_MCU_SIZE		0x1000
#define CONSYS_CHIP_ID_OFF	0x0008

/* SPM unlock / project code (same as CONSYS_PWRON_CONFG_EN_VALUE) */
#define SPM_PROJECT_CODE	0x0b160001

/* PWR_CON bits (mt_spm_mtcmos.c) */
#define PWR_RST_B		(1U << 0)
#define PWR_ISO			(1U << 1)
#define PWR_ON			(1U << 2)
#define PWR_ON_S		(1U << 3)
#define PWR_CLK_DIS		(1U << 4)
#define MD_SRAM_PDN		(1U << 8)

#define CONN_PWR_STA_MASK	(1U << 1)
#define CONN_PROT_MASK		0x0104

static int do_power = 1;
module_param(do_power, int, 0644);
MODULE_PARM_DESC(do_power, "1 = power CONSYS on init (default)");

static int co_clock;
module_param(co_clock, int, 0644);
MODULE_PARM_DESC(co_clock, "1 = co-clock (skip VCN28 SW enable)");

static void __iomem *spm_base;
static void __iomem *infracfg_base;
static void __iomem *conn_base;
static struct regulator *reg_vcn18;
static struct regulator *reg_vcn28;
static struct regulator *reg_vcn33;
static struct clk *clk_connsys;

static inline u32 spm_read(u32 off)
{
	return readl(spm_base + off);
}

static inline void spm_write(u32 off, u32 val)
{
	writel(val, spm_base + off);
	/* mild barrier; MTK uses mt_reg_sync_writel */
	wmb();
}

static int mtcmos_connsys_on(void)
{
	unsigned int val;
	int timeout;

	/* Unlock SPM register writes */
	spm_write(SPM_POWERON_CONFIG_SET, SPM_PROJECT_CODE);

	pr_info(DRV ": SPM_CONN_PWR_CON before = 0x%08x\n",
		spm_read(SPM_CONN_PWR_CON));
	pr_info(DRV ": SPM_PWR_STATUS       = 0x%08x\n",
		spm_read(SPM_PWR_STATUS));
	pr_info(DRV ": SPM_PWR_STATUS_S     = 0x%08x\n",
		spm_read(SPM_PWR_STATUS_S));

	/* Power-on sequence (spm_mtcmos_ctrl_connsys STA_POWER_ON) */
	spm_write(SPM_CONN_PWR_CON, spm_read(SPM_CONN_PWR_CON) | PWR_ON);
	spm_write(SPM_CONN_PWR_CON, spm_read(SPM_CONN_PWR_CON) | PWR_ON_S);

	timeout = 1000;
	while ((!(spm_read(SPM_PWR_STATUS) & CONN_PWR_STA_MASK) ||
		!(spm_read(SPM_PWR_STATUS_S) & CONN_PWR_STA_MASK)) &&
	       --timeout > 0)
		udelay(10);

	if (timeout <= 0) {
		pr_err(DRV ": MTCMOS ACK timeout sta=0x%08x sta_s=0x%08x\n",
		       spm_read(SPM_PWR_STATUS), spm_read(SPM_PWR_STATUS_S));
		return -ETIMEDOUT;
	}

	spm_write(SPM_CONN_PWR_CON, spm_read(SPM_CONN_PWR_CON) & ~PWR_CLK_DIS);
	spm_write(SPM_CONN_PWR_CON, spm_read(SPM_CONN_PWR_CON) & ~PWR_ISO);
	spm_write(SPM_CONN_PWR_CON, spm_read(SPM_CONN_PWR_CON) | PWR_RST_B);
	spm_write(SPM_CONN_PWR_CON, spm_read(SPM_CONN_PWR_CON) & ~MD_SRAM_PDN);

	/* Clear AXI protect */
	if (infracfg_base) {
		val = readl(infracfg_base + TOPAXI_PROT_EN);
		writel(val & ~CONN_PROT_MASK, infracfg_base + TOPAXI_PROT_EN);
		wmb();
		timeout = 1000;
		while ((readl(infracfg_base + TOPAXI_PROT_STA1) & CONN_PROT_MASK) &&
		       --timeout > 0)
			udelay(10);
	}

	pr_info(DRV ": SPM_CONN_PWR_CON after  = 0x%08x\n",
		spm_read(SPM_CONN_PWR_CON));
	pr_info(DRV ": MTCMOS on OK (timeout leftover %d)\n", timeout);
	return 0;
}

static int enable_reg(struct regulator **rp, const char *name)
{
	struct regulator *r;
	int ret;

	r = regulator_get(NULL, name);
	if (IS_ERR(r)) {
		pr_err(DRV ": regulator_get(%s) = %ld\n", name, PTR_ERR(r));
		return PTR_ERR(r);
	}
	ret = regulator_enable(r);
	if (ret) {
		pr_err(DRV ": regulator_enable(%s) = %d\n", name, ret);
		regulator_put(r);
		return ret;
	}
	pr_info(DRV ": %s enabled\n", name);
	*rp = r;
	return 0;
}

static u32 read_chip_id(void)
{
	if (!conn_base)
		return 0;
	return readl(conn_base + CONSYS_CHIP_ID_OFF);
}

static int consys_pwr_on(void)
{
	int ret;
	u32 chip;
	int retry;

	/* 1–2. VCN rails */
	ret = enable_reg(&reg_vcn18, "vcn18");
	if (ret)
		pr_warn(DRV ": continuing without vcn18 (%d)\n", ret);

	udelay(150);

	if (!co_clock) {
		ret = enable_reg(&reg_vcn28, "vcn28");
		if (ret)
			pr_warn(DRV ": continuing without vcn28 (%d)\n", ret);
	}

	/* optional Wi-Fi LDO */
	ret = enable_reg(&reg_vcn33, "vcn33_wifi");
	if (ret)
		pr_warn(DRV ": vcn33_wifi skip (%d)\n", ret);

	/* 3. MTCMOS */
	ret = mtcmos_connsys_on();
	if (ret)
		return ret;

	udelay(10);

	/* 4. Bus clock */
	clk_connsys = clk_get(NULL, "connsys_bus");
	if (IS_ERR(clk_connsys)) {
		/* try DT-style name used by some trees */
		clk_connsys = clk_get(NULL, "bus");
	}
	if (IS_ERR(clk_connsys)) {
		pr_err(DRV ": clk_get(connsys_bus) = %ld\n",
		       PTR_ERR(clk_connsys));
		clk_connsys = NULL;
	} else {
		ret = clk_prepare_enable(clk_connsys);
		if (ret) {
			pr_err(DRV ": clk_prepare_enable = %d\n", ret);
			clk_put(clk_connsys);
			clk_connsys = NULL;
		} else {
			pr_info(DRV ": connsys_bus clock enabled\n");
		}
	}

	/* 5. Poll chip id */
	for (retry = 0; retry < 10; retry++) {
		chip = read_chip_id();
		pr_info(DRV ": chipId try %d = 0x%08x\n", retry, chip);
		if (chip == 0x8127 || chip == 0x8163 ||
		    chip == 0x6582 || chip == 0x6572 || chip == 0x337)
			return 0;
		msleep(20);
	}

	pr_err(DRV ": chipId never valid (last 0x%08x)\n", chip);
	return -ENODEV;
}

static void consys_pwr_cleanup(void)
{
	if (clk_connsys) {
		clk_disable_unprepare(clk_connsys);
		clk_put(clk_connsys);
		clk_connsys = NULL;
	}
	if (reg_vcn33) {
		regulator_disable(reg_vcn33);
		regulator_put(reg_vcn33);
		reg_vcn33 = NULL;
	}
	if (reg_vcn28) {
		regulator_disable(reg_vcn28);
		regulator_put(reg_vcn28);
		reg_vcn28 = NULL;
	}
	if (reg_vcn18) {
		regulator_disable(reg_vcn18);
		regulator_put(reg_vcn18);
		reg_vcn18 = NULL;
	}
	if (conn_base) {
		iounmap(conn_base);
		conn_base = NULL;
	}
	if (infracfg_base) {
		iounmap(infracfg_base);
		infracfg_base = NULL;
	}
	if (spm_base) {
		iounmap(spm_base);
		spm_base = NULL;
	}
}

static int __init consys_pwr_init(void)
{
	int ret = 0;

	pr_info(DRV ": init (do_power=%d co_clock=%d)\n", do_power, co_clock);

	spm_base = ioremap(SPM_PHYS, SPM_SIZE);
	if (!spm_base) {
		pr_err(DRV ": ioremap SPM failed\n");
		return -ENOMEM;
	}
	infracfg_base = ioremap(INFRACFG_AO_PHYS, INFRACFG_AO_SIZE);
	if (!infracfg_base)
		pr_warn(DRV ": ioremap INFRACFG_AO failed (prot skip)\n");

	conn_base = ioremap(CONN_MCU_PHYS, CONN_MCU_SIZE);
	if (!conn_base)
		pr_warn(DRV ": ioremap CONN_MCU failed (no chipId)\n");

	if (do_power) {
		ret = consys_pwr_on();
		if (ret)
			pr_err(DRV ": power-on finished with %d "
			       "(module stays loaded for debug)\n", ret);
		else
			pr_info(DRV ": power-on SUCCESS chipId=0x%08x\n",
				read_chip_id());
	}

	/* Keep mappings even on soft failure so dmesg is useful.
	 * Return 0 so the module stays loaded for rmmod/cleanup.
	 */
	return 0;
}

static void __exit consys_pwr_exit(void)
{
	pr_info(DRV ": exit\n");
	consys_pwr_cleanup();
}

module_init(consys_pwr_init);
module_exit(consys_pwr_exit);

MODULE_LICENSE("GPL");
MODULE_AUTHOR("DL7006 linux lab");
MODULE_DESCRIPTION("Force MT8127 CONSYS VCN+MTCMOS+clk for Wi-Fi bring-up");
MODULE_VERSION("0.1");
