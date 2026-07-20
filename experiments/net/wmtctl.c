/* wmtctl — minimal native WMT launcher for L1.
 *
 * The stock Android /vendor/bin/wmt_launcher runs but stays silent on L1 (it blocks
 * on Android's property service / socket that we don't have), so it never issues the
 * SET_STP_MODE ioctl that establishes the STP-over-SDIO transport handle. Without it,
 * turning on Wi-Fi fails with "CTRL_STP_ENABLE but invalid Handle of WmtStp".
 *
 * This does exactly the ioctls wmt_launcher would, decoded from the 3.18 conn_soc
 * driver source (wmt_dev.c / wmt_lib.c):
 *   WMT_IOCTL_SET_STP_MODE   arg = (FM<<4)|IF  = (WMT_FM_COMM=2)<<4 | (STP_SDIO=0x04) = 0x24
 *   WMT_IOCTL_FUNC_ONOFF_CTRL arg = 0x80000000 | WMTDRV_TYPE  (WMT=4 whole-chip, WIFI=3)
 * Keeps /dev/stpwmt open (daemon) so the transport/power stays up.
 *
 *   wmtctl            # set SDIO mode, power whole chip on, then Wi-Fi on, stay resident
 *   wmtctl off        # Wi-Fi off + whole chip off, exit
 */
#include <stdio.h>
#include <fcntl.h>
#include <unistd.h>
#include <string.h>
#include <errno.h>
#include <sys/ioctl.h>

#define WMT_IOC_MAGIC 0xa0
#define WMT_IOCTL_SET_STP_MODE      _IOW(WMT_IOC_MAGIC, 5, int)
#define WMT_IOCTL_FUNC_ONOFF_CTRL   _IOW(WMT_IOC_MAGIC, 6, int)

#define HIFCONF_SDIO_FMCOMM  0x24          /* (WMT_FM_COMM<<4) | STP_SDIO */
#define FUNC_ON(type)        (0x80000000 | (type))
#define FUNC_OFF(type)       (0x00000000 | (type))
#define TYPE_WIFI 3
#define TYPE_WMT  4

static int step(int fd, unsigned long req, int arg, const char *name) {
    int r = ioctl(fd, req, arg);
    printf("  %-22s arg=0x%08x -> ret=%d%s\n", name, (unsigned)arg, r,
           r ? " (errno " : "");
    if (r) printf("%d)\n", errno);
    return r;
}

int main(int argc, char **argv) {
    int off = (argc > 1 && strcmp(argv[1], "off") == 0);
    int fd = open("/dev/stpwmt", O_RDWR);
    if (fd < 0) { printf("open /dev/stpwmt failed: errno %d\n", errno); return 1; }

    if (off) {
        printf("wmtctl: powering OFF\n");
        step(fd, WMT_IOCTL_FUNC_ONOFF_CTRL, FUNC_OFF(TYPE_WIFI), "FUNC_OFF(WIFI)");
        step(fd, WMT_IOCTL_FUNC_ONOFF_CTRL, FUNC_OFF(TYPE_WMT),  "FUNC_OFF(WMT)");
        close(fd);
        return 0;
    }

    printf("wmtctl: bring-up on /dev/stpwmt\n");
    step(fd, WMT_IOCTL_SET_STP_MODE,    HIFCONF_SDIO_FMCOMM,  "SET_STP_MODE(SDIO)");
    step(fd, WMT_IOCTL_FUNC_ONOFF_CTRL, FUNC_ON(TYPE_WMT),    "FUNC_ON(WMT)");
    step(fd, WMT_IOCTL_FUNC_ONOFF_CTRL, FUNC_ON(TYPE_WIFI),   "FUNC_ON(WIFI)");
    printf("wmtctl: resident (holding /dev/stpwmt open). Ctrl-C or `wmtctl off` to stop.\n");
    fflush(stdout);
    pause();                 /* keep fd open so transport/power persists */
    close(fd);
    return 0;
}
