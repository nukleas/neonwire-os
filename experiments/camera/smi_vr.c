/* smi_vr — toggle the SMI bandwidth "VR" (camera) scenario on the MT8127.
 *
 * Hypothesis (from narnia smi_common.c): the stock camera path switches the SMI
 * profile to SMI_BWC_SCEN_VR, which raises CAM_IMGO's larb2 bandwidth limiter
 * from 1 -> 6 and boosts larb2's L1 grant to 0xD4F.  On NeonOS/L1 nothing ever
 * leaves the NORMAL profile, so IMGO drain is throttled and a full 1592-wide
 * RAW10 line overruns the IMGO FIFO at exactly h/2 (INTX DMA_ERR).
 *
 * ABI (mt_smi.h, matches the 3.18.35 ODM kernel):
 *   MTK_SMI_BWC_CONFIG { int scenario; int b_on_off; }   scenario VR = 1
 *   MTK_IOC_SMI_BWC_CONFIG = _IOW('O', 24, MTK_SMI_BWC_CONFIG)
 *
 * Usage: smi_vr on|off|state
 */
#include <stdio.h>
#include <string.h>
#include <fcntl.h>
#include <unistd.h>
#include <sys/ioctl.h>

#define SMI_BWC_SCEN_VR 1

struct smi_bwc_config {
    int scenario;
    int b_on_off;
};

#define MTK_IOC_SMI_BWC_CONFIG _IOW('O', 24, struct smi_bwc_config)

int main(int argc, char **argv)
{
    if (argc < 2) {
        fprintf(stderr, "usage: %s on|off\n", argv[0]);
        return 2;
    }
    int on;
    if (!strcmp(argv[1], "on")) on = 1;
    else if (!strcmp(argv[1], "off")) on = 0;
    else {
        fprintf(stderr, "usage: %s on|off\n", argv[0]);
        return 2;
    }

    int fd = open("/dev/MTK_SMI", O_RDWR);
    if (fd < 0) {
        perror("open /dev/MTK_SMI");
        return 1;
    }
    struct smi_bwc_config cfg = { SMI_BWC_SCEN_VR, on };
    if (ioctl(fd, MTK_IOC_SMI_BWC_CONFIG, &cfg) < 0) {
        perror("MTK_IOC_SMI_BWC_CONFIG");
        close(fd);
        return 1;
    }
    close(fd);
    printf("SMI VR scenario %s OK\n", on ? "ON" : "OFF");
    return 0;
}
