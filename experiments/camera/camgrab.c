/* camgrab — capture one raw Bayer frame from the SP2509 on L1 (plan A3).
 *
 * CSI-2 path reverse-engineered from stock /vendor/lib/libcamdrv.so
 * (SeninfDrvImp::initTg1CSI2 / setTg1CSI2 / setTg1InputCfg), matching the
 * live Android log:
 *   SettleDelay:14  dlane_num:0  HeaderOrder:1  enable:1
 * which encodes CSI2_CTRL = 0x00000431 (NOT 0x433 — the old lane field was wrong).
 *
 * SAFETY: IMGO DMA only after M4U Virtuality=1 on CAM_IMGO port 17.
 *
 * Build: armv7l-linux-musleabihf-gcc -Os -static -no-pie -Wall -o camgrab camgrab.c
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <errno.h>
#include <fcntl.h>
#include <unistd.h>
#include <stdint.h>
#include <string.h>
#include <signal.h>
#include <sys/ioctl.h>
#include <sys/mman.h>

/* MTK ISP delivers a realtime SIGNAL to the registered PID (ISP_SET_USER_PID)
 * on each pass1/DMA IRQ. Default action terminates us — catch it and count. */
static volatile sig_atomic_t g_isp_sig = 0;
static volatile int g_isp_signo = 0;
static volatile unsigned int g_isp_sival = 0;  /* MTK encodes IRQ status here */
static volatile sig_atomic_t g_stop; /* SIGTERM/INT for --stream */
static void isp_sig_handler(int s, siginfo_t *si, void *uc)
{
    (void)uc;
    g_isp_sig++;
    g_isp_signo = s;
    if (si)
        g_isp_sival = (unsigned int)si->si_value.sival_int;
}
static void stop_sig_handler(int s)
{
    (void)s;
    g_stop = 1;
}

/* ---- imgsensor (magic 'i') ---- */
#define IMGSENSORMAGIC 'i'
typedef struct { unsigned int drvIndex[2]; } SENSOR_DRIVER_INDEX_STRUCT;
/* ACDK_SENSOR_EXPOSURE_WINDOW_STRUCT — all MUINT16 */
typedef struct {
    unsigned short GrabStartX, GrabStartY;
    unsigned short ExposureWindowWidth, ExposureWindowHeight;
    unsigned short ImageTargetWidth, ImageTargetHeight;
    unsigned short ExposurePixel, CurrentExposurePixel;
    unsigned short ExposureLine, ZoomFactor;
} ACDK_SENSOR_EXPOSURE_WINDOW_STRUCT;
/* ACDK_SENSOR_CONFIG_STRUCT — keep large enough; zero-init */
typedef struct {
    int SensorImageMirror;
    int EnableShutterTansfer;
    int EnableFlashlightTansfer;
    int SensorOperationMode;
    unsigned short ImageTargetWidth, ImageTargetHeight;
    unsigned short CaptureShutter;
    unsigned short FlashlightDuty, FlashlightOffset, FlashlightShutFactor, FlashlightMinShutter;
    int MetaMode;
    unsigned int DefaultPclk, Pixels, Lines, Shutter, FrameLines;
    unsigned char pad[64];
} ACDK_SENSOR_CONFIG_STRUCT;
typedef struct {
    unsigned int InvokeCamera;
    unsigned int ScenarioId;
    ACDK_SENSOR_EXPOSURE_WINDOW_STRUCT *pImageWindow;
    ACDK_SENSOR_CONFIG_STRUCT *pSensorConfigData;
} ACDK_SENSOR_CONTROL_STRUCT;
/* feature control (exposure/gain) — 3A does this on stock; we drive it by hand */
typedef struct {
    unsigned int InvokeCamera;
    unsigned int FeatureId;
    unsigned char *pFeaturePara;
    unsigned int *pFeatureParaLen;
} ACDK_SENSOR_FEATURECONTROL_STRUCT;
#define IOC_T_OPEN       _IO(IMGSENSORMAGIC, 0)
#define IOC_X_FEATURE    _IOWR(IMGSENSORMAGIC, 15, ACDK_SENSOR_FEATURECONTROL_STRUCT)
#define IOC_X_CONTROL    _IOWR(IMGSENSORMAGIC, 20, ACDK_SENSOR_CONTROL_STRUCT)
#define IOC_T_CLOSE      _IO(IMGSENSORMAGIC, 25)
#define IOC_X_SET_DRIVER _IOWR(IMGSENSORMAGIC, 35, SENSOR_DRIVER_INDEX_STRUCT)
#define SENSOR_FEATURE_SET_ESHUTTER 3004u  /* para = integration lines (u32) */
#define SENSOR_FEATURE_SET_GAIN     3006u  /* para = gain in 1/64 units (u32) */
#define MAIN_SOCKET 1u
#define SCENARIO_PREVIEW 0u

/* ---- ISP (magic 'k') ---- */
#define ISP_MAGIC 'k'
typedef struct { unsigned int Addr, Val; } ISP_REG_STRUCT;
typedef struct { unsigned int Data, Count; } ISP_REG_IO_STRUCT;
typedef struct { unsigned int Clear, Type, Status, Timeout; } ISP_WAIT_IRQ_STRUCT;
#define ISP_READ_REGISTER    _IOWR(ISP_MAGIC, 2, ISP_REG_IO_STRUCT)
#define ISP_WRITE_REGISTER   _IOWR(ISP_MAGIC, 3, ISP_REG_IO_STRUCT)
#define ISP_WAIT_IRQ         _IOW (ISP_MAGIC, 6, ISP_WAIT_IRQ_STRUCT)
#define ISP_SET_USER_PID     _IOW (ISP_MAGIC, 10, unsigned long)
#define ISP_SENSOR_FREQ_CTRL _IOW (ISP_MAGIC, 14, unsigned long)
#define IRQ_CLEAR_WAIT 1
#define IRQ_TYPE_INT   0
#define IRQ_TYPE_DMA   1
#define PASS1_TG1_DON  (1u << 10)
#define IMGO_DONE_BIT  (1u << 0)   /* ISP_IRQ_DMA_INT_IMGO_DONE_ST */

/* RTBC buffer control (ioctl cmd 11, 16-byte struct) — recovered from stock
 * libcamdrv/libimageio_plat_drv. The kernel owns the IMGO DMA ring: we ENQUE our
 * ion buffer (kernel programs IMGO_BASE from base_pAddr each SOF) then DEQUE the
 * filled frame. Without an enqueued empty buffer the DMA has no target after the
 * first SOF — the exact "one signal then nothing, buffer never written" symptom. */
typedef struct { unsigned int ctrl, buf_id, data_ptr, ex_data_ptr; } ISP_BUFFER_CTRL_STRUCT;
typedef struct {
    unsigned int memID, size, base_vAddr, base_pAddr, tsS, tsUs, bFilled;
} ISP_RT_BUF_INFO_STRUCT;
typedef struct { unsigned int count; ISP_RT_BUF_INFO_STRUCT data[16]; } ISP_DEQUE_BUF_INFO_STRUCT;
#define ISP_BUFFER_CTRL _IOWR(ISP_MAGIC, 11, ISP_BUFFER_CTRL_STRUCT)
#define RT_ENQUE 0u
#define RT_DEQUE 2u
#define DMA_IMGO 4u

/* SENINF / CSI2 (CAMINF-relative; ISP_WRITE_REGISTER window) */
#define R_SENINF_TOP    0x8000u
#define R_SENINF1_CTRL  0x8010u
#define R_SENINF1_INTEN 0x8018u
#define R_SENINF1_SIZE  0x803Cu
#define R_CSI2_CTRL     0x8100u
#define R_CSI2_DELAY    0x8104u
#define R_CSI2_INTEN    0x8108u
#define R_CSI2_LNMUX    0x8128u
#define R_TG1_PH_CNT    0x8300u
#define R_TG1_SEN_CK    0x8304u
#define TG1_PH_VAL      0xA0000001u
/* NCSI2 (alternate digital front-end; MIPI_SENSOR src=8 maps here) */
#define R_NCSI2_CTL     0x8600u
#define R_NCSI2_LNRD_TIMING 0x8608u
#define R_NCSI2_INT_EN  0x8614u
#define R_NCSI2_INT_STA 0x8618u
#define R_NCSI2_DBG     0x8620u
#define R_NCSI2_FRAME   0x862Cu

/* ISP pass1 */
#define R_CTL_EN1       0x4004u
#define R_CTL_EN2       0x4008u
#define R_CTL_DMA_EN    0x400Cu
#define R_CTL_FMT_SEL   0x4010u  /* readable; SET/CLR at 4098/409c are write-1 */
#define R_CTL_SEL       0x4018u
#define R_CTL_INT_EN    0x4020u
#define R_CTL_INT_STATUS 0x4024u
#define R_CTL_DMA_INT   0x4028u
#define R_CTL_MUX_SEL   0x4074u
#define R_MUX_SEL2      0x4078u  /* readable MUX_SEL2 (NOT 0x40C4) */
#define R_CTL_EN1_SET   0x4080u
#define R_CTL_EN1_CLR   0x4084u
#define R_CTL_EN2_SET   0x4088u
#define R_CTL_EN2_CLR   0x408Cu
#define R_CTL_DMA_EN_SET 0x4090u
#define R_CTL_DMA_EN_CLR 0x4094u
#define R_FMT_SEL_SET   0x4098u
#define R_FMT_SEL_CLR   0x409Cu
#define R_CTL_SEL_SET   0x40A0u
#define R_CTL_SEL_CLR   0x40A4u
#define R_MUX_SEL_SET   0x40C0u
#define R_MUX_SEL_CLR   0x40C4u
#define R_MUX_SEL2_SET  0x40C8u
#define R_MUX_SEL2_CLR  0x40CCu
#define R_IMGO_FBC      0x40F4u
#define R_IMGO_SIZE     0x414Cu  /* CTL crop/size mirror */
#define R_CTL_CLK_EN    0x4150u  /* ISP datapath clocks — stock writes 0x0001ffff */
#define R_IMGO_CHECK    0x416Cu
/* CLK: RAW_DP(0)|DIP(2)|FMT(4)|DMA_DP(15) + friends */
#define CLK_EN_PASS1    0x00009efdu
#define PASS1_DB_EN_BIT 0x10u       /* CTL_SEL bit4 */
#define PASS2_DB_EN_BIT 0x20u
#define R_IMGO_BASE     0x4300u
#define R_IMGO_XSIZE    0x4308u
#define R_IMGO_YSIZE    0x430Cu
#define R_IMGO_STRIDE   0x4310u
#define R_TG_SEN_MODE   0x4410u
#define R_TG_VF_CON     0x4414u
#define R_TG_GRAB_PXL   0x4418u
#define R_TG_GRAB_LIN   0x441Cu
#define R_TG_PATH_CFG   0x4420u
#define R_TG_FRM_CNT    0x4444u
#define R_TG_FRMSIZE    0x4448u
/* Stock Pass1: EN1=CAM_EN|PAK_EN|TG1_EN. CQ0 needs a programmed ring — leave EN2 off. */
#define EN1_PASS1       0x40001001u
#define EN2_PASS1       0x8000001Fu   /* low structural bits {0..4} + CQ0_EN (bit31):
                                        * now that RTBC ENQUE gives the kernel a valid
                                        * ring descriptor, the CQ0 engine reloads
                                        * IMGO_BASE from it each SOF and runs the DMA. */

/* MIPIRX config via ISP reg window @ CAMINF+0xC000 */
#define R_MIPI_CFG_24   0xC024u
#define R_MIPI_CFG_38   0xC038u
#define R_MIPI_CFG_3C   0xC03Cu
#define R_MIPI_CFG_44   0xC044u
#define R_MIPI_CFG_48   0xC048u

#define GRAB_W 1600
#define GRAB_H 1200
#define LINE_BYTES (GRAB_W * 10 / 8)     /* 2000 RAW10 packed */
#define FRAME_LEN  (LINE_BYTES * GRAB_H)
#define MIPIRX_ANA_PHYS 0x10010000u
/* SeninfDrvImp::init mmaps these via /dev/camera-isp (ISP_mmap physical pgoff) */
#define IMGSYS_PHYS     0x15000000u
#define GPIO_PHYS       0x10005000u
#define GPIO_MODE_770   0x770u   /* pinmux reg stock BFI mode=1 @ bit12 */
#define GPIO_DRIVE_B60  0xB60u   /* setTg1IODrivingCurrent target (gpio+0xb60) */
#define REG_NCSI2_OFF   0x10u    /* IMGSYS+0x10: stock BFI bits[15:12]=8 */

/* filled after X_CONTROL from driver-adjusted window */
static unsigned g_w = GRAB_W, g_h = GRAB_H, g_sx = 0, g_sy = 0;
static volatile uint32_t *g_imgsys;
static volatile uint32_t *g_gpio;

/* ion — layouts match include/linux/ion_drv.h (MT8127) */
#define ION_MAGIC 'I'
struct ion_alloc { size_t len, align; unsigned int heap_mask, flags; int handle; };
struct ion_fd { int handle, fd; };
struct ion_custom { unsigned int cmd; unsigned long arg; };
#define ION_ALLOC  _IOWR(ION_MAGIC, 0, struct ion_alloc)
#define ION_MAP    _IOWR(ION_MAGIC, 2, struct ion_fd)
#define ION_CUSTOM _IOWR(ION_MAGIC, 6, struct ion_custom)
#define ION_MM_HEAP_MASK (1u << 10)
#define ION_CMD_SYSTEM 0
#define ION_CMD_MM     1
#define ION_MM_CONFIG_BUFFER 0
#define ION_SYS_CACHE_SYNC 0
#define ION_SYS_GET_PHYS   1
#define ION_CACHE_FLUSH_BY_RANGE   2
#define ION_CACHE_INVALID_BY_RANGE 1
/* ion_sys_data / ion_mm_data — pad to 256 so copy_from_user of full kernel struct is safe */
typedef struct {
    unsigned int cmd;
    int handle;
    unsigned int phys;
    unsigned int len;
    unsigned char pad[240];
} sys_data_t;
typedef struct {
    unsigned int cmd;
    int handle;
    void *va;
    unsigned int size;
    unsigned int sync;
    unsigned char pad[232];
} sync_data_t;
typedef struct {
    unsigned int cmd;
    int handle;
    int module;
    unsigned int sec;
    unsigned int coh;
    unsigned char pad[236];
} mm_data_t;

/* M4U — m4u_priv.h (MT8127) */
#define M4U_MAGIC 'g'
typedef struct {
    int ePortID;
    unsigned int Virtuality, Security, domain, Distance, Direction;
} M4U_PORT_STRUCT;
/* module struct for ALLOC/DEALLOC/INSERT_TLB (M4U_MOUDLE_STRUCT) */
typedef struct {
    int eModuleID;
    unsigned int BufAddr;
    unsigned int BufSize;
    unsigned int MVAStart;
    unsigned int MVAEnd;
    int ePriority;
    unsigned int entryCount;
    unsigned int EntryMVA;
    unsigned int Lock;
    int security;
    int cache_coherent;
} M4U_MODULE_STRUCT;
#define MTK_M4U_T_ALLOC_MVA        _IOWR(M4U_MAGIC, 4, int)
#define MTK_M4U_T_DEALLOC_MVA      _IOW(M4U_MAGIC, 5, int)
#define MTK_M4U_T_INSERT_TLB_RANGE _IOW(M4U_MAGIC, 6, int)
#define MTK_M4U_T_CONFIG_PORT      _IOW(M4U_MAGIC, 11, int)
#define MTK_M4U_T_M4UDrv_CONSTRUCT _IOW(M4U_MAGIC, 19, int)
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

/*
 * SeninfDrvImp::init + initTg1CSI2(MAIN) pinmux.
 * Outside ISP_WRITE_REGISTER window → mmap via camera-isp (same as HAL).
 * Without GPIO101-110 mode-1 the MIPI pads never hit the CSI-2 PHY.
 */
static int seninf_hw_init(void)
{
    void *im = mmap(0, 0x1000, PROT_READ | PROT_WRITE, MAP_SHARED, isp, IMGSYS_PHYS);
    void *gp = mmap(0, 0x1000, PROT_READ | PROT_WRITE, MAP_SHARED, isp, GPIO_PHYS);
    if (im == MAP_FAILED || gp == MAP_FAILED) {
        fprintf(stderr, "seninf mmap fail imgsys=%p gpio=%p: %s\n",
                im, gp, strerror(errno));
        return -1;
    }
    g_imgsys = (volatile uint32_t *)im;
    g_gpio   = (volatile uint32_t *)gp;

    /* CMMCLK: GPIO+0x770 bit12 mode=1 */
    {
        uint32_t v = g_gpio[GPIO_MODE_770 / 4];
        uint32_t n = (v & ~0xF000u) | 0x1000u;
        g_gpio[GPIO_MODE_770 / 4] = n;
        fprintf(stderr, "gpio+0x770 CMMCLK 0x%08x -> 0x%08x\n", v, n);
    }

    /* IMGSYS+0x10: stock BFI bits[15:12]=8 */
    {
        uint32_t v = g_imgsys[REG_NCSI2_OFF / 4];
        uint32_t n = (v & ~0xF000u) | (8u << 12);
        g_imgsys[REG_NCSI2_OFF / 4] = n;
        fprintf(stderr, "imgsys+0x10 NCSI2  0x%08x -> 0x%08x\n", v, n);
    }

    /* IO driving @ gpio+0xb60 (level 2 → <<13) */
    {
        uint32_t v = g_gpio[GPIO_DRIVE_B60 / 4];
        uint32_t n = (v & ~0xF000u) | (2u << 13);
        g_gpio[GPIO_DRIVE_B60 / 4] = n;
        fprintf(stderr, "gpio+0xb60 drive  0x%08x -> 0x%08x\n", v, n);
    }

    return 0;
}

/* initTg1CSI2 MAIN: mux GPIO101-110 to MIPI, clear GPI IES bit7 @ 0x910 */
static void mipi_pad_mux_main(void)
{
    if (!g_gpio) {
        fprintf(stderr, "mipi_pad_mux: no gpio map\n");
        return;
    }
#define G(off) g_gpio[(off) / 4]
    uint32_t a910 = G(0x910), a740 = G(0x740), a750 = G(0x750), a760 = G(0x760);
    G(0x910) = a910 & ~0x80u;                 /* GPI*_IES = 0 */
    G(0x740) = (a740 & 0x7u) | 0x1248u;       /* GPIO101-104 mode1 */
    G(0x750) = 0x1249u;                       /* GPIO105-109 mode1 */
    G(0x760) = (a760 & ~0x7u) | 0x1u;         /* GPIO110 mode1 */
    fprintf(stderr,
            "mipi pads MAIN: 910 0x%08x->0x%08x  740 0x%08x->0x%08x\n"
            "                750 0x%08x->0x%08x  760 0x%08x->0x%08x\n",
            a910, G(0x910), a740, G(0x740), a750, G(0x750), a760, G(0x760));
#undef G
}

/*
 * SENINF_TOP_CTRL (0x8000) bitfields (seninf_reg.h):
 *   bit8  SENINF1_PCLK_SEL  — 0=pad, 1=from CSI2/internal
 *   bit9  SENINF2_PCLK_SEL
 *   bit10 SENINF2_PCLK_EN   — stock setTg1PhaseCounter ORs 0x400 only
 *   bit11 SENINF1_PCLK_EN
 *
 * Stock only sets 0x400 (enough for MCLK/ID). TG SOF stayed 0 with that —
 * SYN_VF_DATA_EN never latched, classic "TG domain unclocked". Also enable
 * SENINF1 pclk + select CSI2-derived pclk for the capture path.
 */
#define TOP_PCLK_STOCK  0x400u  /* SENINF2_PCLK_EN — what stock sets */
#define TOP_PCLK_S1_EN  0x800u  /* SENINF1_PCLK_EN */
#define TOP_PCLK_S1_SEL 0x100u  /* SENINF1_PCLK_SEL=1 → CSI2 clock */

static void mclk_on(void)
{
    unsigned long f = 1;
    ioctl(isp, ISP_SENSOR_FREQ_CTRL, &f);
    /* Stock live-preview dump (/proc/driver/isp_reg) shows SENINF_TOP=0x400 ONLY.
     * The extra S1_EN|S1_SEL bits were a wrong guess (mis-routes SENINF1 pclk →
     * width read 101 not 1600). Match stock exactly. */
    rw(R_SENINF_TOP, (rr(R_SENINF_TOP) & ~(TOP_PCLK_S1_EN | TOP_PCLK_S1_SEL)) | TOP_PCLK_STOCK);
    rw(R_TG1_SEN_CK, 0x00010001u); /* 48MHz/2 → 24 MHz */
    rw(R_TG1_PH_CNT, (rr(R_TG1_PH_CNT) & 0x4FFFFFB8u) | TG1_PH_VAL);
    rw(R_TG_SEN_MODE, rr(R_TG_SEN_MODE) | 0x1u); /* CMOS_EN */
    usleep(2000);
    fprintf(stderr, "SENINF_TOP=0x%08x (stock 0x400)\n", rr(R_SENINF_TOP));
}

/*
 * initTg1CSI2(enable=1) — reverse-engineered from libcamdrv SeninfDrvImp::init
 * + initTg1CSI2:
 *   analog working base = mmap(0x10010000) + 0x800   (stored at this+0x40)
 *   config base         = mmap(0x1500C000)            (this+0x44) == ISP 0xC0xx
 */
static int csi2_init_cal(void)
{
    void *m = mmap(0, 0x1000, PROT_READ | PROT_WRITE, MAP_SHARED, isp, MIPIRX_ANA_PHYS);
    if (m == MAP_FAILED) {
        fprintf(stderr, "analog mmap failed: %s\n", strerror(errno));
        return -1;
    }
    /* stock: mipiRxAna = map + 0x800 */
    volatile uint32_t *a = (volatile uint32_t *)((char *)m + 0x800);
#define A(off) a[(off) / 4]

    /* MAIN MIPI pad pinmux BEFORE analog enable (stock initTg1CSI2) */
    mipi_pad_mux_main();

    /* power-domain / bias prep */
    A(0x48) &= ~0x3C0u;
    A(0x4C) &= 0xFEFBEFBEu;
    A(0x50) &= ~1u;

    /* MAIN: MIPI input select on clock + data lanes (bit3), not sub (bit4) */
    A(0x00) |= 0x08; A(0x04) |= 0x08; A(0x08) |= 0x08; A(0x0C) |= 0x08; A(0x10) |= 0x08;
    A(0x00) &= ~0x10u; A(0x0C) &= ~0x10u; A(0x10) &= ~0x10u;
    /* data-lane LDO outs */
    A(0x04) |= 0x01; A(0x08) |= 0x01; A(0x0C) |= 0x01; A(0x10) |= 0x01;
    A(0x20) &= ~0x0Cu; /* RG_CSI0 clock-select clear for MAIN */
    /* MIPI_RX config lane-mux high nibble (MAIN = 0xE4 << 24) */
    rw(R_MIPI_CFG_24, (rr(R_MIPI_CFG_24) & 0x00FFFFFFu) | 0xE4000000u);

    A(0x24) |= 0x03; /* BG core */
    usleep(30);
    A(0x20) |= 0x01; /* LDO core */
    usleep(1);
    A(0x00) |= 0x01; /* LNRC LDO out */
    usleep(1000);

    fprintf(stderr, "analog pre-cal: [0]=0x%08x [0x20]=0x%08x [0x24]=0x%08x cfg24=0x%08x\n",
            A(0x00), A(0x20), A(0x24), rr(R_MIPI_CFG_24));

    /* HW calibration (CSI0) via MIPIRX config window */
    rw(R_MIPI_CFG_24, rr(R_MIPI_CFG_24) | 0x00080000u);
    rw(R_MIPI_CFG_38, rr(R_MIPI_CFG_38) | 0x1u);
    rw(R_MIPI_CFG_3C, 0x1541u);
    rw(R_MIPI_CFG_38, rr(R_MIPI_CFG_38) | 0x4u);
    fprintf(stderr, "CSI0 calibration start\n");
    usleep(500);

    unsigned int c44 = rr(R_MIPI_CFG_44);
    unsigned int c48 = rr(R_MIPI_CFG_48);
    int ok = ((c44 & 0x00010101u) != 0) && ((c48 & 0x00000101u) != 0);
    fprintf(stderr, "CSI0 cal check: 0x44=0x%08x 0x48=0x%08x %s\n",
            c44, c48, ok ? "OK" : "FAIL (continuing)");
    fprintf(stderr, "CSI0 calibration end\n");

    rw(R_MIPI_CFG_38, rr(R_MIPI_CFG_38) & ~1u);

    /* post-cal analog cleanup + LDO bit1 (stock after cal) */
    A(0x20) &= ~0x20u;
    A(0x04) &= ~0x400000u;
    A(0x08) &= ~0x400000u;
    A(0x0C) &= ~0x400000u;
    A(0x10) &= ~0x400000u;
    A(0x20) |= 0x02;

    fprintf(stderr, "analog post: [0]=0x%08x [0x20]=0x%08x [0x24]=0x%08x [0x48]=0x%08x\n",
            A(0x00), A(0x20), A(0x24), A(0x48));
#undef A
    munmap(m, 0x1000);
    return ok ? 0 : 1;
}

/*
 * setTg1CSI2(DataTerm=0, Settle=14, ClkTerm=0, VsyncType=0,
 *            dlane_num=0, enable=1, HeaderOrder=1, DataFlow=0)
 *
 * CSI2_CTRL = ((2<<dlane_num)-2) | (HeaderOrder<<5) | (VsyncType<<13)
 *           | (DataFlow<<17) | 0x411
 *           = 0 | 0x20 | 0 | 0 | 0x411 = 0x431
 */
/* settle delay: stock log SettleDelay:14; override via argv[2] */
static unsigned int g_settle = 14;
/* 0=CSI2@0x8100 (device HAL binary path), 1=NCSI2@0x8600 (src enum MIPI=8) */
static int g_use_ncsi2 = 0;

static void csi2_enable(int on)
{
    unsigned int dlane = 0; /* 0-based: 0 => 1 data lane */
    unsigned int header = 1;
    unsigned int settle = g_settle;
    unsigned int ctrl;

    if (g_use_ncsi2) {
        if (!on) {
            rw(R_NCSI2_CTL, rr(R_NCSI2_CTL) & ~0x1Fu);
            fprintf(stderr, "NCSI2 disable CTL=0x%08x\n", rr(R_NCSI2_CTL));
            return;
        }
        /* select nCSI2 in SRC_SEL bits[15:12]=8 */
        {
            unsigned int s = rr(R_SENINF1_CTRL);
            s = (s & 0xFFFF0FFFu) | 0x8000u;
            rw(R_SENINF1_CTRL, s);
        }
        /* disable first, HSRX_DET off (use settle) */
        rw(R_NCSI2_CTL, rr(R_NCSI2_CTL) & 0xFFFFFEEFu);
        rw(R_NCSI2_LNRD_TIMING, (settle & 0xFFu) << 8);
        /* clock lane + data0, ED_SEL=header, enable */
        {
            unsigned int t = rr(R_NCSI2_CTL);
            t |= (header << 16) | (1u << 4) | 0x1u; /* CLK + D0 */
            rw(R_NCSI2_CTL, t);
        }
        rw(R_NCSI2_INT_EN, 0xF8u);
        /* soft-reset pulse on SENINF1 */
        {
            unsigned int s = rr(R_SENINF1_CTRL) & 0xFFFFFBFCu;
            rw(R_SENINF1_CTRL, s | 0x3u);
            rw(R_SENINF1_CTRL, s);
        }
        fprintf(stderr, "NCSI2 enable CTL=0x%08x TIM=0x%08x SENINF1=0x%08x settle=%u\n",
                rr(R_NCSI2_CTL), rr(R_NCSI2_LNRD_TIMING), rr(R_SENINF1_CTRL), settle);
        return;
    }

    if (!on) {
        /* disable: clear CSI2 enable bit0 */
        rw(R_CSI2_CTRL, rr(R_CSI2_CTRL) & ~1u);
        fprintf(stderr, "CSI2 disable CTRL=0x%08x\n", rr(R_CSI2_CTRL));
        return;
    }

    rw(R_SENINF1_CTRL, rr(R_SENINF1_CTRL) & 0xFFFF0FFFu);
    rw(R_CSI2_DELAY, (settle & 0xFFu) << 16);
    rw(R_CSI2_INTEN, rr(R_CSI2_INTEN) | 0x7u);

    /* exact stock encoding for 1-lane MIPI (dlane_num=0, HeaderOrder=1) */
    ctrl = ((2u << dlane) - 2u) | (header << 5) | 0x411u; /* = 0x431 */
    rw(R_CSI2_CTRL, ctrl);

    /* stock: pulse SW_RST bits 0-1 then clear (setTg1CSI2) */
    {
        unsigned int s = rr(R_SENINF1_CTRL) & 0xFFFFFBFCu;
        rw(R_SENINF1_CTRL, s | 0x3u);
        rw(R_SENINF1_CTRL, s);
    }

    rw(R_CSI2_LNMUX, 0xE4u);

    fprintf(stderr, "CSI2 enable CTRL=0x%08x DELAY=0x%08x LNMUX=0x%08x SENINF1=0x%08x settle=%u\n",
            rr(R_CSI2_CTRL), rr(R_CSI2_DELAY), rr(R_CSI2_LNMUX), rr(R_SENINF1_CTRL), settle);
}

/*
 * setTg1InputCfg(pad=0, inSrcTypeSel=MIPI_SENSOR=8, tgFmt=RAW, senInLsb≠7).
 *
 * Earlier we used the dataBits==7 branch (2-pixel / JPEG-style: PIX_SEL,
 * DBL_DATA_BUS, JPGINF_EN) — wrong for RAW10. Stock non-JPEG MIPI path:
 *   SENINF1 = MUX_EN | ((0x1B<<22)|(0x1F<<16))   // 0x86DF0000, no PIX_SEL
 *   TG_SEN_MODE: clear SOF_SRC(0x300) + clear DBL_DATA_BUS(bit1), keep CMOS_EN
 *   TG_PATH_CFG: clear low2 + clear JPGINF_EN(bit4)
 *   FMT: clear TG1_FMT, do NOT set two-pixel bit24
 */
static void tg_input_mipi(void)
{
    unsigned int s1 = rr(R_SENINF1_CTRL);
    /* MIPI: MUX_EN + pad, do NOT put SRC_SEL in bits 12-15 */
    s1 = (s1 & 0x0FF0FFFFu) | 0x80000000u;
    /* non-JPEG RAW: FIFO push/flush fields 0x1B<<22 | 0x1F<<16 */
    s1 = (s1 & 0xF000FFFFu) | (0x1Bu << 22) | (0x1Fu << 16);
    /* one-pixel mode: clear PIX_SEL bit8 */
    s1 &= ~0x100u;
    /* SENINF1_CTRL polarity nibble: stock live-preview /proc/driver/isp_reg dump
     * reads 0x...0280 (bits 9,7) — NOT bits 10,9 as previously guessed. Wrong
     * polarity bits made the receiver decode 101 px/line instead of 1600. Force
     * the low polarity nibble to stock exactly. */
    s1 = (s1 & ~0x700u) | 0x280u;
    rw(R_SENINF1_CTRL, s1);

    /* TG path: clear SEN_IN_LSB + JPGINF_EN. Do NOT set DB_LOAD_DIS (bit8) — stock
     * TG_PATH=0x01100000 has it CLEAR. DB_LOAD_DIS disables the per-SOF double-buffer
     * LOAD, which is exactly what latches the IMGO shadow base/size to ACTIVE. With
     * PASS1_DB_EN on (0x4018 bit4) AND DB_LOAD_DIS on, the shadow is armed but never
     * commits → IMGO DMA has no target → buffer stays poison. Clear bit8. */
    {
        /* Match stock TG_PATH=0x01100000 exactly: bits 20,24 are the TG→pass1
         * data-path routing (without them TG data never reaches the IMGO path);
         * bit8 (DB_LOAD_DIS) MUST stay clear so the per-SOF DB-load latches the
         * IMGO shadow. camgrab previously left routing off and DB_LOAD_DIS on. */
        unsigned int p = (rr(R_TG_PATH_CFG) & ~0x113u) | 0x01100000u;
        rw(R_TG_PATH_CFG, p);
    }

    /* TG_SEN_MODE: clear SOF_SRC 0x300 + DBL_DATA_BUS bit1; keep CMOS_EN|bit2 */
    rw(R_TG_SEN_MODE, (rr(R_TG_SEN_MODE) & ~0x302u) | 0x5u);

    /*
     * FMT_SEL (isp_reg.h):
     *   SCENARIO[2:0]  CAM_IN/OUT[15:8]  TG1_FMT[18:16]  TWO_PIX[24]
     * TG1_FMT=1 (RAW10), SCENARIO=1 (pass1). Clear TG1_FMT via 0x00070000 not 0x00700000.
     */
    rw(R_FMT_SEL_CLR, 0x00070000u | 0x01000000u | 0x7u);
    rw(R_FMT_SEL_SET, (1u << 16) | 0x1u);

    fprintf(stderr, "input MIPI-RAW: SENINF1=0x%08x TG_MODE=0x%08x PATH=0x%08x FMT=0x%08x\n",
            rr(R_SENINF1_CTRL), rr(R_TG_SEN_MODE), rr(R_TG_PATH_CFG), rr(R_CTL_FMT_SEL));
}

static void tg_grab(void)
{
    /* (end << 16) | start — stock setTg1GrabRange */
    rw(R_TG_GRAB_PXL, ((unsigned)GRAB_W << 16) | 0);
    rw(R_TG_GRAB_LIN, ((unsigned)GRAB_H << 16) | 0);
}

/* setTg1ViewFinderMode: SP bit + continuous */
static void tg_vf_prep(void)
{
    unsigned int v = rr(R_TG_VF_CON);
    v = (v & 0xFFFFE0FDu) | 0x1000u; /* mask + SP_EN style bit */
    rw(R_TG_VF_CON, v);
}

static void reset_seninf(void)
{
    /* stock resetSeninf: OR bits 0-1, then restore original */
    unsigned int v = rr(R_SENINF1_CTRL);
    rw(R_SENINF1_CTRL, v | 0x3u);
    usleep(10);
    rw(R_SENINF1_CTRL, v & ~0x3u);
    usleep(10);
    fprintf(stderr, "resetSeninf done\n");
}

/*
 * Multimedia-heap ion → CONFIG_BUFFER(CAM_IMGO) → GET_PHYS.
 * GET_PHYS triggers m4u_alloc_mva_sg and returns the IOVA (often 0x40000
 * as first free 256KB block — stock logs ~0x1e40000 only after many prior allocs).
 */
static unsigned int ion_buf(void **va, int *mapfd, int *handle, unsigned int *out_len)
{
    struct ion_alloc a;
    memset(&a, 0, sizeof a);
    a.len = FRAME_LEN;
    a.align = 0x1000;
    a.heap_mask = ION_MM_HEAP_MASK;
    if (ioctl(ionfd, ION_ALLOC, &a) < 0) {
        perror("ion alloc");
        return 0;
    }
    *handle = a.handle;

    mm_data_t mm;
    memset(&mm, 0, sizeof mm);
    mm.cmd = ION_MM_CONFIG_BUFFER;
    mm.handle = a.handle;
    mm.module = CAM_IMGO_PORT;
    mm.sec = 0;
    mm.coh = 0;
    struct ion_custom c1 = { ION_CMD_MM, (unsigned long)&mm };
    if (ioctl(ionfd, ION_CUSTOM, &c1) < 0) {
        perror("ion CONFIG_BUFFER(CAM_IMGO)");
        return 0;
    }
    fprintf(stderr, "ion CONFIG_BUFFER(port=%d) ok handle=%d\n", CAM_IMGO_PORT, a.handle);

    sys_data_t sd;
    memset(&sd, 0, sizeof sd);
    sd.cmd = ION_SYS_GET_PHYS;
    sd.handle = a.handle;
    struct ion_custom c2 = { ION_CMD_SYSTEM, (unsigned long)&sd };
    if (ioctl(ionfd, ION_CUSTOM, &c2) < 0) {
        perror("ion GET_PHYS");
        return 0;
    }
    if (!sd.phys || !sd.len) {
        fprintf(stderr, "ion GET_PHYS empty mva=0x%x len=%u\n", sd.phys, sd.len);
        return 0;
    }
    fprintf(stderr, "ion GET_PHYS mva=0x%08x kernel_len=%u\n", sd.phys, sd.len);
    if (out_len)
        *out_len = sd.len;

    struct ion_fd mf = { .handle = a.handle, .fd = -1 };
    if (ioctl(ionfd, ION_MAP, &mf) < 0 || mf.fd < 0) {
        perror("ion MAP");
        return 0;
    }
    *va = mmap(0, FRAME_LEN, PROT_READ | PROT_WRITE, MAP_SHARED, mf.fd, 0);
    *mapfd = mf.fd;
    if (*va == MAP_FAILED) {
        perror("ion mmap");
        return 0;
    }
    return sd.phys;
}

static int ion_cache(int handle, void *va, unsigned size, unsigned sync)
{
    sync_data_t sy;
    memset(&sy, 0, sizeof sy);
    sy.cmd = ION_SYS_CACHE_SYNC;
    sy.handle = handle;
    sy.va = va;
    sy.size = size;
    sy.sync = sync;
    struct ion_custom cs = { ION_CMD_SYSTEM, (unsigned long)&sy };
    return ioctl(ionfd, ION_CUSTOM, &cs);
}

/*
 * Stock live-preview pass1 dump (top+dma+tg config) from Android
 * /proc/driver/isp_reg while CameraTest streamed. Applied wholesale so we stop
 * bit-picking EN1/MUX/FMT. See stock_pass1_regs.inc.
 *
 * Order: modules OFF → config → IMGO base/size → (caller ENQUE) → enables.
 * CQ descriptor pointers from stock are intentionally omitted (stale ion).
 */
#include "stock_pass1_regs.inc"

static int g_stock_regs; /* --stock-regs: bulk-apply stock pass1 dump */
static int g_preview;    /* --preview: also emit downscaled RGB to /tmp/preview.rgb */
static int g_stream;     /* --stream: keep pipeline open, continuous preview */
static unsigned g_ae_shut, g_ae_gain; /* exposure applied this frame (for AE update) */
static unsigned g_last_mean;         /* last AE meter reading (for HUD) */

/* Apply eshutter + gain via sensor feature-control (what 3A uses). */
static void sensor_set_exposure(unsigned eshut, unsigned gain)
{
    unsigned int para, plen = 4;
    ACDK_SENSOR_FEATURECONTROL_STRUCT fc;
    para = eshut;
    fc = (ACDK_SENSOR_FEATURECONTROL_STRUCT){
        MAIN_SOCKET, SENSOR_FEATURE_SET_ESHUTTER,
        (unsigned char *)&para, &plen };
    ioctl(sensor, IOC_X_FEATURE, &fc);
    para = gain;
    fc.FeatureId = SENSOR_FEATURE_SET_GAIN;
    ioctl(sensor, IOC_X_FEATURE, &fc);
    g_ae_shut = eshut;
    g_ae_gain = gain;
}

/*
 * Preview RGB — stock works because it has full 3A + ISP (BNR/LSC/CCM/demosaic).
 * We only have raw IMGO, so the viewfinder does careful software post:
 *   1) full 10-bit MIPI unpack (not high-byte only)
 *   2) black-level subtract
 *   3) 2x2 Bayer-cell average (BGGR) scaled to PW×PH — denoise without mush
 *   4) stock daylight WB (R=1.40 B=1.27) × small gray-world tweak
 *   5) luma 3×3 box denoise, chroma mostly suppressed (noise is color)
 *   6) temporal blend with previous frame in --stream (big win vs grain)
 *   7) soft gamma
 * Output: [u32 w][u32 h][RGB888] + /tmp/preview.meta
 */
#define PW 480u
#define BLACK_LVL 16   /* typical SP2509 pedestal in 8-bit high-byte units */
/* Temporal history for --stream (RGB888, PW x ~360) */
static unsigned char *g_prev_rgb;
static unsigned g_prev_w, g_prev_h;

/* Full 10-bit sample from MIPI RAW10 packed line, returned as 0..1023. */
static inline unsigned pix10(const unsigned char *base, unsigned stride,
                             unsigned x, unsigned y)
{
    const unsigned char *row = base + (size_t)y * stride;
    unsigned g = x >> 2;          /* group of 4 pixels */
    unsigned i = x & 3u;
    const unsigned char *p = row + g * 5;
    unsigned hi = p[i];           /* bits [9:2] */
    unsigned lo = (p[4] >> (i * 2)) & 3u; /* bits [1:0] */
    return (hi << 2) | lo;
}

static void write_preview(const unsigned char *base, unsigned stride,
                          unsigned w, unsigned h)
{
    if (h < 2 || w < 2) return;
    unsigned ph = (PW * h / w) & ~1u;
    if (ph < 2) ph = 2;
    size_t npix = (size_t)PW * ph;
    unsigned char *rgb = malloc(npix * 3);
    if (!rgb) return;

    /* Working buffers in 10-bit-as-16 for precision */
    unsigned short *R = malloc(npix * sizeof(unsigned short));
    unsigned short *G = malloc(npix * sizeof(unsigned short));
    unsigned short *B = malloc(npix * sizeof(unsigned short));
    if (!R || !G || !B) {
        free(rgb); free(R); free(G); free(B);
        return;
    }

    unsigned long sumR = 0, sumG = 0, sumB = 0, nsum = 0;
    for (unsigned oy = 0; oy < ph; oy++) {
        unsigned scy = (oy * h / ph) & ~1u;
        if (scy + 4 > h) scy = (h >= 4) ? (h - 4) & ~1u : 0;
        for (unsigned ox = 0; ox < PW; ox++) {
            unsigned scx = (ox * w / PW) & ~1u;
            if (scx + 4 > w) scx = (w >= 4) ? (w - 4) & ~1u : 0;
            /* 2×2 Bayer cells (4×4 px) average — denoise, keep structure */
            unsigned long ar = 0, ag = 0, ab = 0;
            for (unsigned dy = 0; dy < 4; dy += 2)
                for (unsigned dx = 0; dx < 4; dx += 2) {
                    unsigned sx = scx + dx, sy = scy + dy;
                    unsigned b = pix10(base, stride, sx, sy);
                    unsigned g1 = pix10(base, stride, sx + 1, sy);
                    unsigned g2 = pix10(base, stride, sx, sy + 1);
                    unsigned r = pix10(base, stride, sx + 1, sy + 1);
                    /* black level */
                    b = b > BLACK_LVL ? b - BLACK_LVL : 0;
                    g1 = g1 > BLACK_LVL ? g1 - BLACK_LVL : 0;
                    g2 = g2 > BLACK_LVL ? g2 - BLACK_LVL : 0;
                    r = r > BLACK_LVL ? r - BLACK_LVL : 0;
                    ab += b; ag += g1 + g2; ar += r;
                }
            /* 4 cells → mean; G has 8 samples */
            unsigned rr = (unsigned)(ar >> 2);
            unsigned gg = (unsigned)(ag >> 3);
            unsigned bb = (unsigned)(ab >> 2);
            size_t i = (size_t)oy * PW + ox;
            R[i] = rr > 1023 ? 1023 : (unsigned short)rr;
            G[i] = gg > 1023 ? 1023 : (unsigned short)gg;
            B[i] = bb > 1023 ? 1023 : (unsigned short)bb;
            sumR += R[i]; sumG += G[i]; sumB += B[i]; nsum++;
        }
    }

    /* Stock daylight WB (NVRAM) as base: R=1.40 G=1.0 B=1.273 @ unity 512.
     * Nudge toward gray-world by at most ±15% so indoor/outdoor both work. */
    unsigned mr = nsum ? (unsigned)(sumR / nsum) : 1;
    unsigned mg = nsum ? (unsigned)(sumG / nsum) : 1;
    unsigned mb = nsum ? (unsigned)(sumB / nsum) : 1;
    if (mr < 1) mr = 1;
    if (mg < 1) mg = 1;
    if (mb < 1) mb = 1;
    /* daylight Q8 */
    unsigned wr = 358, wg = 256, wb = 326;
    /* gray-world ideal: wr' = 256*mg/mr … blend 70% daylight + 30% gray */
    unsigned wr_g = (mg * 256) / mr;
    unsigned wb_g = (mg * 256) / mb;
    if (wr_g < 160) wr_g = 160;
    if (wr_g > 480) wr_g = 480;
    if (wb_g < 160) wb_g = 160;
    if (wb_g > 480) wb_g = 480;
    wr = (wr * 7 + wr_g * 3) / 10;
    wb = (wb * 7 + wb_g * 3) / 10;

    /* Apply WB in 10-bit, scale to 8-bit (>>2) */
    for (size_t i = 0; i < npix; i++) {
        unsigned r = (unsigned)R[i] * wr >> 8;
        unsigned g = (unsigned)G[i] * wg >> 8;
        unsigned b = (unsigned)B[i] * wb >> 8;
        if (r > 1023) r = 1023;
        if (g > 1023) g = 1023;
        if (b > 1023) b = 1023;
        R[i] = (unsigned short)r;
        G[i] = (unsigned short)g;
        B[i] = (unsigned short)b;
    }

    /* Luma denoise: 3×3 box on Y, recombine with desaturated chroma.
     * Stock ISP has BNR; this is the poor-man's substitute. */
    {
        unsigned short *Y = malloc(npix * sizeof(unsigned short));
        unsigned short *Ys = malloc(npix * sizeof(unsigned short));
        if (Y && Ys) {
            for (size_t i = 0; i < npix; i++)
                Y[i] = (unsigned short)((R[i] + 2u * G[i] + B[i]) >> 2);
            for (unsigned y = 0; y < ph; y++) {
                for (unsigned x = 0; x < PW; x++) {
                    unsigned long s = 0;
                    unsigned n = 0;
                    for (int dy = -1; dy <= 1; dy++) {
                        int yy = (int)y + dy;
                        if (yy < 0 || yy >= (int)ph) continue;
                        for (int dx = -1; dx <= 1; dx++) {
                            int xx = (int)x + dx;
                            if (xx < 0 || xx >= (int)PW) continue;
                            s += Y[(unsigned)yy * PW + (unsigned)xx];
                            n++;
                        }
                    }
                    Ys[y * PW + x] = (unsigned short)(s / n);
                }
            }
            /* Chroma retention: more when bright & low-gain */
            int sat_cap = 200;
            if (g_ae_gain > 128)
                sat_cap = 200 * 128 / (int)g_ae_gain;
            if (sat_cap < 48) sat_cap = 48;
            if (sat_cap > 220) sat_cap = 220;
            for (size_t i = 0; i < npix; i++) {
                int y0 = (int)Y[i];
                int ys = (int)Ys[i];
                int cr = (int)R[i] - y0;
                int cg = (int)G[i] - y0;
                int cb = (int)B[i] - y0;
                int sat = ys >= 128 ? sat_cap : (ys * sat_cap / 128);
                int r = ys + (cr * sat >> 8);
                int g = ys + (cg * sat >> 8);
                int b = ys + (cb * sat >> 8);
                if (r < 0) r = 0; else if (r > 1023) r = 1023;
                if (g < 0) g = 0; else if (g > 1023) g = 1023;
                if (b < 0) b = 0; else if (b > 1023) b = 1023;
                R[i] = (unsigned short)r;
                G[i] = (unsigned short)g;
                B[i] = (unsigned short)b;
            }
        }
        free(Y); free(Ys);
    }

    /* Soft gamma on 8-bit (>>2 from 10-bit). ~0.75 — between linear and bright. */
    static int gam_ready;
    static unsigned char gam[256];
    if (!gam_ready) {
        for (int v = 0; v < 256; v++) {
            unsigned t = (unsigned)v * 255, r = 0, bit = 1u << 16;
            while (bit > t) bit >>= 2;
            while (bit) {
                if (t >= r + bit) { t -= r + bit; r = (r >> 1) + bit; } else r >>= 1;
                bit >>= 2;
            }
            /* 0.75 ≈ 0.5*linear + 0.5*sqrt */
            unsigned out = ((unsigned)v * 128 + r * 128) >> 8;
            gam[v] = out > 255 ? 255 : (unsigned char)out;
        }
        gam_ready = 1;
    }

    for (size_t i = 0; i < npix; i++) {
        unsigned char *o = &rgb[i * 3];
        o[0] = gam[R[i] >> 2];
        o[1] = gam[G[i] >> 2];
        o[2] = gam[B[i] >> 2];
    }
    free(R); free(G); free(B);

    /* Temporal blend in stream mode — average 50/50 with previous RGB.
     * Stock 3A/ISP is continuous; this is the cheap equivalent for grain. */
    if (g_stream && g_prev_rgb && g_prev_w == PW && g_prev_h == ph) {
        for (size_t i = 0; i < npix * 3; i++)
            rgb[i] = (unsigned char)(((unsigned)rgb[i] + g_prev_rgb[i]) >> 1);
    }
    if (g_stream) {
        if (!g_prev_rgb || g_prev_w != PW || g_prev_h != ph) {
            free(g_prev_rgb);
            g_prev_rgb = malloc(npix * 3);
            g_prev_w = PW;
            g_prev_h = ph;
        }
        if (g_prev_rgb)
            memcpy(g_prev_rgb, rgb, npix * 3);
    }

    int f = open("/tmp/preview.rgb.tmp", O_WRONLY | O_CREAT | O_TRUNC, 0644);
    if (f >= 0) {
        unsigned hdr[2] = { PW, ph };
        write(f, hdr, sizeof hdr);
        write(f, rgb, npix * 3);
        close(f);
        rename("/tmp/preview.rgb.tmp", "/tmp/preview.rgb");
    }
    {
        FILE *mf = fopen("/tmp/preview.meta.tmp", "w");
        if (mf) {
            fprintf(mf, "shut=%u gain=%u mean=%u w=%u h=%u wr=%u wb=%u\n",
                    g_ae_shut, g_ae_gain, g_last_mean ? g_last_mean : (mg >> 2),
                    PW, ph, wr, wb);
            fclose(mf);
            rename("/tmp/preview.meta.tmp", "/tmp/preview.meta");
        }
    }
    free(rgb);
    fprintf(stderr,
            "preview: %ux%u  shut=%u gain=%.1fx mean10 R=%u G=%u B=%u wb r=%u b=%u\n",
            PW, ph, g_ae_shut, g_ae_gain / 64.0, mr, mg, mb, wr, wb);
}

/* Auto-exposure: prefer shutter, keep gain low (stock 3A does this + BNR).
 * Stream mode caps shutter so the viewfinder stays ~10–15 fps-ish. */
static void ae_update(const unsigned char *base, unsigned stride, unsigned w, unsigned h)
{
    unsigned long sum = 0;
    unsigned n = 0;
    for (unsigned y = 0; y + 1 < h; y += 8)
        for (unsigned x = 0; x + 3 < w; x += 8) {
            unsigned v = pix10(base, stride, x, y) >> 2; /* 8-bit-ish for meter */
            if (v >= 8 && v <= 248) { sum += v; n++; }
        }
    unsigned mean = n ? (unsigned)(sum / n) : 100;
    if (mean < 1) mean = 1;
    g_last_mean = mean;
    /* Stock preview is 30 fps @ framelength 1234. For a usable stream we allow
     * up to ~2000 lines (~80 ms) before climbing gain — not 8000 (static + lag). */
    const double TARGET = 105.0, SMIN = 200, GMIN = 64;
    double SMAX = g_stream ? 2000.0 : 4000.0;
    double GMAX = g_stream ? 192.0 : 256.0; /* 3x stream / 4x still */
    if (mean >= 90 && mean <= 125) {
        FILE *sf0 = fopen("/tmp/camgrab_exp", "w");
        if (sf0) { fprintf(sf0, "%u %u\n", g_ae_shut, g_ae_gain); fclose(sf0); }
        fprintf(stderr, "AE: mean=%u hold %u/%u\n", mean, g_ae_shut, g_ae_gain);
        return;
    }
    double ratio = TARGET / mean;
    if (ratio > 1.25) ratio = 1.25;
    if (ratio < 0.80) ratio = 0.80;
    double E = (double)g_ae_shut * g_ae_gain * ratio;
    double shut = E / GMIN;
    if (shut > SMAX) shut = SMAX;
    if (shut < SMIN) shut = SMIN;
    double gain = E / shut;
    if (gain < GMIN) gain = GMIN;
    if (gain > GMAX) gain = GMAX;
    unsigned ns = (unsigned)(shut + 0.5), ng = (unsigned)(gain + 0.5);
    FILE *sf = fopen("/tmp/camgrab_exp", "w");
    if (sf) { fprintf(sf, "%u %u\n", ns, ng); fclose(sf); }
    fprintf(stderr, "AE: mean=%u -> next shut=%u gain=%.1fx\n", mean, ns, ng / 64.0);
}

static void apply_stock_pass1_config(unsigned int mva, unsigned stride, unsigned h)
{
    unsigned i, ncfg;
    /* Kill enables first so a half-configured pipe can't hang the TG. */
    rw(R_CTL_EN1_CLR, 0xFFFFFFFFu);
    rw(R_CTL_EN2_CLR, 0xFFFFFFFFu);
    rw(R_CTL_DMA_EN_CLR, 0xFFFFFFFFu);

    ncfg = (unsigned)(sizeof stock_pass1_cfg / sizeof stock_pass1_cfg[0]);
    for (i = 0; i < ncfg; i++) {
        unsigned int off = stock_pass1_cfg[i].off, val = stock_pass1_cfg[i].val;
        /* Stock MUX2=0xc028030c with minimal EN1 only fills h/2. Use the simple
         * IMGO-after-PAK mux that matches the freerun path; keep stock FMT/SEL. */
        if (off == R_MUX_SEL2)
            val = 0x00100000u;
        if (rw(off, val) < 0)
            fprintf(stderr, "warn stock cfg 0x%04x: %s\n", off, strerror(errno));
    }

    /* Our ion MVA + geometry (stock dump had Android's MVA at IMGO_BASE). */
    rw(R_IMGO_BASE, mva);
    rw(0x4304u, 0);
    rw(R_IMGO_XSIZE, stride - 1);
    rw(R_IMGO_YSIZE, h - 1);
    rw(R_IMGO_STRIDE, stride);
    rw(R_IMGO_SIZE, ((stride - 1) << 16) | (h - 1));

    /*
     * Fixed-base path: clear FBC_EN (bit14). Hybrid stock-regs proved IMGO DMA
     * writes our ion buffer with FBC freelist off; RTBC bFilled still never sets,
     * so ring mode only adds pain. Keep bit4 (write path arm) from stock.
     */
    {
        unsigned int fbc = rr(R_IMGO_FBC);
        fbc = (fbc | 0x10u) & ~(1u << 14);
        rw(R_IMGO_FBC, fbc);
    }

    fprintf(stderr,
            "stock-regs: applied %u cfg regs; IMGO base=0x%08x x=%u y=%u stride=%u\n"
            "  FMT=0x%08x SEL=0x%08x MUX=0x%08x MUX2=0x%08x FBC=0x%08x CLK=0x%08x\n",
            ncfg, rr(R_IMGO_BASE), rr(R_IMGO_XSIZE), rr(R_IMGO_YSIZE),
            rr(R_IMGO_STRIDE), rr(R_CTL_FMT_SEL), rr(R_CTL_SEL),
            rr(R_CTL_MUX_SEL), rr(R_MUX_SEL2), rr(R_IMGO_FBC), rr(R_CTL_CLK_EN));
}

/*
 * g_stock_en_mode:
 *   0 = freerun-safe hybrid (default with --stock-regs): stock FMT/MUX/SEL/DMA
 *       geometry, but EN1=CAM|PAK|TG1 only so TG keeps clocking. Full stock
 *       EN1=0x44b598a9 stalls TG (FRMSIZE PXL→0) without the rest of the HAL.
 *   1 = exact stock EN1/EN2/DMA from dump (--stock-en-full)
 *   2 = stock EN1 + freerun EN2 (no CQ high bits) (--stock-en1-full)
 */
static int g_stock_en_mode;

static void apply_stock_pass1_enables(void)
{
    unsigned en1, en2, dma;
    if (g_stock_en_mode == 1) {
        en1 = 0x44b598a9u;
        en2 = 0xb881401fu;
        dma = 0xabu;
    } else if (g_stock_en_mode == 2) {
        en1 = 0x44b598a9u; /* full modules */
        en2 = 0x0000001fu; /* no CQ0/CQ0B/CQ0C */
        dma = 0xabu;
    } else {
        /* hybrid: freerun-safe EN1; no CQ0 (CQ reloads can truncate a fixed-base
         * frame mid-write — first hybrid capture stopped at exactly h/2). */
        en1 = EN1_PASS1;           /* 0x40001001 */
        en2 = 0x0000001fu;         /* structural bits only */
        dma = 0xabu;
    }
    rw(R_CTL_EN1, en1);
    rw(R_CTL_EN2, en2);
    rw(R_CTL_DMA_EN, dma);
    fprintf(stderr,
            "stock-regs enables mode=%d: EN1=0x%08x EN2=0x%08x DMA=0x%08x "
            "(want EN1=0x%08x)\n",
            g_stock_en_mode, rr(R_CTL_EN1), rr(R_CTL_EN2), rr(R_CTL_DMA_EN), en1);
    (void)stock_pass1_en; /* table kept for reference / mode 1 source */
}

int main(int argc, char **argv)
{
    const char *out = "/tmp/frame.raw";
    /* camgrab [outfile] [settle] [ncsi2=0|1] [--stock-regs]
     * Flags may appear anywhere; first three non-flag args are positional. */
    {
        int pos = 0;
        for (int a = 1; a < argc; a++) {
            if (!strcmp(argv[a], "--stock-regs")) {
                g_stock_regs = 1;
                continue;
            }
            if (!strcmp(argv[a], "--preview")) {
                g_stock_regs = 1;   /* preview rides the working capture path */
                g_preview = 1;
                continue;
            }
            if (!strcmp(argv[a], "--stream")) {
                /* Continuous viewfinder: setup once, keep VF up, re-emit preview. */
                g_stock_regs = 1;
                g_preview = 1;
                g_stream = 1;
                continue;
            }
            if (!strcmp(argv[a], "--stock-en-full")) {
                g_stock_regs = 1;
                g_stock_en_mode = 1;
                continue;
            }
            if (!strcmp(argv[a], "--stock-en1-full")) {
                g_stock_regs = 1;
                g_stock_en_mode = 2;
                continue;
            }
            if (argv[a][0] == '-') {
                fprintf(stderr,
                        "unknown flag %s\n"
                        "  --stock-regs       stock pipe cfg + freerun EN1\n"
                        "  --preview          one-shot RGB preview to /tmp/preview.rgb\n"
                        "  --stream           continuous preview (keeps ISP open)\n"
                        "  --stock-en-full    stock cfg + full stock EN1/EN2\n"
                        "  --stock-en1-full   stock cfg + full EN1, no CQ EN2\n",
                        argv[a]);
                return 1;
            }
            if (pos == 0)
                out = argv[a];
            else if (pos == 1)
                g_settle = (unsigned)atoi(argv[a]);
            else if (pos == 2)
                g_use_ncsi2 = atoi(argv[a]);
            pos++;
        }
    }
    if (g_stock_regs)
        fprintf(stderr, "=== camgrab --stock-regs (bulk stock pass1 dump) ===\n");

    isp = open("/dev/camera-isp", O_RDWR);
    sensor = open("/dev/kd_camera_hw", O_RDWR);
    ionfd = open("/dev/ion", O_RDWR);
    if (isp < 0 || sensor < 0 || ionfd < 0) {
        fprintf(stderr, "open nodes failed (isp=%d sensor=%d ion=%d)\n", isp, sensor, ionfd);
        return 1;
    }

    /* Install handlers for the ISP IRQ signals BEFORE registering our PID, so the
     * first IRQ signal doesn't kill us (default action = terminate). */
    {
        struct sigaction sa;
        memset(&sa, 0, sizeof sa);
        sa.sa_sigaction = isp_sig_handler;
        sa.sa_flags = SA_SIGINFO | SA_RESTART;
        for (int s = SIGRTMIN; s <= SIGRTMAX; s++)
            sigaction(s, &sa, NULL);
        sigaction(SIGUSR1, &sa, NULL);
        sigaction(SIGUSR2, &sa, NULL);
    }
    /* Register our PID with the ISP driver. MTK's ISP IRQ handler advances the
     * pass1/DMA (RTBC) buffer ring and signals the registered user PID on each
     * frame — without this, ISP_WAIT_IRQ EFAULTs AND the kernel's buffer-advance
     * logic never runs, so IMGO never commits a frame. */
    {
        unsigned long pid = (unsigned long)getpid();
        if (ioctl(isp, ISP_SET_USER_PID, &pid) < 0)
            fprintf(stderr, "warn ISP_SET_USER_PID: %s\n", strerror(errno));
        else
            fprintf(stderr, "ISP_SET_USER_PID=%lu ok\n", pid);
    }

    /* 0a. ISP_RESET (ioctl cmd 0) — kernel ISP_Reset() sets the EMI TG bandwidth
     * limiter (*(EMI_BASE+0x120)|=0x3F, which we can't mmap) + HW-resets the CAM.
     * Without the BW limiter the IMGO DMA overruns memory at ~half the frame
     * (INTX DMA_ERR). Do this FIRST, before any config, then configure fresh. */
    if (ioctl(isp, _IO(ISP_MAGIC, 0)) < 0)
        fprintf(stderr, "warn ISP_RESET: %s\n", strerror(errno));
    else
        fprintf(stderr, "ISP_RESET ok (EMI BW limiter + HW reset)\n");

    /* 0. Stock SeninfDrv::init — pinmux + NCSI2 + IO drive (before MCLK/cal) */
    if (seninf_hw_init() < 0)
        fprintf(stderr, "warn: seninf_hw_init failed — continuing without pinmux\n");

    /* 1. MCLK */
    mclk_on();
    fprintf(stderr, "MCLK on  PH_CNT=0x%08x\n", rr(R_TG1_PH_CNT));

    /* 2. CSI2 analog cal + first digital enable (stock: before sensor stream) */
    csi2_init_cal();
    csi2_enable(1);

    /* 3. Sensor open + preview stream with real window/config structs */
    SENSOR_DRIVER_INDEX_STRUCT s = { { (MAIN_SOCKET << 16) | 0, 0 } };
    if (ioctl(sensor, IOC_X_SET_DRIVER, &s) < 0)
        fprintf(stderr, "warn SET_DRIVER: %s\n", strerror(errno));
    if (ioctl(sensor, IOC_T_OPEN) < 0)
        fprintf(stderr, "warn T_OPEN: %s\n", strerror(errno));
    {
        ACDK_SENSOR_EXPOSURE_WINDOW_STRUCT win;
        ACDK_SENSOR_CONFIG_STRUCT cfg;
        memset(&win, 0, sizeof win);
        memset(&cfg, 0, sizeof cfg);
        win.GrabStartX = 0;
        win.GrabStartY = 0;
        win.ExposureWindowWidth = GRAB_W;
        win.ExposureWindowHeight = GRAB_H;
        win.ImageTargetWidth = GRAB_W;
        win.ImageTargetHeight = GRAB_H;
        win.ExposurePixel = GRAB_W;
        win.CurrentExposurePixel = GRAB_W;
        win.ExposureLine = GRAB_H;
        cfg.SensorOperationMode = 0; /* camera preview */
        cfg.ImageTargetWidth = GRAB_W;
        cfg.ImageTargetHeight = GRAB_H;
        /* Exposure — env-tunable (no 3A loop, so we set it by hand). Much longer
         * than the 0x46e default so an indoor scene is actually visible; slow the
         * frame (bigger FrameLines) to allow the long integration. */
        unsigned exp_frame = 8192, exp_shut = 8000;
        { const char *e;
          if ((e = getenv("CAMGRAB_FRAMELINES"))) exp_frame = (unsigned)atoi(e);
          if ((e = getenv("CAMGRAB_SHUTTER")))    exp_shut  = (unsigned)atoi(e); }
        if (exp_shut >= exp_frame) exp_shut = exp_frame - 8;
        cfg.CaptureShutter = exp_shut;
        cfg.DefaultPclk = 24000000;
        cfg.Pixels = GRAB_W;
        cfg.Lines = GRAB_H;
        cfg.Shutter = exp_shut;
        cfg.FrameLines = exp_frame;
        fprintf(stderr, "exposure: shutter=%u framelines=%u (~%u ms integ)\n",
                exp_shut, exp_frame, exp_shut * 947 / 24000);
        ACDK_SENSOR_CONTROL_STRUCT ctl = {
            MAIN_SOCKET, SCENARIO_PREVIEW, &win, &cfg
        };
        if (ioctl(sensor, IOC_X_CONTROL, &ctl) < 0)
            fprintf(stderr, "warn X_CONTROL(preview): %s\n", strerror(errno));
        else
            fprintf(stderr, "X_CONTROL ok  win=%ux%u start=(%u,%u) shutter=%u\n",
                    win.ExposureWindowWidth, win.ExposureWindowHeight,
                    win.GrabStartX, win.GrabStartY, cfg.Shutter);
        /* use driver-adjusted window for TG + DMA (often 1592x1194 not 1600x1200) */
        g_w = win.ExposureWindowWidth ? win.ExposureWindowWidth : GRAB_W;
        g_h = win.ExposureWindowHeight ? win.ExposureWindowHeight : GRAB_H;
        g_sx = win.GrabStartX;
        g_sy = win.GrabStartY;
        /* Grab-width cap: the IMGO DMA can't drain full 1592-wide lines fast enough
         * on L1 and overruns at exactly h/2 (INTX DMA_ERR). Empirically <=1550 wide
         * captures the FULL height; 1592 truncates to 597. So cap at 1550 (only 2.6%
         * FOV) for a full-frame preview. Override with CAMGRAB_GRABW. */
        if (g_preview && g_w > 1550) g_w = 1550;
        { const char *e = getenv("CAMGRAB_GRABW");
          if (e && atoi(e) > 0 && (unsigned)atoi(e) < g_w) g_w = (unsigned)atoi(e) & ~1u; }
        /* center the crop so we lose FOV symmetrically, not off one side */
        if (g_w < win.ExposureWindowWidth)
            g_sx = win.GrabStartX + ((win.ExposureWindowWidth - g_w) / 2 & ~1u);
    }

    /* Real exposure/gain via the sensor feature-control path (what 3A uses) — the
     * preview cfg.Shutter above is ignored by the sensor. In --preview we run a
     * simple auto-exposure loop across invocations via /tmp/camgrab_exp: apply the
     * previous frame's computed exposure now, then update it from this frame's mean
     * (below). Prefer long shutter over gain so the image isn't just amplified noise. */
    {
        unsigned int eshut = 3000, gain = 1024; /* gain 1/64 units: 1024 = 16x */
        const char *e;
        if (g_preview) {
            FILE *sf = fopen("/tmp/camgrab_exp", "r");
            if (sf) { if (fscanf(sf, "%u %u", &eshut, &gain) != 2) { eshut = 2000; gain = 256; } fclose(sf); }
            else { eshut = 2000; gain = 256; } /* first-run seed: 2000 lines, 4x */
        }
        if ((e = getenv("CAMGRAB_ESHUTTER"))) eshut = (unsigned)atoi(e);
        if ((e = getenv("CAMGRAB_GAIN")))     gain  = (unsigned)atoi(e);
        g_ae_shut = eshut; g_ae_gain = gain;
        unsigned int para, plen = 4;
        ACDK_SENSOR_FEATURECONTROL_STRUCT fc;
        para = eshut; fc = (ACDK_SENSOR_FEATURECONTROL_STRUCT){
            MAIN_SOCKET, SENSOR_FEATURE_SET_ESHUTTER,
            (unsigned char *)&para, &plen };
        int r1 = ioctl(sensor, IOC_X_FEATURE, &fc);
        para = gain; fc.FeatureId = SENSOR_FEATURE_SET_GAIN;
        int r2 = ioctl(sensor, IOC_X_FEATURE, &fc);
        fprintf(stderr, "exposure(feature): eshutter=%u(rc=%d) gain=%u/64=%.1fx(rc=%d)\n",
                eshut, r1, gain, gain / 64.0, r2);
    }

    usleep(100000); /* let MIPI clock lane enter HS */

    /* 4. Stock: reset → grab → mode → input → vf → CSI2 disable/enable */
    reset_seninf();
    /* Stock RAW grab: start=SensorGrabStart, end=start+crop (see sensor_hal) */
    rw(R_TG_GRAB_PXL, ((unsigned)(g_sx + g_w) << 16) | g_sx);
    rw(R_TG_GRAB_LIN, ((unsigned)(g_sy + g_h) << 16) | g_sy);
    fprintf(stderr, "TG grab pxl=0x%08x lin=0x%08x (%ux%u @%u,%u)\n",
            rr(R_TG_GRAB_PXL), rr(R_TG_GRAB_LIN), g_w, g_h, g_sx, g_sy);
    rw(R_TG_SEN_MODE, rr(R_TG_SEN_MODE) | 0x5u);
    tg_input_mipi();
    tg_vf_prep();
    /* Stock: CSI2 disable/enable re-arm after inputCfg */
    if (g_use_ncsi2) {
        csi2_enable(0);
        usleep(1000);
        csi2_enable(1);
        /* restore MIPI SENINF1 fields after enable's SRC_SEL write */
        tg_input_mipi();
        /* force SRC_SEL=8 for NCSI2 */
        rw(R_SENINF1_CTRL, (rr(R_SENINF1_CTRL) & 0xFFFF0FFFu) | 0x8000u | 0x80000000u);
        fprintf(stderr, "NCSI2 re-arm CTL=0x%08x SENINF1=0x%08x\n",
                rr(R_NCSI2_CTL), rr(R_SENINF1_CTRL));
    } else {
        unsigned int c = rr(R_CSI2_CTRL);
        rw(R_CSI2_CTRL, c & ~1u);
        usleep(1000);
        rw(R_CSI2_CTRL, (c & ~1u) | 0x431u);
        rw(R_CSI2_DELAY, (g_settle & 0xFFu) << 16);
        rw(R_CSI2_INTEN, rr(R_CSI2_INTEN) | 0x7u);
        rw(R_CSI2_LNMUX, 0xE4u);
        rw(0x810c, rr(0x810c));
        fprintf(stderr, "CSI2 re-arm CTRL=0x%08x SENINF1=0x%08x settle=%u\n",
                rr(R_CSI2_CTRL), rr(R_SENINF1_CTRL), g_settle);
    }

    /* 5. DMA buffer + M4U — size to actual grab */
    unsigned line_b = (g_w * 10 + 7) / 8;
    /* stock warns RAW10 stride multiple of 8 */
    unsigned stride = (line_b + 7u) & ~7u;
    unsigned frame_len = stride * g_h;
    if (frame_len > FRAME_LEN)
        frame_len = FRAME_LEN;
    void *va; int mapfd, handle;
    unsigned int mva_len = 0;
    unsigned int mva = ion_buf(&va, &mapfd, &handle, &mva_len);
    if (!mva || va == MAP_FAILED) {
        fprintf(stderr, "buffer alloc failed\n");
        return 1;
    }
    fprintf(stderr, "ion MVA=0x%08x mva_len=%u frame=%u (w=%u line=%u stride=%u)\n",
            mva, mva_len, frame_len, g_w, line_b, stride);

    int m4u = open("/proc/M4U_device", O_RDONLY);
    if (m4u < 0) {
        fprintf(stderr, "ABORT: no /proc/M4U_device — refusing DMA\n");
        return 1;
    }
    /* userspace M4U client construct (matches libm4u) */
    if (ioctl(m4u, MTK_M4U_T_M4UDrv_CONSTRUCT, 0) < 0)
        fprintf(stderr, "warn M4U CONSTRUCT: %s\n", strerror(errno));

    M4U_PORT_STRUCT port;
    memset(&port, 0, sizeof port);
    port.ePortID = CAM_IMGO_PORT;
    port.Virtuality = 1;
    port.Security = 0;
    port.domain = 3; /* kernel hardcodes domain 3 anyway */
    port.Distance = 1;
    port.Direction = 0;
    if (ioctl(m4u, MTK_M4U_T_CONFIG_PORT, &port) < 0) {
        fprintf(stderr, "ABORT: M4U CONFIG_PORT failed (%s)\n", strerror(errno));
        return 1;
    }
    fprintf(stderr, "M4U CAM_IMGO Virtuality=1 domain=3 OK\n");

    /* sequential TLB range for the whole frame (optional perf; harmless) */
    {
        M4U_MODULE_STRUCT tr;
        memset(&tr, 0, sizeof tr);
        tr.eModuleID = CAM_IMGO_PORT;
        tr.MVAStart = mva;
        tr.MVAEnd = mva + frame_len - 1;
        tr.ePriority = 0;
        tr.entryCount = 1;
        if (ioctl(m4u, MTK_M4U_T_INSERT_TLB_RANGE, &tr) < 0)
            fprintf(stderr, "warn INSERT_TLB_RANGE: %s\n", strerror(errno));
        else
            fprintf(stderr, "M4U TLB range 0x%08x-0x%08x\n", tr.MVAStart, tr.MVAEnd);
    }

    /* poison + flush so DRAM has known pattern (detect DMA / prove cache path).
     * Top half 0xA5, bottom half 0x5A — distinguishes "DMA stopped at h/2" from
     * "cache invalidate only covered the first megabyte". */
    memset(va, 0xA5, frame_len / 2);
    memset((unsigned char *)va + frame_len / 2, 0x5A, frame_len - frame_len / 2);
    if (ion_cache(handle, va, frame_len, ION_CACHE_FLUSH_BY_RANGE) < 0)
        fprintf(stderr, "warn cache FLUSH: %s\n", strerror(errno));

    if (g_stock_regs) {
        /* Bulk stock pass1: config only (enables after RTBC ENQUE). */
        apply_stock_pass1_config(mva, stride, g_h);
    } else {
        /*
         * IMGO DMA — stock DMAO_B::_config: CON=0x850, CON2=0x800.
         * FBC OFF (fixed BASE_ADDR). IMGO_MUX=0 + MUX_EN → after PAK.
         * EN1/DMA_EN via SET regs (HAL ISP_WRITE_ENABLE path).
         */
        rw(R_IMGO_BASE, mva);
        rw(0x4304u, 0);                 /* IMGO_OFST */
        rw(R_IMGO_XSIZE, stride - 1);   /* bytes-1 */
        rw(R_IMGO_YSIZE, g_h - 1);      /* lines-1 */
        rw(R_IMGO_STRIDE, stride);      /* byte stride (8-aligned) */
        /* Stock live dump (/proc/driver/isp_reg dma sec) shows the REAL runtime values
         * carry high bits the disasm missed: CON=0x08100850, CON2=0x00100800. The
         * shared 0x00100000 bit looks like the DMA enable/valid — without it IMGO
         * never commits (buffer stays poison). Match stock exactly. */
        rw(0x4314u, 0x08100850u);       /* IMGO_CON  FIFO/burst + 0x08100000 */
        rw(0x4318u, 0x00100800u);       /* IMGO_CON2 + 0x00100000 */
        rw(0x431Cu, 0);                 /* IMGO_CROP */
        /* CTL_IMGO_SIZE: YSIZE[12:0] | XSIZE[28:16] */
        rw(R_IMGO_SIZE, ((stride - 1) << 16) | (g_h - 1));

        rw(R_CTL_CLK_EN, CLK_EN_PASS1);
        /* DB-LOAD: stock CTL_SEL(0x4018)=0x00018054 has PASS1_DB_EN (bit4) SET — it's
         * the double-buffer LOAD enable that latches the IMGO shadow base/size to ACTIVE
         * each frame. camgrab was CLEARING it, so the IMGO config never went active and
         * the writeback DMA had no valid target. SET it; clear pass2/cq + bogus bit31. */
        rw(R_CTL_SEL_CLR, PASS2_DB_EN_BIT | 0xCu | 0x80000000u);
        rw(R_CTL_SEL_SET, PASS1_DB_EN_BIT);
        /* IMGO from PAK: clear MUX bit4, set MUX_EN bit20 */
        rw(R_MUX_SEL2_CLR, (1u << 4));
        rw(R_MUX_SEL2_SET, (1u << 20));
        /* MUX_SEL (0x4074): stock live = 0x00100008 — camgrab never set this. It's the
         * primary CAM input MUX (bit3 + bit20); without it the TG data path into pass1
         * isn't selected, so IMGO gets nothing even with MUX_SEL2/routing correct. */
        rw(0x4074u, 0x00100008u);
        /* enable modules via SET (not raw 4004/400C) */
        rw(R_CTL_EN1_CLR, 0xFFFFFFFFu);
        rw(R_CTL_EN1_SET, EN1_PASS1);
        rw(R_CTL_EN2_CLR, 0xFFFFFFFFu);
        rw(R_CTL_EN2_SET, EN2_PASS1);

        /*
         * Stock first-buffer path (camera_isp RTBC_ENQUE empty_count==1):
         *   write IMGO_BASE, then explicitly clear FBC_EN (bit14) so DMA uses
         *   fixed base — not freelist. FBC freelist mode needs CQ0C ring.
         */
        rw(R_IMGO_BASE, mva);
        {
            /* With the RTBC ring (ISP_BUFFER_CTRL ENQUE), the kernel manages the IMGO
             * frame-buffer-control RING — so FBC_EN (bit14) must be SET, not cleared.
             * Match stock live value exactly (0x02234010): bit4, bit14(FBC_EN),
             * bits16,17, bit21, bit25. Clearing bit14 (old fixed-base logic) fought the
             * ring and produced a TG grab error. */
            rw(R_IMGO_FBC, 0x02234010u);
        }
        fprintf(stderr, "FBC (RTBC ring, stock 0x02234010): 0x%08x\n", rr(R_IMGO_FBC));

        /* CTL_DMA_EN: stock arms 5 DMA masters (0xab), not IMGO bit0 alone —
         * the IMGO writeback needs its companion DMA bits {1,3,5,7}. */
        rw(R_CTL_DMA_EN_CLR, 0xFFFFFFFFu);
        rw(R_CTL_DMA_EN_SET, 0xABu);
        /* IRQs: TG1 done + IMGO err so INT_STATUS is informative */
        rw(R_CTL_INT_EN, (1u << 10) | (1u << 16));
        rw(R_IMGO_BASE, mva);
    }

    fprintf(stderr,
            "IMGO base=0x%08x x=%u y=%u stride=%u CON=0x%08x CON2=0x%08x\n",
            rr(R_IMGO_BASE), rr(R_IMGO_XSIZE), rr(R_IMGO_YSIZE),
            rr(R_IMGO_STRIDE), rr(0x4314u), rr(0x4318u));
    fprintf(stderr,
            "pre-ENQUE: EN1=0x%08x EN2=0x%08x DMA=0x%08x CSI2=0x%08x VF=0x%08x "
            "FMT=0x%08x CLK_EN=0x%08x SEL=0x%08x MUX2=0x%08x FBC=0x%08x "
            "IMGO_SZ=0x%08x\n",
            rr(R_CTL_EN1), rr(R_CTL_EN2), rr(R_CTL_DMA_EN), rr(R_CSI2_CTRL),
            rr(R_TG_VF_CON), rr(R_CTL_FMT_SEL), rr(R_CTL_CLK_EN), rr(R_CTL_SEL),
            rr(R_MUX_SEL2), rr(R_IMGO_FBC), rr(R_IMGO_SIZE));

    /* 5b. RTBC ENQUE — only for the non-stock path. Stock hybrid uses fixed-base
     * IMGO (FBC_EN clear): ENQUE was rewriting YSIZE from buf size and left us
     * with exactly h/2 lines. Skip the ring entirely when --stock-regs. */
#define NRTBUF 3
    static ISP_RT_BUF_INFO_STRUCT rtbuf[NRTBUF];
    void *bva[NRTBUF]; unsigned int bmva[NRTBUF]; int bhandle[NRTBUF];
    bva[0] = va; bmva[0] = mva; bhandle[0] = handle;
    if (!g_stock_regs) {
        for (int b = 1; b < NRTBUF; b++) {
            int bfd; unsigned int bl = 0;
            unsigned int m = ion_buf(&bva[b], &bfd, &bhandle[b], &bl);
            if (!m || bva[b] == MAP_FAILED) {
                fprintf(stderr, "extra buf %d alloc fail — falling back to 1 buffer\n", b);
                bva[b] = va; bmva[b] = mva; bhandle[b] = handle;
            } else {
                bmva[b] = m;
                memset(bva[b], 0xA5, frame_len);
            }
        }
        memset(rtbuf, 0, sizeof rtbuf);
        for (int b = 0; b < NRTBUF; b++) {
            rtbuf[b].memID = (unsigned int)bhandle[b];
            rtbuf[b].size = frame_len;
            rtbuf[b].base_vAddr = (unsigned int)(unsigned long)bva[b];
            rtbuf[b].base_pAddr = bmva[b];
            rtbuf[b].bFilled = 0;
            ISP_BUFFER_CTRL_STRUCT bc = {
                RT_ENQUE, DMA_IMGO, (unsigned int)(unsigned long)&rtbuf[b], 0
            };
            if (ioctl(isp, ISP_BUFFER_CTRL, &bc) < 0)
                fprintf(stderr, "warn RTBC ENQUE[%d]: %s\n", b, strerror(errno));
            else
                fprintf(stderr, "RTBC ENQUE[%d] pa=0x%08x va=%p memID=%d ok\n",
                        b, bmva[b], bva[b], bhandle[b]);
        }
    } else {
        fprintf(stderr, "stock-regs: skip RTBC ENQUE (fixed-base IMGO)\n");
    }

    if (g_stock_regs) {
        apply_stock_pass1_enables();
        rw(R_IMGO_BASE, mva);
        {
            unsigned int fbc = rr(R_IMGO_FBC);
            fbc = (fbc | 0x10u) & ~(1u << 14);
            rw(R_IMGO_FBC, fbc);
        }
        /* Normal YSIZE=h-1 with fixed base; the half-height bug was ENQUE clobber. */
        rw(R_IMGO_XSIZE, stride - 1);
        rw(R_IMGO_YSIZE, g_h - 1);
        rw(R_IMGO_STRIDE, stride);
        rw(R_IMGO_SIZE, ((stride - 1) << 16) | (g_h - 1));
        rw(R_IMGO_BASE, mva);
        fprintf(stderr,
                "post-stock-en: EN1=0x%08x EN2=0x%08x DMA=0x%08x FBC=0x%08x "
                "BASE=0x%08x Y=0x%x\n",
                rr(R_CTL_EN1), rr(R_CTL_EN2), rr(R_CTL_DMA_EN),
                rr(R_IMGO_FBC), rr(R_IMGO_BASE), rr(R_IMGO_YSIZE));
    }

    /* 5c. THE GATE (from kernel camera_isp.c disasm): the driver's frame-done
     * handler, RTBC ring-advance and DEQUE are ALL gated on *(ISP_base+0x414)&1 —
     * the pass1/CMOS "streaming active" bit. ISP_base(CAMINF) physical = 0x15004000
     * (ioctl 0x8000=SENINF=0x1500C000 ⇒ CAMINF=0x15004000); the CAM_CTL block at
     * 0x15004000..0x15007fff sits BELOW the ioctl window [0x4000,0x10000) so camgrab
     * never touched it. 0x414 = CAMINF+0x414 = phys 0x15004414, reachable via mmap.
     * Without this bit no frame-done IRQ fires → bFilled never set → WAIT_IRQ EFAULTs. */
    {
        /* ISP_mmap only accepts the IMGSYS base 0x15000000 as pgoff, not a sub-
         * offset. CAMINF = IMGSYS+0x4000, so the gate 0x15004414 = offset 0x4414
         * in a large-enough 0x15000000 mapping. */
        volatile uint32_t *img = mmap(0, 0x8000, PROT_READ | PROT_WRITE,
                                      MAP_SHARED, isp, 0x15000000u);
        if (img == MAP_FAILED) {
            fprintf(stderr, "warn: IMGSYS 0x8000 mmap failed for gate: %s\n",
                    strerror(errno));
        } else {
            unsigned int before = img[0x4414 / 4];
            img[0x4414 / 4] = before | 1u;
            __sync_synchronize();
            fprintf(stderr, "GATE CAMINF+0x414 (imgsys+0x4414): 0x%08x -> 0x%08x\n",
                    before, img[0x4414 / 4]);
        }
    }

    /* 6. Viewfinder on. Re-assert IMGO geometry once more just before VF so a
     * late ENQUE/FBC poke cannot leave a half-height YSIZE latched. */
    if (g_stock_regs) {
        rw(R_IMGO_BASE, mva);
        rw(R_IMGO_XSIZE, stride - 1);
        rw(R_IMGO_YSIZE, g_h - 1);
        rw(R_IMGO_STRIDE, stride);
        rw(R_IMGO_SIZE, ((stride - 1) << 16) | (g_h - 1));
        fprintf(stderr, "pre-VF reassert: BASE=0x%08x X=0x%x Y=0x%x STR=0x%x FBC=0x%08x\n",
                rr(R_IMGO_BASE), rr(R_IMGO_XSIZE), rr(R_IMGO_YSIZE),
                rr(R_IMGO_STRIDE), rr(R_IMGO_FBC));
    }
    /* (EMI TG bandwidth limiter is applied kernel-side by ISP_RESET at 0a — the EMI
     * block isn't in the ISP_mmap whitelist so we can't poke it from userspace.) */
    rw(R_TG_VF_CON, rr(R_TG_VF_CON) | 0x1u);

    /* Settle after first exposure program. Stream mode only needs ~1 frame;
     * one-shot --preview keeps a longer settle so a single snap looks right. */
    if (g_preview)
        usleep(g_stream ? 200000 : 350000);

    int got = 0;
    for (int i = 0; i < 30; i++) {
        usleep(100000);
        /* Chunked invalidate — some MTK ion paths silently cap large ranges. */
        {
            unsigned chunk = 256 * 1024;
            for (unsigned off = 0; off < frame_len; off += chunk) {
                unsigned n = frame_len - off;
                if (n > chunk) n = chunk;
                ion_cache(handle, (unsigned char *)va + off, n,
                          ION_CACHE_INVALID_BY_RANGE);
            }
        }
        unsigned char *bp = va;
        unsigned nz = 0, chg = 0, n = frame_len / 64;
        unsigned still_a5 = 0, still_5a = 0;
        for (unsigned k = 0; k < frame_len; k += 64) {
            if (bp[k] != 0xA5 && bp[k] != 0x5A) chg++;
            else if (bp[k] == 0xA5) still_a5++;
            else still_5a++;
            if (bp[k]) nz++;
        }
        fprintf(stderr, "    poison: still_A5=%u still_5A=%u real_chg=%u\n",
                still_a5, still_5a, chg);
        fprintf(stderr,
                "  t=%3dms SIZE=0x%08x INTER=0x%08x SIG=%d/sig%d "
                "INT=0x%08x DMAINT=0x%08x IMGOERR=0x%08x BASE=0x%08x "
                "buf(chg=%u/%u nz=%u)\n",
                (i + 1) * 100, rr(R_TG_FRMSIZE), rr(0x444c),
                (int)g_isp_sig, g_isp_signo,
                rr(R_CTL_INT_STATUS), rr(R_CTL_DMA_INT),
                rr(0x43ACu), rr(R_IMGO_BASE),
                chg, n, nz);
        /* Real pixels beat RTBC bFilled — accept DMA writeback as a frame. */
        if (chg > (n / 4)) {
            got = 1;
            fprintf(stderr, "  >>> buffer fill detected (chg=%u/%u) — treating as frame\n",
                    chg, n);
            break;
        }
    }

    for (int frame = 0; frame < 6 && !got; frame++) {
        /* WAIT_IRQ: the kernel copies Status back out, so pass a WRITABLE,
         * over-allocated struct (fixes the EFAULT camgrab hit before). Wait on
         * the IMGO DMA-done bit. */
        struct { ISP_WAIT_IRQ_STRUCT w; unsigned char pad[48]; } wq;
        memset(&wq, 0, sizeof wq);
        wq.w.Clear = IRQ_CLEAR_WAIT;
        wq.w.Type = IRQ_TYPE_DMA;
        wq.w.Status = IMGO_DONE_BIT;
        wq.w.Timeout = 2000;
        int irq = ioctl(isp, ISP_WAIT_IRQ, &wq);
        fprintf(stderr, "frame %d: WAIT_IRQ(DMA IMGO) -> %d (%s) sig=%d sival=0x%08x status=0x%08x\n",
                frame, irq, irq ? strerror(errno) : "DONE",
                (int)g_isp_sig, g_isp_sival, wq.w.Status);

        /* DEQUE: ask which buffer(s) the kernel filled */
        ISP_DEQUE_BUF_INFO_STRUCT deq;
        memset(&deq, 0, sizeof deq);
        ISP_BUFFER_CTRL_STRUCT bd = {
            RT_DEQUE, DMA_IMGO, (unsigned int)(unsigned long)&deq, 0
        };
        int dr = ioctl(isp, ISP_BUFFER_CTRL, &bd);
        fprintf(stderr, "   DEQUE rc=%d count=%u", dr, deq.count);
        for (unsigned k = 0; k < deq.count && k < 16; k++)
            fprintf(stderr, " [pa=0x%08x filled=%u]",
                    deq.data[k].base_pAddr, deq.data[k].bFilled);
        fprintf(stderr, "\n");
        if (dr == 0) {
            for (unsigned k = 0; k < deq.count && k < 16; k++) {
                if (!deq.data[k].bFilled)
                    continue;
                /* point the read/write-out at whichever buffer the kernel filled */
                for (int b = 0; b < NRTBUF; b++)
                    if (bmva[b] == deq.data[k].base_pAddr) {
                        va = bva[b];
                        handle = bhandle[b];
                        break;
                    }
                got = 1;
            }
            if (got)
                break;
        }
        if (irq != 0)
            usleep(50000);
    }

    /* Keep VF up through buffer read / --stream; hardware is stopped at exit. */

    if (ion_cache(handle, va, frame_len, ION_CACHE_INVALID_BY_RANGE) < 0)
        fprintf(stderr, "warn cache INVALID: %s\n", strerror(errno));

    unsigned int nz = 0, sum = 0, n = 0, chg = 0, a5 = 0;
    unsigned char *p = va;
    for (unsigned i = 0; i < frame_len; i += 64) {
        if (p[i]) nz++;
        if (p[i] != 0xA5) chg++;
        if (p[i] == 0xA5) a5++;
        sum += p[i];
        n++;
    }
    /* Also count every-byte (not just stride-64 sample) for poison boundary. */
    {
        unsigned first_a5_run = frame_len, last_chg = 0, lines_hit = 0;
        for (unsigned i = 0; i < frame_len; i++) {
            if (p[i] != 0xA5)
                last_chg = i;
        }
        for (unsigned i = 0; i + 64 < frame_len; i++) {
            if (p[i] == 0xA5) {
                int run = 1;
                for (unsigned j = 1; j < 64; j++)
                    if (p[i + j] != 0xA5) { run = 0; break; }
                if (run) { first_a5_run = i; break; }
            }
        }
        if (stride) {
            for (unsigned y = 0; y < g_h; y++) {
                unsigned off = y * stride;
                if (off >= frame_len) break;
                if (p[off] != 0xA5 || (off + 8 < frame_len && p[off + 8] != 0xA5))
                    lines_hit++;
            }
        }
        fprintf(stderr,
                "buffer: non-zero %u/%u  changed_from_A5 %u  still_A5 %u  mean~%u  "
                "got_frame=%d  first_A5_run@%u last_chg@%u lines_hit~%u/%u\n",
                nz, n, chg, a5, n ? sum / n : 0, got,
                first_a5_run, last_chg, lines_hit, g_h);
        if (!got && chg > (n / 4))
            got = 1;
    }

    int f = open(out, O_WRONLY | O_CREAT | O_TRUNC, 0644);
    if (f >= 0) {
        ssize_t w = write(f, va, frame_len);
        close(f);
        fprintf(stderr, "wrote %s (%zd bytes)  meta %ux%u RAW10\n", out, w, g_w, g_h);
    }

    if (g_preview) {
        /* Meter first so write_preview can log / meta the true mean. */
        ae_update(va, stride, g_w, g_h);
        write_preview(va, stride, g_w, g_h);
    }

    /* --stream: keep VF + sensor open; re-sample the fixed-base buffer each
     * frame period, re-emit /tmp/preview.rgb, chase AE. This is the fps win —
     * no per-frame ISP open/cal/CSI-setup. Kill with SIGTERM/SIGINT. */
    if (g_stream && got) {
        signal(SIGTERM, stop_sig_handler);
        signal(SIGINT, stop_sig_handler);
        unsigned fr = 1;
        fprintf(stderr, "stream: continuous preview (SIGTERM to stop)\n");
        while (!g_stop) {
            /* Apply AE target computed from previous frame */
            {
                unsigned eshut = g_ae_shut, gain = g_ae_gain;
                FILE *sf = fopen("/tmp/camgrab_exp", "r");
                if (sf) {
                    unsigned s, g;
                    if (fscanf(sf, "%u %u", &s, &g) == 2) {
                        eshut = s; gain = g;
                    }
                    fclose(sf);
                }
                if (eshut != g_ae_shut || gain != g_ae_gain)
                    sensor_set_exposure(eshut, gain);
            }
            /* Wait ~1 frame (+margin). Shutter lines @ ~947 pclk/line / 24 MHz. */
            {
                unsigned frame_us = g_ae_shut * 947u / 24u + 40u * 1000u;
                if (frame_us < 80000) frame_us = 80000;
                if (frame_us > 400000) frame_us = 400000;
                /* Pace on TG INTER so we actually get a new SOF. */
                unsigned i0 = rr(0x444c);
                usleep(frame_us / 2);
                for (int t = 0; t < 40 && !g_stop; t++) {
                    if (rr(0x444c) != i0) break;
                    usleep(10000);
                }
                if (g_stop) break;
            }
            /* CPU-side snapshot of the live IMGO buffer */
            {
                unsigned chunk = 256 * 1024;
                for (unsigned off = 0; off < frame_len; off += chunk) {
                    unsigned n = frame_len - off;
                    if (n > chunk) n = chunk;
                    ion_cache(handle, (unsigned char *)va + off, n,
                              ION_CACHE_INVALID_BY_RANGE);
                }
            }
            ae_update(va, stride, g_w, g_h);
            write_preview(va, stride, g_w, g_h);
            fr++;
            if ((fr & 3) == 0)
                fprintf(stderr, "stream: fr=%u shut=%u gain=%.1fx mean=%u\n",
                        fr, g_ae_shut, g_ae_gain / 64.0, g_last_mean);
        }
        fprintf(stderr, "stream: stopped after %u frames\n", fr);
    }

    /* Diagnostic dump of the half-frame stop condition (DMA/FBC/err/format),
     * read while we still hold the ISP fd (clocks gate the moment it closes). */
    if (!g_stream)
        fprintf(stderr,
                "HALF-DIAG: INT=0x%08x DMAINT=0x%08x INTX=0x%08x IMGO_ERR=0x%08x "
                "FBC=0x%08x FMT=0x%08x TG_SEN_MODE=0x%08x TG_PATH=0x%08x "
                "FRMSIZE=0x%08x GRAB_LIN=0x%08x IMGO_YSIZE=0x%08x IMGO_STRIDE=0x%08x "
                "CROP_X=0x%08x CROP_Y=0x%08x\n",
                rr(0x4024u), rr(0x4028u), rr(0x4044u), rr(0x43ACu),
                rr(R_IMGO_FBC), rr(R_CTL_FMT_SEL), rr(R_TG_SEN_MODE), rr(R_TG_PATH_CFG),
                rr(R_TG_FRMSIZE), rr(R_TG_GRAB_LIN), rr(R_IMGO_YSIZE), rr(R_IMGO_STRIDE),
                rr(0x4110u), rr(0x4114u));
    /* Optional hold so /proc/driver/isp_reg can be read live from another shell. */
    {
        const char *h = getenv("CAMGRAB_HOLD");
        if (h && atoi(h) > 0) {
            fprintf(stderr, "HOLD %ss (ISP fd open for external isp_reg dump)...\n", h);
            sleep((unsigned)atoi(h));
        }
    }

    /* 7. stop streaming hardware */
    rw(R_TG_VF_CON, rr(R_TG_VF_CON) & ~0x1u);
    rw(R_CTL_DMA_EN, rr(R_CTL_DMA_EN) & ~0x1u);

    munmap(va, FRAME_LEN);
    close(mapfd);
    close(m4u);
    ioctl(sensor, IOC_T_CLOSE);
    close(sensor);
    close(isp);
    close(ionfd);
    return got ? 0 : 2;
}
