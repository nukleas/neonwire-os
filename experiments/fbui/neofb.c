/*
 * neofb — cyberpunk framebuffer HUD for the DL7006 custom OS.
 *
 * Paints a neon system dashboard onto /dev/fb0 (mtkfb, 32bpp) from live /proc.
 * Pure syscalls + libc; no X, no GL. Shared primitives live in fbgfx.h.
 *
 *   neofb              one-shot render, exit
 *   neofb --loop [s]   redraw every s seconds (default 1) until killed
 *   neofb --shot PATH  also dump the pane raw to PATH (for host screenshot)
 *
 * Build: armv7l-linux-musleabihf-gcc -Os -static -no-pie -o neofb neofb.c
 */
#include <sys/utsname.h>
#include "fbgfx.h"

static void scene(void) {
    background();

    /* outer HUD frame + corner brackets */
    neonbox(10, 10, xres - 20, yres - 20, mix(BG, CYAN, 60));
    corners(10, 10, xres - 20, yres - 20, CYAN, 22);

    struct utsname un; uname(&un);
    char host[64]; gethostname(host, sizeof host);
    if (!host[0] || !strcmp(host, "(none)")) strcpy(host, "dl7006");

    /* ---- header ---- */
    textg(28, 22, "NEONWIRE", CYAN, 3);
    textg(28 + FONT_W * 3 * 8, 22, "// DL-7006", MAGENTA, 3);
    text(30, 22 + FONT_H * 3 + 6, "mediatek mt8127  ::  cyberpunk shell  ::  self-built linux", TEXT2, 1);
    hline(28, 118, xres - 56, CYAN);
    hline(28, 120, xres - 56, mix(BG, CYAN, 60));
    text(xres - 28 - 9 * FONT_W, 22, "SYS ONLINE", GREEN, 1);

    int top = 152, colw = (xres - 84) / 2, lx = 28, rx = 28 + colw + 28;

    /* ---- left: SYSTEM ---- */
    panel(lx, top, colw, 300, CYAN, "[ SYSTEM ]");
    char line[256]; int y = top + 30, lh = FONT_H + 8;
    char model[64]; int ncpu = cpu_count(model, sizeof model);

    snprintf(line, sizeof line, "host    %s", host);
    text(lx + 18, y, line, WHITE, 1); y += lh;
    snprintf(line, sizeof line, "kernel  %s", un.release);
    text(lx + 18, y, line, TEXT, 1); y += lh;
    snprintf(line, sizeof line, "arch    %s", un.machine);
    text(lx + 18, y, line, TEXT, 1); y += lh;
    snprintf(line, sizeof line, "cpu     %d core  %s", ncpu, model[0] ? model : "ARMv7");
    text(lx + 18, y, line, TEXT, 1); y += lh;

    double up = uptime_s();
    int uh = (int)up / 3600, um = ((int)up % 3600) / 60, us = (int)up % 60;
    snprintf(line, sizeof line, "uptime  %02d:%02d:%02d", uh, um, us);
    text(lx + 18, y, line, TEXT, 1); y += lh;
    char la[64]; loadavg(la, sizeof la);
    snprintf(line, sizeof line, "load    %s", la);
    text(lx + 18, y, line, TEXT, 1); y += lh + 6;

    long mt = meminfo("MemTotal:"), ma = meminfo("MemAvailable:");
    long used = mt - ma; int mpct = mt ? (int)(used * 100 / mt) : 0;
    snprintf(line, sizeof line, "mem     %ld / %ld MB  (%d%%)", used / 1024, mt / 1024, mpct);
    text(lx + 18, y, line, TEXT, 1); y += lh - 2;
    bar(lx + 18, y, colw - 40, 16, mpct, CYAN);

    /* ---- right: SUBSYSTEMS ---- */
    panel(rx, top, colw, 300, MAGENTA, "[ SUBSYSTEMS ]");
    y = top + 30;
    struct { const char *k; const char *v; uint32_t c; } st[] = {
        {"kernel      3.18.35", "ONLINE",  GREEN},
        {"framebuffer mtkfb",   "ONLINE",  GREEN},
        {"usb-acm     ttyGS0",  "ONLINE",  GREEN},
        {"storage     emmc+sd", "ONLINE",  GREEN},
        {"touch       icn85xx", "READY",   AMBER},
        {"wifi        consys",  "OFFLINE", MAGENTA},
    };
    for (unsigned i = 0; i < sizeof st / sizeof st[0]; i++) {
        text(rx + 18, y, st[i].k, TEXT, 1);
        int vx = rx + colw - 18 - (int)strlen(st[i].v) * FONT_W;
        fill(vx - 16, y + 6, 8, 8, st[i].c);
        text(vx, y, st[i].v, st[i].c, 1);
        y += lh;
    }
    y += 10;
    text(rx + 18, y, "> booting neon subsystems ..._", TEXT2, 1);

    /* ---- bottom status bar ---- */
    int by = yres - 40;
    fill(12, by, xres - 24, 28, mix(BG, CYAN, 14));
    hline(12, by, xres - 24, CYAN);
    text(24, by + 5, "root@dl7006:~#", GREEN, 1);
    snprintf(line, sizeof line, "up %02d:%02d:%02d", uh, um, us);
    text(xres - 24 - (int)strlen(line) * FONT_W, by + 5, line, CYAN, 1);
    textg(xres / 2 - 8 * FONT_W, by + 5, "[ NEONWIRE OS v0.1 ]", MAGENTA, 1);

    scanlines(0, 0, xres, yres);
}

int main(int argc, char **argv) {
    int loop = 0, interval = 1;
    const char *shot = NULL;
    for (int i = 1; i < argc; i++) {
        if (!strcmp(argv[i], "--loop")) { loop = 1; if (i + 1 < argc && argv[i+1][0] != '-') interval = atoi(argv[++i]); }
        else if (!strcmp(argv[i], "--shot") && i + 1 < argc) shot = argv[++i];
    }

    if (fb_open() < 0) return 1;
    printf("SHOT w=%u h=%u stride=%u r=%u g=%u b=%u\n",
           xres, yres, stride, vi.red.offset, vi.green.offset, vi.blue.offset);

    do {
        scene();
        fb_present();
        if (loop) sleep(interval);
    } while (loop);

    if (shot) fb_shot(shot);
    return 0;
}
