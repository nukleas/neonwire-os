/* wmtctl_min — freestanding (no libc) ARM WMT launcher. Tiny, for easy serial push.
 * Does the ioctls the stalled wmt_launcher won't: SET_STP_MODE(BTIF) then power on.
 *   SET_STP_MODE     = _IOW(0xa0,5,int) = 0x4004a005, arg 0x23 (FM_COMM<<4 | STP_BTIF_FULL)
 *     !! 0x24 (STP_SDIO) was WRONG for MT8127 SOC consys — the on-die chip has no SDIO
 *     function ("hif_sdio_stp_on: no supported func probed"). conn_soc wmt_dev.h:
 *     STP_UART_FULL=0x01 STP_BTIF_FULL=0x03 STP_SDIO=0x04; wmt_ctrl_stp_conf opens
 *     the STP link via mtk_wcn_stp_open_btif() — BTIF is the SOC transport.
 *   FUNC_ONOFF_CTRL  = _IOW(0xa0,6,int) = 0x4004a006, arg 0x8000000|type (WMT=4,WIFI=3)
 * Build: arm gcc -nostdlib -static -Os -o wmtctl_min wmtctl_min.c
 */
#define SYS_exit 1
#define SYS_write 4
#define SYS_open 5
#define SYS_ioctl 54
#define SYS_pause 29

static long syscall3(long n, long a, long b, long c) {
    register long r7 __asm__("r7") = n;
    register long r0 __asm__("r0") = a;
    register long r1 __asm__("r1") = b;
    register long r2 __asm__("r2") = c;
    __asm__ volatile("svc 0" : "+r"(r0) : "r"(r7), "r"(r1), "r"(r2) : "memory");
    return r0;
}
static void say(const char *s) { long n = 0; while (s[n]) n++; syscall3(SYS_write, 1, (long)s, n); }
static void sayhex(long v) {
    char b[11]; int i; b[0]='0'; b[1]='x';
    for (i = 0; i < 8; i++) { int nib = (v >> ((7-i)*4)) & 0xF; b[2+i] = nib < 10 ? '0'+nib : 'a'+nib-10; }
    b[10] = '\n'; syscall3(SYS_write, 1, (long)b, 11);
}

void _start(void) {
    long fd = syscall3(SYS_open, (long)"/dev/stpwmt", 2 /*O_RDWR*/, 0);
    if (fd < 0) { say("open /dev/stpwmt FAILED\n"); syscall3(SYS_exit, 1, 0, 0); }
    say("SET_STP_MODE(BTIF) ret="); sayhex(syscall3(SYS_ioctl, fd, 0x4004a005, 0x23));
    say("FUNC_ON(WMT)  ret=");      sayhex(syscall3(SYS_ioctl, fd, 0x4004a006, 0x80000004));
    say("FUNC_ON(WIFI) ret=");      sayhex(syscall3(SYS_ioctl, fd, 0x4004a006, 0x80000003));
    say("resident (holding stpwmt open)\n");
    syscall3(SYS_pause, 0, 0, 0);
    syscall3(SYS_exit, 0, 0, 0);
}
