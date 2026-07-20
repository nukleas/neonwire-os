/* ion_probe — can we allocate a DMA frame buffer on L1 without /dev/mem?
 *
 * /dev/mem is STRICT_DEVMEM-blocked and there's no /dev/m4u node, but /dev/ion
 * exists. This tests the MTK ion path the stock camera HAL uses: ION_IOC_ALLOC
 * from the multimedia heap, then the ION_IOC_CUSTOM ION_SYS_GET_PHYS extension
 * to obtain the physical/MVA address we'd program into the ISP IMGO DMA base.
 * If this returns a plausible DRAM address (>=0x80000000 on MT8127), A3 (raw
 * frame capture) has a viable buffer. ABI from include/linux/{mtk_ion,ion_drv}.h
 * and drivers/staging/android/uapi/ion.h.
 *
 * Build: armv7l-linux-musleabihf-gcc -Os -static -o ion_probe ion_probe.c
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

/* --- staging/android/uapi/ion.h --- */
#define ION_IOC_MAGIC 'I'
typedef int ion_user_handle_t;
struct ion_allocation_data { size_t len, align; unsigned int heap_id_mask, flags; ion_user_handle_t handle; };
struct ion_fd_data { ion_user_handle_t handle; int fd; };
struct ion_handle_data { ion_user_handle_t handle; };
struct ion_custom_data { unsigned int cmd; unsigned long arg; };
#define ION_IOC_ALLOC  _IOWR(ION_IOC_MAGIC, 0, struct ion_allocation_data)
#define ION_IOC_FREE   _IOWR(ION_IOC_MAGIC, 1, struct ion_handle_data)
#define ION_IOC_MAP    _IOWR(ION_IOC_MAGIC, 2, struct ion_fd_data)
#define ION_IOC_CUSTOM _IOWR(ION_IOC_MAGIC, 6, struct ion_custom_data)

/* --- include/linux/mtk_ion.h + ion_drv.h --- */
#define ION_HEAP_MULTIMEDIA_MASK (1u << 10)
enum { ION_CMD_SYSTEM = 0, ION_CMD_MULTIMEDIA = 1 };
enum { ION_MM_CONFIG_BUFFER = 0 };
enum { ION_SYS_GET_PHYS = 1 };

/* The kernel copy_from_user's the FULL ion_{sys,mm}_data_t (a big union), so
 * these must be at least as large or copy_from_user EFAULTs. Field offsets:
 * sys_cmd@0, then the union at @4 (handle@4, phy_addr@8, len@12). Pad to 256. */
typedef struct {
    unsigned int sys_cmd;    /* @0 */
    ion_user_handle_t handle;/* @4 (union base) */
    unsigned int phy_addr;   /* @8 */
    unsigned int len;        /* @12 */
    unsigned char pad[240];
} ion_sys_data_t;
typedef struct {
    unsigned int mm_cmd;     /* @0 */
    ion_user_handle_t handle;/* @4 (union base) */
    int eModuleID;           /* @8 */
    unsigned int security;   /* @12 */
    unsigned int coherent;   /* @16 */
    unsigned char pad[236];
} ion_mm_data_t;

#define FRAME_LEN (1600 * 1200 * 5 / 4)  /* 1600x1200 RAW10 packed = 2.4MB */

int main(int argc, char **argv)
{
    int module = argc > 1 ? atoi(argv[1]) : -1; /* optional M4U port for CONFIG_BUFFER */
    int fd = open("/dev/ion", O_RDWR);
    if (fd < 0) { printf("no /dev/ion: %s\n", strerror(errno)); return 1; }

    struct ion_allocation_data ad = { .len = FRAME_LEN, .align = 0,
                                      .heap_id_mask = ION_HEAP_MULTIMEDIA_MASK, .flags = 0 };
    if (ioctl(fd, ION_IOC_ALLOC, &ad) < 0) { printf("ALLOC fail: %s\n", strerror(errno)); return 1; }
    printf("ALLOC ok: handle=%d len=%u\n", ad.handle, (unsigned)FRAME_LEN);

    /* optional: configure the buffer for an M4U port (needed if M4U enforces) */
    if (module >= 0) {
        ion_mm_data_t mm; memset(&mm, 0, sizeof mm);
        mm.mm_cmd = ION_MM_CONFIG_BUFFER;
        mm.handle = ad.handle; mm.eModuleID = module;
        struct ion_custom_data cd = { .cmd = ION_CMD_MULTIMEDIA, .arg = (unsigned long)&mm };
        printf("CONFIG_BUFFER(port=%d): %s\n", module,
               ioctl(fd, ION_IOC_CUSTOM, &cd) < 0 ? strerror(errno) : "ok");
    }

    /* GET_PHYS: the address the ISP DMA would use */
    ion_sys_data_t sd; memset(&sd, 0, sizeof sd);
    sd.sys_cmd = ION_SYS_GET_PHYS; sd.handle = ad.handle;
    struct ion_custom_data cd = { .cmd = ION_CMD_SYSTEM, .arg = (unsigned long)&sd };
    if (ioctl(fd, ION_IOC_CUSTOM, &cd) < 0) {
        printf("GET_PHYS fail: %s\n", strerror(errno));
    } else {
        printf("GET_PHYS ok: phys=0x%08x len=%u %s\n", sd.phy_addr, sd.len,
               sd.phy_addr >= 0x40000000u ? "(plausible DRAM - IMGO target viable!)"
                                          : "(low addr - MVA or needs config)");
    }

    /* verify CPU can map it too (fill test pattern later) */
    struct ion_fd_data mfd = { .handle = ad.handle };
    if (ioctl(fd, ION_IOC_MAP, &mfd) == 0 && mfd.fd >= 0) {
        void *va = mmap(0, FRAME_LEN, PROT_READ | PROT_WRITE, MAP_SHARED, mfd.fd, 0);
        printf("MAP ok fd=%d mmap=%s\n", mfd.fd, va == MAP_FAILED ? strerror(errno) : "ok");
        if (va != MAP_FAILED) munmap(va, FRAME_LEN);
        close(mfd.fd);
    } else {
        printf("MAP fail: %s\n", strerror(errno));
    }

    struct ion_handle_data hd = { .handle = ad.handle };
    ioctl(fd, ION_IOC_FREE, &hd);
    close(fd);
    return 0;
}
