/*
 * camdump — READ-ONLY register snapshot of the MT8127 ISP/SENINF/TG/MIPI blocks.
 *
 * Purpose: capture the *working* register state while stock Android has a live
 * camera preview streaming, so we can diff it against our L1 (camgrab) state and
 * find why SENINF never clocks CAM_TG. This tool NEVER writes a register — it
 * only issues ISP_READ_REGISTER and mmap reads. Safe to run alongside the stock
 * mtkcam HAL (worst case: EBUSY on open, or stale reads — reported, not fatal).
 *
 * Build: armv7l-linux-musleabihf-gcc -Os -static -no-pie -Wall -o camdump camdump.c
 * Run:   ./camdump [outfile]     (default: stdout)
 *
 * Output is a stable, greppable "0xADDR = 0xVALUE  # label" format so a plain
 * diff against a camgrab-side dump highlights exactly which bits differ.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <errno.h>
#include <fcntl.h>
#include <unistd.h>
#include <stdint.h>
#include <sys/ioctl.h>
#include <sys/mman.h>

/* ---- ISP (magic 'k') — identical ABI to camgrab.c ---- */
#define ISP_MAGIC 'k'
typedef struct { unsigned int Addr, Val; } ISP_REG_STRUCT;
typedef struct { unsigned int Data, Count; } ISP_REG_IO_STRUCT;
#define ISP_READ_REGISTER _IOWR(ISP_MAGIC, 2, ISP_REG_IO_STRUCT)

/* ioctl register window bounds (camera_isp.c:1884): [0x4000, 0x10000) */
#define WIN_LO 0x4000u
#define WIN_HI 0x10000u

/* mmap physical blocks the stock HAL maps via ISP_mmap (pgoff = phys) */
#define IMGSYS_PHYS     0x15000000u   /* CAMINF/IMGSYS base */
#define GPIO_PHYS       0x10005000u
#define MIPIRX_ANA_PHYS 0x10010000u
#define PLL_PHYS        0x10000000u   /* APMIXED/CLK — CAMTG mux + univpll */

static int isp = -1;

static int rr(unsigned int addr, unsigned int *out)
{
    ISP_REG_STRUCT r = { addr, 0 };
    ISP_REG_IO_STRUCT io = { (unsigned int)(unsigned long)&r, 1 };
    if (ioctl(isp, ISP_READ_REGISTER, &io) < 0)
        return -1;
    *out = r.Val;
    return 0;
}

/* Named registers we care about most (from camgrab.c). Dumped first, labeled,
 * so the diff is human-readable before the raw full-window sweep. */
static const struct { unsigned int a; const char *n; } NAMED[] = {
    { 0x8000, "SENINF_TOP" },        { 0x8010, "SENINF1_CTRL" },
    { 0x8018, "SENINF1_INTEN" },     { 0x803C, "SENINF1_SIZE" },
    { 0x8020, "SENINF1_DBG0(testmodel)" }, { 0x8040, "SENINF1_DBG1(testmodel)" },
    { 0x8100, "CSI2_CTRL" },         { 0x8104, "CSI2_DELAY" },
    { 0x8108, "CSI2_INTEN" },        { 0x8128, "CSI2_LNMUX" },
    { 0x8300, "TG1_PH_CNT(PCEN|ADCLK|SEL)" }, { 0x8304, "TG1_SEN_CK" },
    { 0x8600, "NCSI2_CTL" },         { 0x8608, "NCSI2_LNRD_TIMING" },
    { 0x8614, "NCSI2_INT_EN" },      { 0x8618, "NCSI2_INT_STA" },
    { 0x8620, "NCSI2_DBG" },         { 0x862C, "NCSI2_FRAME" },
    { 0x4004, "CTL_EN1(CAM|PAK|TG1)" }, { 0x4008, "CTL_EN2(CQ0)" },
    { 0x400C, "CTL_DMA_EN" },        { 0x4010, "CTL_FMT_SEL" },
    { 0x4024, "CTL_INT_STATUS" },
    { 0x4300, "IMGO_BASE" },         { 0x4308, "IMGO_XSIZE" },
    { 0x430C, "IMGO_YSIZE" },        { 0x4310, "IMGO_STRIDE" },
    { 0x4410, "TG_SEN_MODE(CMOS_EN)" }, { 0x4414, "TG_VF_CON" },
    { 0x4418, "TG_GRAB_PXL" },       { 0x441C, "TG_GRAB_LIN" },
    { 0x4420, "TG_PATH_CFG" },
    { 0x4430, "TG_SOF_CNT  <-- 0 = dead" },
    { 0x4444, "TG_FRM_CNT" },
    { 0x4448, "TG_FRMSIZE  <-- 0 = dead" },
    { 0x444C, "TG_INTER_ST <-- SYN_VF latch" },
    { 0xC024, "MIPI_CFG_24" },       { 0xC038, "MIPI_CFG_38" },
    { 0xC03C, "MIPI_CFG_3C" },       { 0xC044, "MIPI_CFG_44" },
    { 0xC048, "MIPI_CFG_48" },
};

static void dump_named(FILE *o)
{
    unsigned int v;
    fprintf(o, "## named registers (ISP_READ_REGISTER, window [0x4000,0x10000))\n");
    for (size_t i = 0; i < sizeof(NAMED) / sizeof(NAMED[0]); i++) {
        if (rr(NAMED[i].a, &v) == 0)
            fprintf(o, "0x%04x = 0x%08x  # %s\n", NAMED[i].a, v, NAMED[i].n);
        else
            fprintf(o, "0x%04x = ????????  # %s (read failed: %s)\n",
                    NAMED[i].a, NAMED[i].n, strerror(errno));
    }
}

static void dump_sweep(FILE *o)
{
    unsigned int v, nfail = 0, n = 0;
    fprintf(o, "\n## full ioctl-window sweep 0x4000..0xFFFC (every 4 bytes)\n");
    for (unsigned int a = WIN_LO; a < WIN_HI; a += 4) {
        if (rr(a, &v) == 0) {
            fprintf(o, "0x%04x = 0x%08x\n", a, v);
            n++;
        } else {
            nfail++;
        }
    }
    fprintf(o, "## sweep done: %u read, %u failed\n", n, nfail);
}

static void dump_mmap(FILE *o, unsigned int phys, unsigned int len, const char *name)
{
    void *m = mmap(0, len, PROT_READ, MAP_SHARED, isp, phys);
    if (m == MAP_FAILED) {
        fprintf(o, "\n## mmap %s @ 0x%08x FAILED: %s\n", name, phys, strerror(errno));
        return;
    }
    volatile uint32_t *r = (volatile uint32_t *)m;
    fprintf(o, "\n## mmap %s @ phys 0x%08x, %u bytes\n", name, phys, len);
    for (unsigned int off = 0; off < len; off += 4)
        fprintf(o, "0x%08x = 0x%08x\n", phys + off, r[off / 4]);
    munmap(m, len);
}

int main(int argc, char **argv)
{
    FILE *o = stdout;
    if (argc > 1) {
        o = fopen(argv[1], "w");
        if (!o) { perror("fopen"); return 1; }
    }

    isp = open("/dev/camera-isp", O_RDWR);
    if (isp < 0) {
        fprintf(o, "!! open(/dev/camera-isp) failed: %s\n", strerror(errno));
        fprintf(o, "!! (EBUSY = stock HAL holds it exclusively; EACCES = SELinux/perm)\n");
        if (o != stdout) fclose(o);
        return 2;
    }
    fprintf(o, "# camdump — read-only ISP/SENINF/TG/MIPI snapshot\n");
    fprintf(o, "# /dev/camera-isp fd=%d\n", isp);

    dump_named(o);
    dump_sweep(o);
    /* Blocks outside the ioctl window — the HAL reaches these via ISP_mmap.
     * 4 KiB pages; small labeled slices keep the diff readable. */
    dump_mmap(o, IMGSYS_PHYS,     0x1000, "IMGSYS/CAMINF");
    dump_mmap(o, MIPIRX_ANA_PHYS, 0x1000, "MIPI-RX analog");
    dump_mmap(o, PLL_PHYS,        0x1000, "APMIXED/PLL");
    dump_mmap(o, GPIO_PHYS,       0x1000, "GPIO");

    fflush(o);
    if (o != stdout) fclose(o);
    close(isp);
    return 0;
}
