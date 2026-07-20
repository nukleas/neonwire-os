/*
 * fbgfx.h — shared cyberpunk framebuffer primitives for the DL7006 OS UI.
 *
 * Palette + drawing (glyphs, glow, panels, scanlines) + fb lifecycle, shared by
 * neofb (dashboard) and neui (touch launcher). Palette follows cyberdesign tokens.
 * Header-only, all `static` — include once per translation unit.
 */
#ifndef FBGFX_H
#define FBGFX_H

#ifndef _GNU_SOURCE
#define _GNU_SOURCE
#endif
#include <fcntl.h>
#include <linux/fb.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <unistd.h>

#include "font_neon.h"

/* ---- palette: cyberdesign tokens (0xRRGGBB) ---- */
#define BG        0x05060a   /* bgBase */
#define BG2       0x0b0f18   /* bgPanel */
#define PANEL     0x0b0f18   /* panel fill */
#define BORDER    0x26324c   /* borderMid */
#define CYAN      0x47f6ff
#define CYANHI    0xbdffff   /* cyanBright */
#define MAGENTA   0xff2bd6
#define GREEN     0x52ff9f   /* greenBright */
#define AMBER     0xffaa00
#define GOLD      0xffcc00
#define PURPLE    0x9d7cff
#define REDA      0xff456c   /* redAlt */
#define DIM       0x26324c
#define TEXT      0xe8e4e0   /* textPrimary */
#define TEXT2     0xa7b7d6   /* textSecondary */
#define TEXTDIM   0x6a625c   /* textDim */
#define WHITE     0xf2f7ff   /* textPrimaryAlt */
#define GRID      0x47f6ff   /* pixel-grid tint (applied faintly) */

static struct fb_var_screeninfo vi;
static struct fb_fix_screeninfo fi;
static uint8_t *fbmem;
static uint8_t *back;         /* off-screen composite buffer */
static uint32_t xres, yres, stride;
static int fb_fd = -1;        /* kept for pan-flip on present */
static int fb_nbuf = 1;       /* panels available in the virtual fb */
static int fb_cur;            /* current back-buffer index */

/* pack an 0xRRGGBB into the fb's native 32bpp layout */
static inline uint32_t pack(uint32_t rgb) {
    uint32_t r = (rgb >> 16) & 0xff, g = (rgb >> 8) & 0xff, b = rgb & 0xff;
    return ((r >> (8 - vi.red.length)) << vi.red.offset) |
           ((g >> (8 - vi.green.length)) << vi.green.offset) |
           ((b >> (8 - vi.blue.length)) << vi.blue.offset) |
           (vi.transp.length ? (0xffu >> (8 - vi.transp.length)) << vi.transp.offset : 0);
}

/* unpack a native fb pixel back to 0xRRGGBB */
static inline uint32_t unpack(uint32_t v) {
    uint32_t r = (v >> vi.red.offset) & ((1u << vi.red.length) - 1);
    uint32_t g = (v >> vi.green.offset) & ((1u << vi.green.length) - 1);
    uint32_t b = (v >> vi.blue.offset) & ((1u << vi.blue.length) - 1);
    r <<= (8 - vi.red.length); g <<= (8 - vi.green.length); b <<= (8 - vi.blue.length);
    return (r << 16) | (g << 8) | b;
}

static inline void px(int x, int y, uint32_t rgb) {
    if ((unsigned)x >= xres || (unsigned)y >= yres) return;
    *(uint32_t *)(back + y * stride + x * 4) = pack(rgb);
}

/* alpha 0..255 blend of a toward b */
static inline uint32_t mix(uint32_t a, uint32_t b, int t) {
    int ar = (a >> 16) & 0xff, ag = (a >> 8) & 0xff, ab = a & 0xff;
    int br = (b >> 16) & 0xff, bg = (b >> 8) & 0xff, bb = b & 0xff;
    int r = ar + (br - ar) * t / 255;
    int g = ag + (bg - ag) * t / 255;
    int bl = ab + (bb - ab) * t / 255;
    return (r << 16) | (g << 8) | bl;
}

static void fill(int x, int y, int w, int h, uint32_t rgb) {
    for (int j = y; j < y + h; j++)
        for (int i = x; i < x + w; i++) px(i, j, rgb);
}
static void hline(int x, int y, int w, uint32_t rgb) { fill(x, y, w, 1, rgb); }
static void vline(int x, int y, int h, uint32_t rgb) { fill(x, y, 1, h, rgb); }

/* neon rectangle: bright edge + dim inner halo */
static void neonbox(int x, int y, int w, int h, uint32_t rgb) {
    uint32_t halo = mix(BG, rgb, 70);
    hline(x, y, w, rgb); hline(x, y + h - 1, w, rgb);
    vline(x, y, h, rgb); vline(x + w - 1, y, h, rgb);
    hline(x + 1, y + 1, w - 2, halo); hline(x + 1, y + h - 2, w - 2, halo);
    vline(x + 1, y + 1, h - 2, halo); vline(x + w - 2, y + 1, h - 2, halo);
}

/* one glyph, integer-scaled, alpha-blended in rgb over the background colour bg */
static void glyph_bg(int x, int y, int ch, uint32_t rgb, uint32_t bg, int scale) {
    if (ch < FONT_FIRST || ch > FONT_LAST) return;
    const unsigned char *g = font_alpha + (ch - FONT_FIRST) * FONT_W * FONT_H;
    for (int gy = 0; gy < FONT_H; gy++)
        for (int gx = 0; gx < FONT_W; gx++) {
            int a = g[gy * FONT_W + gx];
            if (!a) continue;
            for (int sy = 0; sy < scale; sy++)
                for (int sx = 0; sx < scale; sx++)
                    px(x + gx * scale + sx, y + gy * scale + sy, mix(bg, rgb, a));
        }
}
static void glyph(int x, int y, int ch, uint32_t rgb, int scale) {
    glyph_bg(x, y, ch, rgb, BG, scale);
}

static int text(int x, int y, const char *s, uint32_t rgb, int scale) {
    int cx = x;
    for (; *s; s++) {
        if (*s == '\n') { y += (FONT_H + 2) * scale; cx = x; continue; }
        glyph(cx, y, (unsigned char)*s, rgb, scale);
        cx += FONT_W * scale;
    }
    return cx;
}

/* text with a cyberdesign-style neon bloom (0 0 6px / 12px text-shadow) */
static int textg(int x, int y, const char *s, uint32_t rgb, int scale) {
    uint32_t halo = mix(BG, rgb, 90);
    int d = scale;
    text(x - d, y, s, halo, scale);
    text(x + d, y, s, halo, scale);
    text(x, y - d, s, halo, scale);
    text(x, y + d, s, halo, scale);
    return text(x, y, s, rgb, scale);
}

/* L-shaped corner brackets, brighter than the panel border */
static void corners(int x, int y, int w, int h, uint32_t rgb, int len) {
    hline(x, y, len, rgb); vline(x, y, len, rgb);
    hline(x + w - len, y, len, rgb); vline(x + w - 1, y, len, rgb);
    hline(x, y + h - 1, len, rgb); vline(x, y + h - len, len, rgb);
    hline(x + w - len, y + h - 1, len, rgb); vline(x + w - 1, y + h - len, len, rgb);
}

/* cyberdesign panel: fill + dim border + bright corner brackets + title tab */
static void panel(int x, int y, int w, int h, uint32_t accent, const char *title) {
    fill(x, y, w, h, PANEL);
    neonbox(x, y, w, h, BORDER);
    corners(x, y, w, h, accent, 16);
    if (title && *title) textg(x + 24, y - FONT_H / 2 - 1, title, accent, 1);
}

/* horizontal meter, cyan→magenta sweep with a bright cap */
static void bar(int x, int y, int w, int h, int pct, uint32_t rgb) {
    fill(x, y, w, h, mix(BG, BORDER, 60));
    neonbox(x, y, w, h, BORDER);
    if (pct < 0) pct = 0;
    if (pct > 100) pct = 100;
    int fillw = (w - 4) * pct / 100;
    for (int i = 0; i < fillw; i++)
        vline(x + 2 + i, y + 2, h - 4, mix(rgb, MAGENTA, i * 255 / (w - 4)));
    if (fillw > 0) vline(x + 1 + fillw, y + 2, h - 4, CYANHI);
}

/* CRT scanline overlay: darken 2 of every 4 rows (~18%), like cd-scanline */
static void scanlines(int x, int y, int w, int h) {
    for (int j = y; j < y + h; j++) {
        if ((j & 3) < 2) continue;
        for (int i = x; i < x + w; i++) {
            uint32_t *p = (uint32_t *)(back + j * stride + i * 4);
            px(i, j, mix(unpack(*p), 0x000000, 46));
        }
    }
}

/* faint cyan pixel grid every 24px */
static void pixelgrid(void) {
    for (uint32_t y = 0; y < yres; y += 24)
        for (uint32_t x = 0; x < xres; x++) {
            uint32_t *p = (uint32_t *)(back + y * stride + x * 4);
            px(x, y, mix(unpack(*p), GRID, 14));
        }
    for (uint32_t x = 0; x < xres; x += 24)
        for (uint32_t y = 0; y < yres; y++) {
            uint32_t *p = (uint32_t *)(back + y * stride + x * 4);
            px(x, y, mix(unpack(*p), GRID, 14));
        }
}

/* vertical gradient background */
static void background(void) {
    for (uint32_t y = 0; y < yres; y++)
        hline(0, y, xres, mix(BG, 0x070a12, y * 255 / yres));
    pixelgrid();
}

/* ---- fb lifecycle ---- */
static int fb_open(void) {
    int fd = open("/dev/fb0", O_RDWR);
    if (fd < 0) { perror("open /dev/fb0"); return -1; }
    if (ioctl(fd, FBIOGET_VSCREENINFO, &vi) || ioctl(fd, FBIOGET_FSCREENINFO, &fi)) {
        perror("ioctl"); return -1;
    }
    xres = vi.xres; yres = vi.yres; stride = fi.line_length;
    fb_fd = fd;
    fb_nbuf = vi.yres_virtual / vi.yres;          /* 3 on mtkfb (triple buffer) */
    if (fb_nbuf < 1) fb_nbuf = 1;
    vi.xoffset = 0; vi.yoffset = 0;
    ioctl(fd, FBIOPAN_DISPLAY, &vi);
    size_t fbsize = (size_t)fi.line_length * vi.yres_virtual;
    fbmem = mmap(0, fbsize, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (fbmem == MAP_FAILED) { perror("mmap"); return -1; }
    back = malloc((size_t)stride * yres);
    if (!back) { fprintf(stderr, "oom\n"); return -1; }
    return fd;
}

/* Blit the composed frame to the next hardware buffer and PAN to it.
 * KEY HARDWARE FACT: the ZS070BE3019B3H7II panel is a command-mode MIPI panel.
 * mtkfb only pushes a frame to the glass on FBIOPAN_DISPLAY, and skips the flush
 * when the offset is unchanged — a plain memcpy into the mmap updates memory but
 * NOT the screen. So we cycle through the 3 hardware buffers every present to
 * force a real refresh. (This is why memory screenshots looked live while the
 * physical panel sat frozen.) */
static void fb_present(void) {
    if (fb_nbuf > 1) fb_cur = (fb_cur + 1) % fb_nbuf; else fb_cur = 0;
    uint32_t yoff = (uint32_t)fb_cur * yres;
    memcpy(fbmem + (size_t)yoff * stride, back, (size_t)stride * yres);
    if (fb_fd >= 0) {
        vi.xoffset = 0; vi.yoffset = yoff;
        ioctl(fb_fd, FBIOPAN_DISPLAY, &vi);
    }
}

static void fb_shot(const char *path) {
    int sf = open(path, O_WRONLY | O_CREAT | O_TRUNC, 0644);
    if (sf >= 0) { if (write(sf, back, (size_t)stride * yres)) {} close(sf); }
}

/* ---- /proc + uname helpers ---- */
static long meminfo(const char *key) {
    FILE *f = fopen("/proc/meminfo", "r");
    if (!f) return 0;
    char line[128]; long v = 0;
    while (fgets(line, sizeof line, f))
        if (!strncmp(line, key, strlen(key))) { sscanf(line + strlen(key), " %ld", &v); break; }
    fclose(f);
    return v; /* kB */
}

static int cpu_count(char *model, size_t n) {
    FILE *f = fopen("/proc/cpuinfo", "r");
    int c = 0; if (model && n) model[0] = 0;
    if (!f) return 0;
    char line[256];
    while (fgets(line, sizeof line, f)) {
        if (!strncmp(line, "processor", 9)) c++;
        if (model && n && !model[0] && !strncmp(line, "Hardware", 8)) {
            char *p = strchr(line, ':');
            if (p) { p += 2; strncpy(model, p, n - 1); model[strcspn(model, "\n")] = 0; }
        }
    }
    fclose(f);
    return c;
}

static double uptime_s(void) {
    FILE *f = fopen("/proc/uptime", "r");
    double u = 0; if (f) { if (fscanf(f, "%lf", &u) != 1) u = 0; fclose(f); }
    return u;
}

static void loadavg(char *buf, size_t n) {
    FILE *f = fopen("/proc/loadavg", "r");
    buf[0] = 0;
    if (f) { if (!fgets(buf, n, f)) buf[0] = 0; fclose(f); }
    buf[strcspn(buf, "\n")] = 0;
}

#endif /* FBGFX_H */
