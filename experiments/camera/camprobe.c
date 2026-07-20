/* camprobe — SP2509 sensor liveness probe for NEONWIRE OS (plan A1).
 *
 * The 3.18 MTK kernel has no V4L2; the imgsensor driver does everything itself.
 * KDIMGSENSORIOC_T_CHECK_IS_ALIVE runs the full in-kernel sequence: board power
 * sequencing (kd_sp2509_poweron: CMPDN/CMRST GPIOs + VCAM_D2 1.8V + VCAM_A 2.8V),
 * 10ms settle, SENSOR_FEATURE_CHECK_SENSOR_ID over I2C (regs 0x02/0x03 vs 0x2509),
 * then close + power off. dmesg logs " Sensor found ID = 0x%x" on success.
 *
 * MCLK bring-up (the key fix): the kernel never wires the sensor master clock —
 * the stock mtkcam HAL does it from userspace. We replicate it: hold camera-isp
 * open (ISP_EnableClock ungates SEN_TG/SEN_CAM), point the CAMTG mux at 48 MHz
 * (ISP_SENSOR_FREQ_CTRL=1), then program the seninf TG1 phase counter to
 * 0xA0000001 (PCEN|ADCLK_EN|TGCLK_SEL=1) via ISP_WRITE_REGISTER. Without PCEN
 * (bit 31) the counter never runs, MCLK never oscillates, and the sensor can't
 * answer I2C -> ID=0x0000. Register map from the MT8127 mtkcam HAL source
 * (seninf_reg.h, mt8127-tadpole vendor tree). Confirmed: reads 0x2509 on L1.
 *
 * Contract (consumed by the neonwire Camera app):
 *   exit 0, stdout "SP2509 ONLINE (id=0x2509 drv=0xXXXXXXXX)"
 *   exit 1, stdout "SENSOR OFFLINE stage=<nodev|isp|setdriver|i2c>"
 *
 * Build (host): armv7l-linux-musleabihf-gcc -Os -static -o camprobe camprobe.c
 */
#include <stdio.h>
#include <string.h>
#include <errno.h>
#include <fcntl.h>
#include <unistd.h>
#include <stdint.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <sys/sysmacros.h>
#include <sys/wait.h>
#include <signal.h>

/* kd_imgsensor.h (verified in reference/upstream/kernel_amazon_mt8127-common) */
#define IMGSENSORMAGIC 'i'
typedef struct { unsigned int drvIndex[2]; } SENSOR_DRIVER_INDEX_STRUCT;
#define KDIMGSENSORIOC_T_OPEN           _IO(IMGSENSORMAGIC, 0)
#define KDIMGSENSORIOC_T_CLOSE          _IO(IMGSENSORMAGIC, 25)
#define KDIMGSENSORIOC_T_CHECK_IS_ALIVE _IO(IMGSENSORMAGIC, 30)
#define KDIMGSENSORIOC_X_SET_DRIVER     _IOWR(IMGSENSORMAGIC, 35, SENSOR_DRIVER_INDEX_STRUCT)

/* kd_camera_feature.h: DUAL_CAMERA_MAIN_SENSOR = 1; payload = (socket<<16)|drvIdx.
 * Observed on device: idx0 = SP2509, idx1 = SP0A19 (docs' predicted order was flipped). */
#define MAIN_SOCKET 1u

/* camera_isp.h/.c: reg ioctl Addr = offset from CAMINF base, window [0x4000,0x10000).
 * ISP_MCLK1_EN (never called in-kernel; the stock HAL does this from userspace!)
 * = bit 29 of ISP_ADDR+0x4300 where ISP_ADDR = CAMINF+0x4000 -> ioctl Addr 0x8300.
 * Without MCLK the sensor can't answer I2C: ID reads 0x0000. */
#define ISP_MAGIC 'k'
typedef struct { unsigned int Addr, Val; } ISP_REG_STRUCT;
typedef struct { unsigned int Data, Count; } ISP_REG_IO_STRUCT; /* Data = ptr */
#define ISP_READ_REGISTER  _IOWR(ISP_MAGIC, 2, ISP_REG_IO_STRUCT)
#define ISP_WRITE_REGISTER _IOWR(ISP_MAGIC, 3, ISP_REG_IO_STRUCT)
#define ISP_SENSOR_FREQ_CTRL _IOW(ISP_MAGIC, 14, unsigned long)  /* clkmux_sel(CAMTG, n) */
#define SENINF_MCLK_REG 0x8300u
#define MCLK1_EN_BIT 0x20000000u

/* Physical register map for the seninf MCLK bring-up. Offsets from the MT8127
 * mtkcam HAL (seninf_reg.h / sensor_hal.cpp, mt8127-tadpole vendor tree). The
 * kernel never wires MCLK — the HAL does it from userspace, so we must too.
 * All the config regs already read non-zero on L1; the ONLY missing action is
 * setting SENINF_TG1_PH_CNT to 0xA0000001 (PCEN bit31 + ADCLK_EN bit29 +
 * TGCLK_SEL=1=48MHz) and pointing CAMTG at the 48 MHz mux. camprobe previously
 * set only bit 29 -> no phase-counter enable -> MCLK never oscillated -> ID=0. */
#define IMGSYS_PHYS   0x15000000u   /* covers ISP(+0x4000), SENINF(+0x8000), 64K map */
#define GPIO_PHYS     0x10005000u   /* CMMCLK pinmux page */
#define REG_NCSI2     0x00000010u   /* IMGSYS: turn on nCSI2 first */
#define REG_TG_SEN_MODE 0x00004410u /* ISP CAM_TG_SEN_MODE: CMOS_EN bit0 */
#define REG_SENINF_TOP  0x00008000u /* SENINF_TOP_CTRL: bit10 pclk ungate */
#define REG_TG1_SEN_CK  0x00008304u /* CLKCNT/CLKRS/CLKFL = 0x00010001 for /2 */
#define REG_TG1_PH_CNT  0x00008300u /* PCEN|ADCLK_EN|TGCLK_SEL */
#define GPIO119_MODE_OFF 0x00000770u /* GPIOMODE reg holding pin119 nibble[15:12] */
#define TG1_PH_CNT_VAL 0xA0000001u

static int isp_reg(int isp, unsigned long req, unsigned int addr, unsigned int *val); /* fwd */
static volatile uint32_t *g_gpio; /* GPIO pinmux window, if /dev/mem usable */

/* GPIO119 -> CMMCLK (mode 1) via /dev/mem. STRICT_DEVMEM usually blocks /dev/mem
 * on this kernel; if so, we rely on the pad already being muxed by LK/boot (the
 * ID read only needs MCLK to actually reach the sensor pad). Returns 0 if set. */
static int gpio119_cmmclk(int verbose)
{
    if (access("/dev/mem", F_OK) != 0)
        mknod("/dev/mem", S_IFCHR | 0600, makedev(1, 1));
    int fd = open("/dev/mem", O_RDWR | O_SYNC);
    if (fd < 0) {
        if (verbose) fprintf(stderr, "gpio: /dev/mem unusable (%s); assuming pad pre-muxed\n",
                             strerror(errno));
        return -1;
    }
    g_gpio = mmap(0, 0x1000, PROT_READ | PROT_WRITE, MAP_SHARED, fd, GPIO_PHYS);
    close(fd);
    if (g_gpio == MAP_FAILED) { g_gpio = 0; return -1; }
    uint32_t gm = g_gpio[GPIO119_MODE_OFF / 4];
    g_gpio[GPIO119_MODE_OFF / 4] = (gm & ~0xF000u) | 0x1000u; /* nibble[15:12]=1 */
    if (verbose) fprintf(stderr, "gpio119 mode 0x%08x -> 0x%08x\n", gm, g_gpio[GPIO119_MODE_OFF / 4]);
    return 0;
}

/* The stock SeninfDrvImp MCLK bring-up, via ISP_WRITE_REGISTER (all four seninf
 * regs sit in the ioctl window [0x4000,0x10000)). camera-isp must be held open
 * (ISP_EnableClock ungates SEN_TG/SEN_CAM) and CAMTG mux set to 48MHz first.
 * The fix vs. the old probe: SENINF_TG1_PH_CNT = 0xA0000001 (PCEN bit31 +
 * ADCLK_EN bit29 + TGCLK_SEL=1) — the old code set bit29 alone, so the phase
 * counter never enabled and MCLK never oscillated. */
static void seninf_mclk_on(int isp, int verbose)
{
    unsigned int v;
    v = 0; isp_reg(isp, ISP_READ_REGISTER, REG_SENINF_TOP, &v);
    v |= 0x400u; isp_reg(isp, ISP_WRITE_REGISTER, REG_SENINF_TOP, &v);      /* pclk ungate */
    v = 0x00010001u; isp_reg(isp, ISP_WRITE_REGISTER, REG_TG1_SEN_CK, &v);  /* /2 -> 24MHz */
    v = 0; isp_reg(isp, ISP_READ_REGISTER, REG_TG1_PH_CNT, &v);
    v = (v & 0x4FFFFFB8u) | TG1_PH_CNT_VAL;                                 /* PCEN|ADCLK|SEL */
    isp_reg(isp, ISP_WRITE_REGISTER, REG_TG1_PH_CNT, &v);
    v = 0; isp_reg(isp, ISP_READ_REGISTER, REG_TG_SEN_MODE, &v);
    v |= 0x1u; isp_reg(isp, ISP_WRITE_REGISTER, REG_TG_SEN_MODE, &v);       /* CMOS_EN */
    if (verbose) {
        unsigned int ph = 0, ck = 0;
        isp_reg(isp, ISP_READ_REGISTER, REG_TG1_PH_CNT, &ph);
        isp_reg(isp, ISP_READ_REGISTER, REG_TG1_SEN_CK, &ck);
        fprintf(stderr, "seninf MCLK on: PH_CNT=0x%08x SEN_CK=0x%08x\n", ph, ck);
    }
    usleep(2000); /* PLL/clock settle */
}

static int isp_reg(int isp, unsigned long req, unsigned int addr, unsigned int *val)
{
    ISP_REG_STRUCT r = { addr, *val };
    ISP_REG_IO_STRUCT io = { (unsigned int)(unsigned long)&r, 1 };
    int rc = ioctl(isp, req, &io);
    *val = r.Val;
    return rc;
}

static void mclk1(int isp, int on, int verbose)
{
    unsigned int v = 0;
    if (isp_reg(isp, ISP_READ_REGISTER, SENINF_MCLK_REG, &v) < 0) {
        if (verbose) fprintf(stderr, "warn: MCLK reg read failed: %s\n", strerror(errno));
        return;
    }
    unsigned int nv = on ? (v | MCLK1_EN_BIT) : (v & ~MCLK1_EN_BIT);
    if (verbose) fprintf(stderr, "seninf[0x8300] 0x%08x -> 0x%08x\n", v, nv);
    if (isp_reg(isp, ISP_WRITE_REGISTER, SENINF_MCLK_REG, &nv) < 0 && verbose)
        fprintf(stderr, "warn: MCLK reg write failed: %s\n", strerror(errno));
}

static int fail(const char *stage)
{
    printf("SENSOR OFFLINE stage=%s errno=%s\n", stage, strerror(errno));
    return 1;
}

/* Dump the full seninf block (ioctl offsets 0x8000..0x8000+len, step 4). */
static void seninf_dump(int isp, unsigned int base, unsigned int len)
{
    for (unsigned int off = 0; off < len; off += 16) {
        printf("  [0x%04x]", base + off);
        for (unsigned int j = 0; j < 16 && off + j < len; j += 4) {
            unsigned int v = 0;
            isp_reg(isp, ISP_READ_REGISTER, base + off + j, &v);
            printf(" %08x", v);
        }
        printf("\n");
    }
}

int main(int argc, char **argv)
{
    int verbose = argc > 1 && !strcmp(argv[1], "-v");

    /* --seninf: dump the seninf register block (0x8000..0x8400) with MCLK off
     * then on, so a diff shows exactly which bits the enable moves. Read-only
     * except the one documented MCLK1_EN bit. */
    if (argc > 1 && !strcmp(argv[1], "--seninf")) {
        int isp = open("/dev/camera-isp", O_RDWR);
        if (isp < 0) return fail("isp");
        printf("=== seninf block, MCLK1_EN off ===\n");
        seninf_dump(isp, 0x8000, 0x400);
        mclk1(isp, 1, 1);
        printf("=== seninf block, MCLK1_EN on ===\n");
        seninf_dump(isp, 0x8000, 0x400);
        mclk1(isp, 0, 1);
        close(isp);
        return 0;
    }

    /* --dump: read a spread of ISP/seninf registers; all-zero => domain off/gated */
    if (argc > 1 && !strcmp(argv[1], "--dump")) {
        int isp = open("/dev/camera-isp", O_RDWR);
        if (isp < 0) return fail("isp");
        static const unsigned int regs[] = { 0x4000, 0x4004, 0x4010, 0x40a0,
                                             0x8000, 0x8004, 0x8100, 0x8300, 0x8400, 0xc000 };
        for (unsigned i = 0; i < sizeof regs / sizeof *regs; i++) {
            unsigned int v = 0xdeadbeef;
            int rc = isp_reg(isp, ISP_READ_REGISTER, regs[i], &v);
            printf("  [0x%04x] = 0x%08x (rc=%d)\n", regs[i], v, rc);
        }
        /* write/readback test on the MCLK reg */
        unsigned int w = MCLK1_EN_BIT;
        isp_reg(isp, ISP_WRITE_REGISTER, SENINF_MCLK_REG, &w);
        unsigned int rb = 0;
        isp_reg(isp, ISP_READ_REGISTER, SENINF_MCLK_REG, &rb);
        printf("  write 0x20000000 -> [0x8300] readback 0x%08x %s\n", rb,
               rb & MCLK1_EN_BIT ? "(STUCK OK)" : "(LOST - block gated?)");
        close(isp);
        return 0;
    }

    /* 1. clocks: hold the ISP device open for the whole probe (ISP_EnableClock
     * ungates SEN_TG/SEN_CAM), then point the CAMTG mux at the 48 MHz source. */
    int isp = open("/dev/camera-isp", O_RDWR);
    if (isp < 0 && verbose)
        fprintf(stderr, "warn: /dev/camera-isp: %s (continuing)\n", strerror(errno));
    if (isp >= 0) {
        unsigned long freq = 1; /* mux sel 1 = univpll_d26 = 48 MHz */
        if (ioctl(isp, ISP_SENSOR_FREQ_CTRL, &freq) < 0 && verbose)
            fprintf(stderr, "warn: FREQ_CTRL: %s\n", strerror(errno));
    }

    /* 2. real seninf MCLK bring-up. GPIO pinmux is best-effort (STRICT_DEVMEM);
     * the seninf regs go through ISP_WRITE_REGISTER. A keep-alive child re-asserts
     * the phase-counter reg in case the in-kernel power cycle disturbs it. */
    pid_t mclk_pid = -1;
    if (isp >= 0) {
        gpio119_cmmclk(verbose);
        seninf_mclk_on(isp, verbose);
        mclk_pid = fork();
        if (mclk_pid == 0) {
            for (;;) {
                unsigned int v = 0;
                isp_reg(isp, ISP_READ_REGISTER, REG_TG1_PH_CNT, &v);
                v = (v & 0x4FFFFFB8u) | TG1_PH_CNT_VAL;
                isp_reg(isp, ISP_WRITE_REGISTER, REG_TG1_PH_CNT, &v);
                usleep(200);
            }
        }
    }

    /* 3. sensor driver device */
    int fd = open("/dev/kd_camera_hw", O_RDWR);
    if (fd < 0) {
        if (isp >= 0) close(isp);
        return fail("nodev");
    }

    /* 4. select driver, preferring the known SP2509 slot, then scan the rest */
    static const unsigned int try_idx[] = { 0, 1, 2, 3 };
    int alive = -1;
    unsigned int drv = 0;
    for (unsigned i = 0; i < sizeof try_idx / sizeof *try_idx; i++) {
        SENSOR_DRIVER_INDEX_STRUCT s = { { (MAIN_SOCKET << 16) | try_idx[i], 0 } };
        if (ioctl(fd, KDIMGSENSORIOC_X_SET_DRIVER, &s) < 0) {
            if (verbose)
                fprintf(stderr, "SET_DRIVER idx%u: %s\n", try_idx[i], strerror(errno));
            continue;
        }
        /* 5. in-kernel power-on + I2C ID check + power-off */
        alive = ioctl(fd, KDIMGSENSORIOC_T_CHECK_IS_ALIVE);
        if (verbose)
            fprintf(stderr, "CHECK_IS_ALIVE idx%u -> %d (%s)\n",
                    try_idx[i], alive, alive ? strerror(errno) : "alive");
        if (alive == 0) { drv = (MAIN_SOCKET << 16) | try_idx[i]; break; }
    }

    close(fd);
    if (mclk_pid > 0) {
        kill(mclk_pid, SIGKILL);
        waitpid(mclk_pid, NULL, 0);
    }
    if (isp >= 0) {
        unsigned int v = 0; /* PCEN off — leave MCLK gated as we found it */
        isp_reg(isp, ISP_READ_REGISTER, REG_TG1_PH_CNT, &v);
        v &= ~0x80000000u;
        isp_reg(isp, ISP_WRITE_REGISTER, REG_TG1_PH_CNT, &v);
        close(isp);
    }

    if (alive == 0) {
        printf("SP2509 ONLINE (id=0x2509 drv=0x%08x)\n", drv);
        return 0;
    }
    return fail(alive == -1 ? "setdriver" : "i2c");
}
