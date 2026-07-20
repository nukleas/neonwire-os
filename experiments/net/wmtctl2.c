/* wmtctl2 — freestanding ARM WMT launcher v2: BTIF mode + firmware patch registration.
 *
 * Supersedes wmtctl_min. Two fixes over v1, both decoded from kernel conn_soc source
 * (reference/upstream/kernel_amazon_mt8127-common/.../conn_soc) and a disassembly of
 * the stock /vendor/bin/wmt_launcher (reference/firmware/work/wifi-extract):
 *
 * 1) STP mode 0x23 = (WMT_FM_COMM=2)<<4 | STP_BTIF_FULL(0x03).  v1 used 0x24 = STP_SDIO,
 *    which is for EXTERNAL combo chips — MT8127's on-die consys has no SDIO function
 *    ("hif_sdio_stp_on: no supported func probed"). BTIF is the SOC transport
 *    (wmt_ctrl_stp_conf -> mtk_wcn_stp_open_btif).
 *
 * 2) The kernel's patch download asks USERSPACE for patch info: mtk_wcn_soc_patch_info_prepare
 *    posts "srh_patch" (readable on /dev/stpwmt, 2s timeout) and expects the launcher to have
 *    issued SET_PATCH_NUM + SET_PATCH_INFO, then write "ok" back.  Stock launcher algorithm
 *    (from disassembly @0x16d4..0x1898): for each ROMv2_patch_*_hdr.bin, read 4 bytes at
 *    offset 24: b0>>4 = total patch count, b0&0xf = download seq, addRess = {0,b1,b2,b3}
 *    (byte0 zeroed).  patchName must be a FULL path (kernel filp_opens it verbatim).
 *
 * ioctls (_IOW(0xa0,N,int)): SET_STP_MODE=0x4004a005  FUNC_ONOFF=0x4004a006
 *                            SET_PATCH_NUM=0x4004a00e SET_PATCH_INFO=0x4004a00f (ptr arg)
 * FUNC_ONOFF arg = 0x80000000|type, WMT=4 (whole-chip power-on + patch dl), WIFI=3 (wlan probe).
 *
 * Flow: open stpwmt -> set mode -> parse+register both patches -> fork responder child
 * (reads /dev/stpwmt cmds like srh_patch/close_stp, logs, acks "ok") -> FUNC_ON(WMT)
 * -> FUNC_ON(WIFI) -> stay resident.
 *
 * Build: armv7l-linux-musleabihf-gcc -nostdlib -static -no-pie -Os -o wmtctl2 wmtctl2.c
 */
#define SYS_exit 1
#define SYS_fork 2
#define SYS_read 3
#define SYS_write 4
#define SYS_open 5
#define SYS_close 6
#define SYS_lseek 19
#define SYS_pause 29
#define SYS_ioctl 54
#define SYS_nanosleep 162

static long sc3(long n, long a, long b, long c) {
    register long r7 __asm__("r7") = n;
    register long r0 __asm__("r0") = a;
    register long r1 __asm__("r1") = b;
    register long r2 __asm__("r2") = c;
    __asm__ volatile("svc 0" : "+r"(r0) : "r"(r7), "r"(r1), "r"(r2) : "memory");
    return r0;
}
static void say(const char *s) { long n = 0; while (s[n]) n++; sc3(SYS_write, 1, (long)s, n); }
static void sayhex(long v) {
    char b[11]; int i; b[0]='0'; b[1]='x';
    for (i = 0; i < 8; i++) { int nib = (v >> ((7-i)*4)) & 0xF; b[2+i] = nib < 10 ? '0'+nib : 'a'+nib-10; }
    b[10] = '\n'; sc3(SYS_write, 1, (long)b, 11);
}
static void msleep(int ms) {
    long ts[2]; ts[0] = ms / 1000; ts[1] = (ms % 1000) * 1000000L;
    sc3(SYS_nanosleep, (long)ts, 0, 0);
}

struct pinfo { unsigned int seq; unsigned char addr[4]; char name[256]; };

/* gcc emits memset calls for struct zeroing even with -nostdlib */
void *memset(void *d, int c, unsigned long n) {
    unsigned char *p = d;
    while (n--) *p++ = (unsigned char)c;
    return d;
}

static const char *PATCHES[2] = {
    "/etc/firmware/ROMv2_patch_1_0_hdr.bin",
    "/etc/firmware/ROMv2_patch_1_1_hdr.bin",
};

void _start(void) {
    long fd = sc3(SYS_open, (long)"/dev/stpwmt", 2 /*O_RDWR*/, 0);
    if (fd < 0) { say("open /dev/stpwmt FAILED\n"); sc3(SYS_exit, 1, 0, 0); }

    say("SET_STP_MODE(BTIF 0x23) ret="); sayhex(sc3(SYS_ioctl, fd, 0x4004a005, 0x23));

    /* parse patch headers: 4 bytes at offset 24 of each _hdr.bin */
    unsigned char hb[2][4];
    int i, num = 0;
    for (i = 0; i < 2; i++) {
        long pf = sc3(SYS_open, (long)PATCHES[i], 0 /*O_RDONLY*/, 0);
        if (pf < 0) { say("patch open FAILED: "); say(PATCHES[i]); say("\n"); sc3(SYS_exit, 2, 0, 0); }
        sc3(SYS_lseek, pf, 24, 0);
        if (sc3(SYS_read, pf, (long)hb[i], 4) != 4) { say("patch hdr read FAILED\n"); sc3(SYS_exit, 3, 0, 0); }
        sc3(SYS_close, pf, 0, 0);
    }
    num = hb[0][0] >> 4;
    say("patch num="); sayhex(num);
    say("SET_PATCH_NUM ret="); sayhex(sc3(SYS_ioctl, fd, 0x4004a00e, num));

    for (i = 0; i < 2; i++) {
        struct pinfo pi;
        long j;
        char *p = (char *)&pi;
        for (j = 0; j < (long)sizeof(pi); j++) p[j] = 0;
        pi.seq = hb[i][0] & 0xF;
        pi.addr[0] = 0; pi.addr[1] = hb[i][1]; pi.addr[2] = hb[i][2]; pi.addr[3] = hb[i][3];
        for (j = 0; PATCHES[i][j]; j++) pi.name[j] = PATCHES[i][j];
        say("SET_PATCH_INFO seq="); sayhex(pi.seq);
        say("  ret="); sayhex(sc3(SYS_ioctl, fd, 0x4004a00f, (long)&pi));
    }

    /* responder: ack kernel ul-cmds (srh_patch, stale close_stp, ...) with "ok" */
    long pid = sc3(SYS_fork, 0, 0, 0);
    if (pid == 0) {
        char buf[260];
        for (;;) {
            long n = sc3(SYS_read, fd, (long)buf, 256);
            if (n > 0) {
                buf[n] = 0;
                say("ul-cmd: "); say(buf); say(" -> ok\n");
                sc3(SYS_write, fd, (long)"ok", 2);
            }
            msleep(50);
        }
    }

    msleep(300);
    say("FUNC_ON(WMT)  ret="); sayhex(sc3(SYS_ioctl, fd, 0x4004a006, 0x80000004));
    say("FUNC_ON(WIFI) ret="); sayhex(sc3(SYS_ioctl, fd, 0x4004a006, 0x80000003));
    say("resident (holding stpwmt open)\n");
    for (;;) sc3(SYS_pause, 0, 0, 0);
}
