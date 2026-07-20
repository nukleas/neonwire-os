/* camprobe — SP2509 sensor liveness probe for NEONWIRE OS (plan A1).
 *
 * The 3.18 MTK kernel has no V4L2; the imgsensor driver does everything itself.
 * KDIMGSENSORIOC_T_CHECK_IS_ALIVE runs the full in-kernel sequence: board power
 * sequencing (kd_sp2509_poweron: CMPDN/CMRST GPIOs + VCAM_D2 1.8V + VCAM_A 2.8V),
 * 10ms settle, SENSOR_FEATURE_CHECK_SENSOR_ID over I2C (regs 0x02/0x03 vs 0x2509),
 * then close + power off. dmesg logs " Sensor found ID = 0x%x" on success.
 *
 * We hold /dev/camera-isp open across the probe: ISP_open -> ISP_EnableClock(TRUE)
 * gates on seninf/MCLK clocks (insurance; the sensor needs MCLK to answer I2C).
 *
 * Contract (consumed by the neonwire Camera app):
 *   exit 0, stdout "SP2509 ONLINE (drv=0xXXXXXXXX)"
 *   exit 1, stdout "SENSOR OFFLINE stage=<nodev|isp|setdriver|i2c>"
 *
 * Build (host): armv7l-linux-musleabihf-gcc -Os -static -o camprobe camprobe.c
 */
#include <stdio.h>
#include <string.h>
#include <errno.h>
#include <fcntl.h>
#include <unistd.h>
#include <sys/ioctl.h>
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
#define SENINF_MCLK_REG 0x8300u
#define MCLK1_EN_BIT 0x20000000u

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

int main(int argc, char **argv)
{
    int verbose = argc > 1 && !strcmp(argv[1], "-v");

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

    /* 1. clocks: hold the ISP device open for the whole probe */
    int isp = open("/dev/camera-isp", O_RDWR);
    if (isp < 0 && verbose)
        fprintf(stderr, "warn: /dev/camera-isp: %s (continuing)\n", strerror(errno));

    /* 2. sensor MCLK on (the step the stock HAL does from userspace). Something in
     * the in-kernel power cycle clears the bit, so keep a child re-asserting it
     * for the duration of the probe. */
    pid_t mclk_pid = -1;
    if (isp >= 0) {
        mclk1(isp, 1, verbose);
        mclk_pid = fork();
        if (mclk_pid == 0) {
            for (;;) {
                unsigned int v = MCLK1_EN_BIT;
                isp_reg(isp, ISP_WRITE_REGISTER, SENINF_MCLK_REG, &v);
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
        mclk1(isp, 0, verbose); /* leave the clock as we found it */
        close(isp);
    }

    if (alive == 0) {
        printf("SP2509 ONLINE (drv=0x%08x)\n", drv);
        return 0;
    }
    return fail(alive == -1 ? "setdriver" : "i2c");
}
