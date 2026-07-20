/* camgrab — capture one raw Bayer frame from the SP2509 on L1 (plan A3).
 *
 * Assembles proven pieces (camprobe MCLK + ion buffer) with the ISP pass1
 * capture path reverse-engineered from the MT8127 mtkcam HAL source
 * (seninf_reg.h / isp_reg.h, mt8127-tadpole vendor tree) + the local kernel
 * tree. Dumps the IMGO DMA output to a file for host debayer.py.
 *
 * SAFETY: the IMGO DMA writes to an M4U MVA. That MVA only translates to our
 * ion buffer AFTER MTK_M4U_T_CONFIG_PORT(CAM_IMGO, Virtuality=1). If the port
 * config fails we ABORT before enabling the DMA — otherwise the engine would
 * emit the MVA as a raw physical address into low memory and corrupt the kernel.
 *
 * ISP reg Addr = byte offset from CAMINF base 0x15000000 (ISP_WRITE_REGISTER
 * window [0x4000,0x10000)). ISP block @0x4000, SENINF @0x8000, MIPIRX-cfg @0xC000.
 * MIPIRX-analog @0x10010000 needs mmap (ISP_mmap physical passthrough).
 *
 * Build: armv7l-linux-musleabihf-gcc -Os -static -no-pie -o camgrab camgrab.c
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

/* ---- imgsensor (magic 'i') ---- */
#define IMGSENSORMAGIC 'i'
typedef struct { unsigned int drvIndex[2]; } SENSOR_DRIVER_INDEX_STRUCT;
typedef struct { unsigned int cam, scenario; void *win, *cfg; } ACDK_SENSOR_CONTROL_STRUCT;
#define IOC_T_OPEN     _IO(IMGSENSORMAGIC, 0)
#define IOC_X_CONTROL  _IOWR(IMGSENSORMAGIC, 20, ACDK_SENSOR_CONTROL_STRUCT)
#define IOC_T_CLOSE    _IO(IMGSENSORMAGIC, 25)
#define IOC_X_SET_DRIVER _IOWR(IMGSENSORMAGIC, 35, SENSOR_DRIVER_INDEX_STRUCT)
#define MAIN_SOCKET 1u

/* ---- ISP (magic 'k') ---- */
#define ISP_MAGIC 'k'
typedef struct { unsigned int Addr, Val; } ISP_REG_STRUCT;
typedef struct { unsigned int Data, Count; } ISP_REG_IO_STRUCT;
typedef struct { unsigned int Clear, Type, Status, Timeout; } ISP_WAIT_IRQ_STRUCT;
#define ISP_READ_REGISTER  _IOWR(ISP_MAGIC, 2, ISP_REG_IO_STRUCT)
#define ISP_WRITE_REGISTER _IOWR(ISP_MAGIC, 3, ISP_REG_IO_STRUCT)
#define ISP_WAIT_IRQ       _IOW (ISP_MAGIC, 6, ISP_WAIT_IRQ_STRUCT)
#define ISP_SENSOR_FREQ_CTRL _IOW(ISP_MAGIC, 14, unsigned long)
#define IRQ_CLEAR_WAIT 1
#define IRQ_TYPE_INT   0
#define PASS1_TG1_DON  (1u << 10)

/* seninf MCLK (camprobe) */
#define R_SENINF_TOP   0x8000u
#define R_SENINF1_CTRL 0x8010u
#define R_CSI2_CTRL    0x8100u
#define R_CSI2_DELAY   0x8104u
#define R_CSI2_INTEN   0x8108u
#define R_CSI2_LNMUX   0x8128u
#define R_TG1_SEN_CK   0x8304u
#define R_TG1_PH_CNT   0x8300u
#define TG1_PH_VAL     0xA0000001u

/* ISP pass1 (offsets verified from isp_reg.h) */
#define R_CTL_EN1      0x4004u   /* TG1_EN:0, CAM_EN:30 */
#define R_CTL_DMA_EN   0x400Cu   /* IMGO_EN:0 */
#define R_CTL_FMT_SEL  0x4010u
#define R_FMT_SEL_SET  0x4098u
#define R_FMT_SEL_CLR  0x409Cu
#define R_TG_SEN_MODE  0x4410u   /* CMOS_EN:0 */
#define R_TG_VF_CON    0x4414u   /* VFDATA_EN */
#define R_TG_GRAB_PXL  0x4418u   /* PXL_S:0-14, PXL_E:16-30 */
#define R_TG_GRAB_LIN  0x441Cu   /* LIN_S:0-12, LIN_E:16-28 */
#define R_TG_PATH_CFG  0x4420u
#define R_IMGO_BASE    0x4300u
#define R_IMGO_XSIZE   0x4308u   /* bytes-1 */
#define R_IMGO_YSIZE   0x430Cu   /* lines-1 */
#define R_IMGO_STRIDE  0x4310u   /* bytes/line */

#define GRAB_W 1600
#define GRAB_H 1200
#define LINE_BYTES (GRAB_W * 10 / 8)          /* 2000 */
#define FRAME_LEN  (LINE_BYTES * GRAB_H)      /* 2,400,000 */
#define MIPIRX_ANA_PHYS 0x10010000u

/* ---- ion (magic 'I') ---- */
#define ION_MAGIC 'I'
struct ion_alloc { size_t len, align; unsigned int heap_mask, flags; int handle; };
struct ion_fd { int handle, fd; };
struct ion_hd { int handle; };
struct ion_custom { unsigned int cmd; unsigned long arg; };
#define ION_ALLOC  _IOWR(ION_MAGIC, 0, struct ion_alloc)
#define ION_MAP    _IOWR(ION_MAGIC, 2, struct ion_fd)
#define ION_CUSTOM _IOWR(ION_MAGIC, 6, struct ion_custom)
#define ION_MM_HEAP_MASK (1u << 10)
#define ION_CMD_SYSTEM 0
#define ION_CMD_MM     1
#define ION_MM_CONFIG  0
#define ION_SYS_PHYS   1
#define ION_SYS_CACHE_SYNC 0
#define ION_CACHE_INVALID_BY_RANGE 1
typedef struct { unsigned int cmd; int handle; unsigned int phys, len; unsigned char pad[240]; } sys_data_t;
typedef struct { unsigned int cmd; int handle; void *va; unsigned int size, sync; unsigned char pad[232]; } sync_data_t;
typedef struct { unsigned int cmd; int handle; int module; unsigned int sec, coh; unsigned char pad[236]; } mm_data_t;

/* ---- M4U (/proc/M4U_device, magic 'g') ---- */
#define M4U_MAGIC 'g'
typedef struct { int ePortID; unsigned int Virtuality, Security, domain, Distance, Direction; } M4U_PORT_STRUCT;
#define MTK_M4U_T_CONFIG_PORT _IOW(M4U_MAGIC, 11, int)
#define CAM_IMGO_PORT 17

static int isp, sensor, ionfd;

static int rw(unsigned int addr, unsigned int val)
{
    ISP_REG_STRUCT r = { addr, val };
    ISP_REG_IO_STRUCT io = { (unsigned int)(unsigned long)&r, 1 };
    return ioctl(isp, ISP_WRITE_REGISTER, &io);
}
static unsigned int rr(unsigned int addr)
{
    ISP_REG_STRUCT r = { addr, 0 };
    ISP_REG_IO_STRUCT io = { (unsigned int)(unsigned long)&r, 1 };
    ioctl(isp, ISP_READ_REGISTER, &io);
    return r.Val;
}

static void mclk_on(void)
{
    unsigned long f = 1;
    ioctl(isp, ISP_SENSOR_FREQ_CTRL, &f);
    rw(R_SENINF_TOP, rr(R_SENINF_TOP) | 0x400u);
    rw(R_TG1_SEN_CK, 0x00010001u);
    rw(R_TG1_PH_CNT, (rr(R_TG1_PH_CNT) & 0x4FFFFFB8u) | TG1_PH_VAL);
    rw(R_TG_SEN_MODE, rr(R_TG_SEN_MODE) | 0x1u);
    usleep(2000);
}

/* CSI-2 MIPI analog calibration (initTg1CSI2) via mmap'd 0x10010000 block.
 * Registers indexed at map_base + 0x800 + off (seninf_drv.cpp convention). */
static void csi2_analog(void)
{
    void *m = mmap(0, 0x1000, PROT_READ | PROT_WRITE, MAP_SHARED, isp, MIPIRX_ANA_PHYS);
    if (m == MAP_FAILED) { fprintf(stderr, "warn: analog mmap: %s\n", strerror(errno)); return; }
    volatile uint32_t *a = (volatile uint32_t *)((char *)m + 0x800);
#define A(o) a[(o) / 4]
    A(0x48) &= 0xFFFFFC3Fu; A(0x4C) &= 0xFEFBEFBEu; A(0x50) &= 0xFFFFFFFEu;
    A(0x00) |= 0x08; A(0x04) |= 0x08; A(0x08) |= 0x08; A(0x0C) |= 0x08; A(0x10) |= 0x08;
    A(0x24) |= 0x03;   /* BG core */
    A(0x20) |= 0x01;   /* LDO core */
    A(0x00) |= 0x01;   /* LNRC LDO out */
    /* MIPIRX-config writes are in-window (0xC024/0xC038/0xC03C) */
    rw(0xC024, rr(0xC024) | 0x00080000u);   /* HW_CAL_START */
    rw(0xC038, rr(0xC038) | 0x1u);          /* SW_CTRL_MODE */
    rw(0xC03C, 0x1541u);
    rw(0xC038, rr(0xC038) | 0x4u);          /* cal start */
    usleep(1000);
    rw(0xC038, rr(0xC038) & 0xFFFFFFFEu);
    A(0x20) |= 0x02;
    rw(R_CSI2_LNMUX, 0xE4u);
    /* readback: do writes to the analog block actually stick? (power domain check) */
    fprintf(stderr, "  analog readback: [0x00]=0x%08x [0x20]=0x%08x [0x24]=0x%08x [0x48]=0x%08x\n",
            A(0x00), A(0x20), A(0x24), A(0x48));
#undef A
    munmap(m, 0x1000);
}

/* CSI-2 digital receive (setTg1CSI2), 1 lane, SettleDelay 14, BGGR */
static void csi2_digital(void)
{
    rw(R_SENINF1_CTRL, rr(R_SENINF1_CTRL) & 0xFFFF0FFFu);
    rw(R_CSI2_DELAY, (14u & 0xFF) << 16);   /* dataSettleDelay */
    rw(R_CSI2_INTEN, rr(R_CSI2_INTEN) | 0x7u);
    /* CTRL = bit10 | headerOrder<<5 | bit4 | lanes | en. 1 lane -> lane field 0x2 */
    rw(R_CSI2_CTRL, 0x400u | (1u << 5) | 0x10u | 0x2u | 0x1u);
    rw(R_SENINF1_CTRL, (rr(R_SENINF1_CTRL) & 0xFFFF0FFFu) | 0x3u);
}

/* TG1 grab window + input cfg (MIPI, inSrcTypeSel=8) */
static void tg_config(void)
{
    rw(R_TG_GRAB_PXL, (GRAB_W << 16) | 0);
    rw(R_TG_GRAB_LIN, (GRAB_H << 16) | 0);
    rw(R_TG_SEN_MODE, rr(R_TG_SEN_MODE) | 0x5u);        /* CMOS_EN + SOT_MODE */
    rw(R_SENINF1_CTRL, rr(R_SENINF1_CTRL) | 0x80000000u); /* input src enable */
    rw(R_TG_PATH_CFG, rr(R_TG_PATH_CFG) & 0xFFFFFFFCu);   /* SEN_IN_LSB=0 */
    rw(R_FMT_SEL_CLR, 0x70000u);                          /* clear TG1_FMT */
    rw(R_FMT_SEL_SET, 0x0u);                              /* TG1_FMT=0 (raw) */
}

static unsigned int ion_buf(void **va, int *mapfd, int *handle)
{
    struct ion_alloc a = { .len = FRAME_LEN, .align = 0x1000, .heap_mask = ION_MM_HEAP_MASK };
    if (ioctl(ionfd, ION_ALLOC, &a) < 0) { perror("ion alloc"); return 0; }
    *handle = a.handle;
    mm_data_t mm = { .cmd = ION_MM_CONFIG, .handle = a.handle, .module = CAM_IMGO_PORT };
    struct ion_custom c1 = { ION_CMD_MM, (unsigned long)&mm };
    ioctl(ionfd, ION_CUSTOM, &c1);
    sys_data_t sd = { .cmd = ION_SYS_PHYS, .handle = a.handle };
    struct ion_custom c2 = { ION_CMD_SYSTEM, (unsigned long)&sd };
    if (ioctl(ionfd, ION_CUSTOM, &c2) < 0) { perror("ion get_phys"); return 0; }
    struct ion_fd mf = { .handle = a.handle };
    ioctl(ionfd, ION_MAP, &mf);
    *va = mmap(0, FRAME_LEN, PROT_READ | PROT_WRITE, MAP_SHARED, mf.fd, 0);
    *mapfd = mf.fd;
    return sd.phys;
}

int main(int argc, char **argv)
{
    const char *out = argc > 1 ? argv[1] : "/tmp/frame.raw";

    isp = open("/dev/camera-isp", O_RDWR);
    sensor = open("/dev/kd_camera_hw", O_RDWR);
    ionfd = open("/dev/ion", O_RDWR);
    if (isp < 0 || sensor < 0 || ionfd < 0) { fprintf(stderr, "open nodes failed\n"); return 1; }

    /* 1. MCLK + sensor open + stream (preview scenario) */
    mclk_on();
    SENSOR_DRIVER_INDEX_STRUCT s = { { (MAIN_SOCKET << 16) | 0, 0 } };
    ioctl(sensor, IOC_X_SET_DRIVER, &s);
    if (ioctl(sensor, IOC_T_OPEN) < 0) fprintf(stderr, "warn: T_OPEN: %s\n", strerror(errno));
    static unsigned char win[256], cfg[512];
    ACDK_SENSOR_CONTROL_STRUCT ctl = { MAIN_SOCKET, 0, win, cfg };
    if (ioctl(sensor, IOC_X_CONTROL, &ctl) < 0)
        fprintf(stderr, "warn: X_CONTROL(preview): %s\n", strerror(errno));
    usleep(50000);

    /* 2. CSI-2 receive */
    csi2_analog();
    csi2_digital();
    tg_config();

    /* 3. DMA buffer + M4U port config (SAFETY GATE) */
    void *va; int mapfd, handle;
    unsigned int mva = ion_buf(&va, &mapfd, &handle);
    if (!mva || va == MAP_FAILED) { fprintf(stderr, "buffer alloc failed\n"); return 1; }
    fprintf(stderr, "ion MVA=0x%08x len=%d\n", mva, FRAME_LEN);
    memset(va, 0, FRAME_LEN);

    int m4u = open("/proc/M4U_device", O_RDONLY);
    if (m4u < 0) { fprintf(stderr, "ABORT: no /proc/M4U_device (%s) — refusing DMA\n", strerror(errno)); return 1; }
    M4U_PORT_STRUCT port = { CAM_IMGO_PORT, 1, 0, 0, 1, 0 };
    if (ioctl(m4u, MTK_M4U_T_CONFIG_PORT, &port) < 0) {
        fprintf(stderr, "ABORT: M4U CONFIG_PORT failed (%s) — refusing DMA (MVA would be raw phys)\n",
                strerror(errno));
        return 1;
    }
    fprintf(stderr, "M4U CAM_IMGO port -> virtual OK\n");

    /* 4. IMGO DMA target */
    rw(R_IMGO_BASE, mva);
    rw(R_IMGO_XSIZE, LINE_BYTES - 1);
    rw(R_IMGO_YSIZE, GRAB_H - 1);
    rw(R_IMGO_STRIDE, LINE_BYTES);
    rw(R_CTL_DMA_EN, rr(R_CTL_DMA_EN) | 0x1u);          /* IMGO_EN */
    rw(R_CTL_EN1, rr(R_CTL_EN1) | 0x40000001u);         /* TG1_EN | CAM_EN */

    fprintf(stderr, "pre-VF: EN1=0x%08x DMA_EN=0x%08x FMT=0x%08x CSI2_CTRL=0x%08x VF=0x%08x\n",
            rr(R_CTL_EN1), rr(R_CTL_DMA_EN), rr(R_CTL_FMT_SEL), rr(R_CSI2_CTRL), rr(R_TG_VF_CON));

    /* 5. trigger + wait (first frame partial; take the second) */
    rw(R_TG_VF_CON, rr(R_TG_VF_CON) | 0x1u);

    /* diagnostic: is any MIPI data reaching the TG? poll frame/line counts +
     * seninf CSI2 status over ~500ms. FRMSIZE line count > 0 => data flowing. */
    for (int i = 0; i < 5; i++) {
        usleep(100000);
        fprintf(stderr, "  t=%dms FRM_CNT=0x%08x FRMSIZE=0x%08x SENINF_STA=0x%08x/0x%08x INT=0x%08x\n",
                (i + 1) * 100, rr(0x4444), rr(0x4448), rr(0x8014), rr(0x8018), rr(0x4024));
    }

    for (int frame = 0; frame < 2; frame++) {
        ISP_WAIT_IRQ_STRUCT wq = { IRQ_CLEAR_WAIT, IRQ_TYPE_INT, PASS1_TG1_DON, 2000 };
        int irq = ioctl(isp, ISP_WAIT_IRQ, &wq);
        fprintf(stderr, "frame %d: WAIT PASS1_TG1_DON -> %d (%s)\n", frame, irq,
                irq ? strerror(errno) : "DONE");
        if (irq) break;
    }

    /* 6. stop */
    rw(R_TG_VF_CON, rr(R_TG_VF_CON) & ~0x1u);
    rw(R_CTL_DMA_EN, rr(R_CTL_DMA_EN) & ~0x1u);

    /* 7. cache-invalidate + dump */
    sync_data_t sy = { .cmd = ION_SYS_CACHE_SYNC, .handle = handle, .va = va,
                       .size = FRAME_LEN, .sync = ION_CACHE_INVALID_BY_RANGE };
    struct ion_custom cs = { ION_CMD_SYSTEM, (unsigned long)&sy };
    ioctl(ionfd, ION_CUSTOM, &cs);

    unsigned int nz = 0, sum = 0;
    unsigned char *p = va;
    for (int i = 0; i < FRAME_LEN; i += 101) { if (p[i]) nz++; sum += p[i]; }
    fprintf(stderr, "buffer: non-zero %u/%d  mean~%u\n", nz, FRAME_LEN / 101, sum / (FRAME_LEN / 101));
    int f = open(out, O_WRONLY | O_CREAT | O_TRUNC, 0644);
    if (f >= 0) { if (write(f, va, FRAME_LEN)) {} close(f); fprintf(stderr, "wrote %s\n", out); }

    munmap(va, FRAME_LEN);
    close(mapfd); close(m4u);
    ioctl(sensor, IOC_T_CLOSE);
    return 0;
}
