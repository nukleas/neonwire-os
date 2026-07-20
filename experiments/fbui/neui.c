/*
 * neui — cyberpunk touch launcher / multi-panel HUD for the DL7006 custom OS.
 *
 * A left nav rail of tiles + a live content panel, painted on /dev/fb0.
 * Touch comes from the mtk-tpd evdev node (default /dev/input/event4); axis
 * ranges are read at runtime via EVIOCGABS. Panels read real kernel state.
 *
 *   neui                       interactive; poll touch, redraw ~1Hz
 *   neui --panel N             start on panel N (0..4)
 *   neui --shot PATH [--panel N]  render one panel and exit (host screenshot)
 *   neui --probe               print touch axis ranges + live taps, exit on tap
 *   neui --dev /dev/input/eventX   touch device override
 *   neui --swap --flipx --flipy    touch calibration
 *
 * Build: armv7l-linux-musleabihf-gcc -Os -static -no-pie -o neui neui.c
 */
#include <sys/utsname.h>
#include <sys/statvfs.h>
#include <sys/klog.h>
#include <sys/reboot.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <sys/ioctl.h>
#include <net/if.h>
#include <netinet/in.h>
#include <signal.h>
#include <dirent.h>
#include <poll.h>
#include <time.h>
#include <linux/input.h>
#include "fbgfx.h"

enum { P_SYSTEM, P_PROC, P_STORAGE, P_LOG, P_NET, P_COUNT };
static const char *PNAME[P_COUNT] = { "SYSTEM", "PROCESS", "STORAGE", "KERNLOG", "NETWORK" };
static const uint32_t PACC[P_COUNT] = { CYAN, GREEN, AMBER, PURPLE, MAGENTA };

#define NAVX 16
#define NAVY 92
#define NAVW 200
#define TILEH 66
#define TILEG 12

static int cx, cy, cw, ch;   /* content rect */

/* ---------- content panels ---------- */
static void draw_system(void) {
    struct utsname un; uname(&un);
    char host[64]; gethostname(host, sizeof host);
    if (!host[0] || !strcmp(host, "(none)")) strcpy(host, "dl7006");
    char model[64]; int ncpu = cpu_count(model, sizeof model);
    double up = uptime_s();
    int uh = (int)up / 3600, um = ((int)up % 3600) / 60, us = (int)up % 60;
    char la[64]; loadavg(la, sizeof la);
    long mt = meminfo("MemTotal:"), ma = meminfo("MemAvailable:");
    long used = mt - ma; int mpct = mt ? (int)(used * 100 / mt) : 0;

    char line[256]; int x = cx + 22, y = cy + 20, lh = FONT_H + 10;
    struct { const char *k; const char *v; } row[] = {
        {"host",   host}, {"kernel", un.release}, {"arch", un.machine},
    };
    for (unsigned i = 0; i < 3; i++) {
        text(x, y, row[i].k, TEXT2, 1);
        text(x + 120, y, row[i].v, WHITE, 1); y += lh;
    }
    snprintf(line, sizeof line, "%d core   %s", ncpu, model[0] ? model : "ARMv7");
    text(x, y, "cpu", TEXT2, 1); text(x + 120, y, line, TEXT, 1); y += lh;
    snprintf(line, sizeof line, "%02d:%02d:%02d", uh, um, us);
    text(x, y, "uptime", TEXT2, 1); text(x + 120, y, line, TEXT, 1); y += lh;
    text(x, y, "load", TEXT2, 1); text(x + 120, y, la, TEXT, 1); y += lh + 8;

    snprintf(line, sizeof line, "memory   %ld / %ld MB   (%d%%)", used / 1024, mt / 1024, mpct);
    text(x, y, line, TEXT, 1); y += lh - 2;
    bar(x, y, cw - 90, 18, mpct, CYAN);
}

/* parse one /proc/<pid>/stat: name, state, rss pages */
static int read_stat(const char *pid, char *name, char *st, long *rss) {
    char path[64], buf[512];
    snprintf(path, sizeof path, "/proc/%s/stat", pid);
    FILE *f = fopen(path, "r"); if (!f) return 0;
    int n = fread(buf, 1, sizeof buf - 1, f); fclose(f);
    if (n <= 0) return 0;
    buf[n] = 0;
    char *lp = strchr(buf, '('), *rp = strrchr(buf, ')');
    if (!lp || !rp) return 0;
    int len = rp - lp - 1; if (len > 31) len = 31;
    memcpy(name, lp + 1, len); name[len] = 0;
    /* after ") " come: state ppid ... ; rss is field 24 overall => 22nd after ')' */
    char *p = rp + 2; *st = *p;
    for (int i = 0; i < 21; i++) { p = strchr(p, ' '); if (!p) return 1; p++; }
    *rss = atol(p);
    return 1;
}

struct proc { char name[32]; char st; long rss; int pid; };
static void draw_proc(void) {
    struct proc ps[256]; int n = 0;
    DIR *d = opendir("/proc"); struct dirent *e;
    while (d && (e = readdir(d)) && n < 256) {
        if (e->d_name[0] < '0' || e->d_name[0] > '9') continue;
        struct proc p; p.pid = atoi(e->d_name);
        if (read_stat(e->d_name, p.name, &p.st, &p.rss)) ps[n++] = p;
    }
    if (d) closedir(d);
    /* insertion sort by rss desc (n small) */
    for (int i = 1; i < n; i++) { struct proc t = ps[i]; int j = i - 1;
        while (j >= 0 && ps[j].rss < t.rss) { ps[j+1] = ps[j]; j--; } ps[j+1] = t; }

    int x = cx + 20, y = cy + 16, lh = FONT_H + 6;
    char line[128];
    snprintf(line, sizeof line, "%-6s %-4s %8s  %s", "PID", "ST", "RSS-KB", "COMMAND");
    text(x, y, line, TEXT2, 1); y += lh + 2;
    hline(x, y - 4, cw - 40, mix(BG, GREEN, 60));
    int rows = (ch - 88) / lh; if (rows > n) rows = n;
    for (int i = 0; i < rows; i++) {
        uint32_t c = ps[i].st == 'R' ? GREEN : ps[i].st == 'D' ? AMBER : TEXT;
        snprintf(line, sizeof line, "%-6d %c    %8ld  %s",
                 ps[i].pid, ps[i].st, ps[i].rss * 4, ps[i].name);
        text(x, y, line, c, 1); y += lh;
    }
    snprintf(line, sizeof line, "%d processes", n);
    text(x, cy + ch - 26, line, TEXT2, 1);
}

static void draw_storage(void) {
    FILE *f = fopen("/proc/mounts", "r");
    int x = cx + 20, y = cy + 18, lh = FONT_H + 20;
    char dev[128], mnt[128], type[32], rest[128], line[192];
    int shown = 0;
    while (f && fscanf(f, "%127s %127s %31s %127s %*d %*d\n", dev, mnt, type, rest) == 4) {
        if (strcmp(type, "ext4") && strcmp(type, "vfat") && strcmp(type, "tmpfs")) continue;
        struct statvfs vfs;
        if (statvfs(mnt, &vfs) != 0 || vfs.f_blocks == 0) continue;
        unsigned long long tot = (unsigned long long)vfs.f_blocks * vfs.f_frsize;
        unsigned long long fre = (unsigned long long)vfs.f_bfree * vfs.f_frsize;
        unsigned long long usd = tot - fre;
        int pct = tot ? (int)(usd * 100 / tot) : 0;
        snprintf(line, sizeof line, "%-14s %-6s %5llu / %-5llu MB  %d%%",
                 mnt, type, usd >> 20, tot >> 20, pct);
        text(x, y, line, WHITE, 1);
        bar(x, y + FONT_H + 2, cw - 60, 12, pct, AMBER);
        y += lh; shown++;
        if (y > cy + ch - 40) break;
    }
    if (f) fclose(f);
    if (!shown) text(x, y, "no mounted filesystems", TEXT2, 1);
}

static void draw_log(void) {
    static char buf[64 * 1024];
    int n = klogctl(3 /*READ_ALL*/, buf, sizeof buf - 1);
    int x = cx + 16, y = cy + 14, lh = FONT_H + 3;
    if (n <= 0) { text(x, y, "klogctl unavailable", TEXT2, 1); return; }
    buf[n] = 0;
    /* collect line starts, show the last screenful */
    int rows = (ch - 30) / lh;
    char *lines[512]; int ln = 0;
    for (char *p = buf; *p && ln < 512; ) {
        lines[ln++] = p;
        char *nl = strchr(p, '\n');
        if (!nl) break;
        *nl = 0; p = nl + 1;
    }
    int start = ln > rows ? ln - rows : 0;
    int maxc = (cw - 24) / FONT_W;
    for (int i = start; i < ln; i++) {
        char *s = lines[i];
        if (*s == '<') { char *g = strchr(s, '>'); if (g) s = g + 1; }  /* strip <n> prio */
        if ((int)strlen(s) > maxc) s[maxc] = 0;
        uint32_t c = TEXT2;
        if (strstr(lines[i], "fail") || strstr(lines[i], "error")) c = REDA;
        else if (strstr(lines[i], "WMT") || strstr(lines[i], "wlan")) c = MAGENTA;
        text(x, y, s, c, 1); y += lh;
    }
}

/* ---------- Wi-Fi manager (NETWORK panel) ----------
 * Talks straight to wpa_supplicant's ctrl socket (/tmp/wpa/wlan0, UNIX dgram).
 * Stack bring-up + join tooling lives on the SD (see experiments/net/wifi-up2.sh).
 */
#define WPA_CTRL "/tmp/wpa/wlan0"
#define LAB "/mnt/sd/linux-lab"

static void set_toast(const char *m, time_t now);   /* fwd */

static int wpa_fd = -1;
static char wpa_local[64];
static int wpa_open_ctrl(void) {
    if (wpa_fd >= 0) return 0;
    int fd = socket(AF_UNIX, SOCK_DGRAM, 0);
    if (fd < 0) return -1;
    struct sockaddr_un loc, rem;
    memset(&loc, 0, sizeof loc); loc.sun_family = AF_UNIX;
    snprintf(wpa_local, sizeof wpa_local, "/tmp/.neui-wpa-%d", getpid());
    strncpy(loc.sun_path, wpa_local, sizeof loc.sun_path - 1);
    unlink(wpa_local);
    memset(&rem, 0, sizeof rem); rem.sun_family = AF_UNIX;
    strncpy(rem.sun_path, WPA_CTRL, sizeof rem.sun_path - 1);
    if (bind(fd, (struct sockaddr *)&loc, sizeof loc) < 0 ||
        connect(fd, (struct sockaddr *)&rem, sizeof rem) < 0) {
        close(fd); unlink(wpa_local); return -1;
    }
    struct timeval tv = { 2, 0 };
    setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof tv);
    wpa_fd = fd;
    return 0;
}
static int wpa_cmd(const char *cmd, char *out, int outsz) {
    out[0] = 0;
    if (wpa_open_ctrl() < 0) return -1;
    if (send(wpa_fd, cmd, strlen(cmd), 0) < 0) { close(wpa_fd); wpa_fd = -1; return -1; }
    int n = recv(wpa_fd, out, outsz - 1, 0);
    if (n < 0) { close(wpa_fd); wpa_fd = -1; return -1; }   /* timeout/err: force reopen */
    out[n] = 0;
    return n;
}
/* value of "key=" line in a STATUS reply, or "" */
static void wpa_field(const char *reply, const char *key, char *out, int n) {
    out[0] = 0;
    size_t kl = strlen(key);
    for (const char *p = reply; p && *p; ) {
        if (!strncmp(p, key, kl) && p[kl] == '=') {
            const char *e = strchr(p, '\n'); int L = e ? (int)(e - p - kl - 1) : (int)strlen(p + kl + 1);
            if (L > n - 1) L = n - 1;
            memcpy(out, p + kl + 1, L); out[L] = 0; return;
        }
        p = strchr(p, '\n'); if (p) p++;
    }
}

static int wlan_present(void) { return access("/sys/class/net/wlan0", F_OK) == 0; }
static int wlan_ip(char *buf, int n) {
    buf[0] = 0;
    int s = socket(AF_INET, SOCK_DGRAM, 0);
    if (s < 0) return 0;
    struct ifreq ifr; memset(&ifr, 0, sizeof ifr);
    strncpy(ifr.ifr_name, "wlan0", sizeof ifr.ifr_name - 1);
    int ok = ioctl(s, SIOCGIFADDR, &ifr) == 0;
    close(s);
    if (!ok) return 0;
    unsigned char *a = (unsigned char *)&((struct sockaddr_in *)&ifr.ifr_addr)->sin_addr;
    snprintf(buf, n, "%d.%d.%d.%d", a[0], a[1], a[2], a[3]);
    return 1;
}

static void spawn_sh(const char *cmdline) {           /* fire-and-forget shell */
    if (fork() == 0) {
        setsid();
        execl("/bin/sh", "sh", "-c", cmdline, (char *)NULL);
        _exit(127);
    }
}

struct ap { char ssid[33]; int rssi; int wpa; };
static struct ap aps[10];
static int nap;
static int scan_inflight;
static time_t scan_sent_at;
static int stack_starting;
static time_t stack_started_at;
static int joining, dhcp_started;
static time_t join_started;
static char join_ssid[33];
static int join_wpa;

/* hit rects, filled during draw */
static int net_rowy[10], net_rown, net_rowh;
static int net_btn[4];        /* the one button (RESCAN or BRING UP): x,y,w,h */
static int net_btn_mode;      /* 0 none, 1 bring-up, 2 rescan */

static void net_send_scan(time_t now) {
    char out[64];
    if (wpa_cmd("SCAN", out, sizeof out) >= 0) { scan_inflight = 1; scan_sent_at = now; }
}

static void net_fetch_scan(void) {
    static char buf[8192];
    if (wpa_cmd("SCAN_RESULTS", buf, sizeof buf) < 0) return;
    nap = 0;
    char *p = strchr(buf, '\n');                       /* skip header row */
    while (p && *++p && nap < 10) {
        char *nl = strchr(p, '\n'); if (nl) *nl = 0;
        char *f[5] = { p, 0, 0, 0, 0 };                /* bssid freq signal flags ssid */
        int nf = 1;
        for (char *t = p; *t && nf < 5; t++) if (*t == '\t') { *t = 0; f[nf++] = t + 1; }
        if (nf == 5 && f[4][0]) {
            int rssi = atoi(f[2]);
            int wpa = strstr(f[3], "WPA") != NULL;
            int k = -1;                                /* dedupe by ssid, keep strongest */
            for (int i = 0; i < nap; i++) if (!strcmp(aps[i].ssid, f[4])) k = i;
            if (k >= 0) { if (rssi > aps[k].rssi) aps[k].rssi = rssi; }
            else {
                strncpy(aps[nap].ssid, f[4], 32); aps[nap].ssid[32] = 0;
                aps[nap].rssi = rssi; aps[nap].wpa = wpa; nap++;
            }
        }
        p = nl ? nl + 1 : NULL;
    }
    for (int i = 1; i < nap; i++) {                    /* sort by rssi desc */
        struct ap t = aps[i]; int j = i - 1;
        while (j >= 0 && aps[j].rssi < t.rssi) { aps[j+1] = aps[j]; j--; }
        aps[j+1] = t;
    }
    if (nap) scan_inflight = 0;
}

/* join: reuse a saved network id if the ssid is known, else add a fresh one */
static void net_join(const char *ssid, const char *psk, int wpa, time_t now) {
    static char buf[2048];
    char cmd[160], out[64];
    int id = -1;
    if (wpa_cmd("LIST_NETWORKS", buf, sizeof buf) > 0) {
        for (char *p = strchr(buf, '\n'); p && *++p; ) {
            char *nl = strchr(p, '\n'); if (nl) *nl = 0;
            char *t1 = strchr(p, '\t');
            if (t1) { char *t2 = strchr(t1 + 1, '\t'); if (t2) *t2 = 0;
                      if (!strcmp(t1 + 1, ssid)) id = atoi(p); }
            p = nl ? nl + 1 : NULL;
        }
    }
    if (id < 0) {
        if (wpa_cmd("ADD_NETWORK", out, sizeof out) < 0) { set_toast("supplicant not responding", now); return; }
        id = atoi(out);
        snprintf(cmd, sizeof cmd, "SET_NETWORK %d ssid \"%s\"", id, ssid);
        wpa_cmd(cmd, out, sizeof out);
        if (wpa) { snprintf(cmd, sizeof cmd, "SET_NETWORK %d psk \"%s\"", id, psk);
                   wpa_cmd(cmd, out, sizeof out);
                   if (strncmp(out, "OK", 2)) { set_toast("psk rejected (8..63 chars)", now); return; } }
        else     { snprintf(cmd, sizeof cmd, "SET_NETWORK %d key_mgmt NONE", id);
                   wpa_cmd(cmd, out, sizeof out); }
    }
    snprintf(cmd, sizeof cmd, "SELECT_NETWORK %d", id);
    wpa_cmd(cmd, out, sizeof out);
    wpa_cmd("SAVE_CONFIG", out, sizeof out);
    spawn_sh("cp /tmp/wpa.conf " LAB "/wpa.conf 2>/dev/null");
    if (ssid != join_ssid) { strncpy(join_ssid, ssid, 32); join_ssid[32] = 0; }
    joining = 1; dhcp_started = 0; join_started = now;
    char m[64]; snprintf(m, sizeof m, "joining %s ...", ssid); set_toast(m, now);
}

/* once per second: drive scan results + join/DHCP state machine */
static void net_tick(time_t now) {
    if (stack_starting) {
        if (wlan_present() && wpa_open_ctrl() == 0) { stack_starting = 0; net_send_scan(now); }
        else if (now - stack_started_at > 45) stack_starting = 0;   /* give up flag; log stays */
        return;
    }
    if (!wlan_present()) return;
    if (scan_inflight && now - scan_sent_at >= 3) net_fetch_scan();
    if (joining) {
        static char st[4096]; char state[32];
        if (wpa_cmd("STATUS", st, sizeof st) > 0) {
            wpa_field(st, "wpa_state", state, sizeof state);
            if (!strcmp(state, "COMPLETED")) {
                char ip[20];
                if (!dhcp_started) {
                    dhcp_started = 1;
                    /* devpts must be mounted or telnetd dies after one connection (L1 has
                       /dev/ptmx but nothing mounts devpts). Mount it before starting telnetd. */
                    spawn_sh("udhcpc -i wlan0 -n -q -s " LAB "/udhcpc.script >/tmp/udhcpc.log 2>&1; "
                             "[ -d /dev/pts ] || mkdir -p /dev/pts; "
                             "mount | grep -q ' /dev/pts ' || mount -t devpts devpts /dev/pts 2>/dev/null; "
                             "pgrep telnetd >/dev/null || telnetd -l /bin/sh -p 23 2>/dev/null");
                } else if (wlan_ip(ip, sizeof ip)) {
                    char m[64]; snprintf(m, sizeof m, "ONLINE  %s  (telnet ready)", ip);
                    set_toast(m, now); joining = 0;
                }
            }
        }
        if (joining && now - join_started > 40) { joining = 0; set_toast("join timed out — check psk", now); }
    }
}

static void net_button(int x, int y, int w, int h, const char *label, uint32_t acc) {
    fill(x, y, w, h, mix(BG, acc, 20));
    neonbox(x, y, w, h, acc);
    corners(x, y, w, h, acc, 8);
    text(x + w / 2 - (int)strlen(label) * FONT_W / 2, y + h / 2 - FONT_H / 2, label, acc, 1);
}

static void draw_net(void) {
    int x = cx + 20, y = cy + 16, lh = FONT_H + 8;
    char line[160], st[4096], state[32], cssid[40], ip[20];
    net_btn_mode = 0; net_rown = 0;

    /* --- status line --- */
    int up = wlan_present();
    int ctrl = up && wpa_open_ctrl() == 0;
    if (ctrl && wpa_cmd("STATUS", st, sizeof st) > 0) {
        wpa_field(st, "wpa_state", state, sizeof state);
        wpa_field(st, "ssid", cssid, sizeof cssid);
    } else { state[0] = 0; cssid[0] = 0; }
    int have_ip = up && wlan_ip(ip, sizeof ip);

    if (!up) {
        text(x, y, "wlan0", TEXT2, 1);
        text(x + 90, y, stack_starting ? "BRINGING UP CONSYS STACK ..." : "STACK DOWN", stack_starting ? AMBER : REDA, 1);
    } else if (!strcmp(state, "COMPLETED")) {
        text(x, y, "wlan0", TEXT2, 1);
        textg(x + 90, y, "CONNECTED", GREEN, 1);
        snprintf(line, sizeof line, "%s   %s", cssid, have_ip ? ip : "(dhcp...)");
        text(x + 90 + 10 * FONT_W + 16, y, line, WHITE, 1);
    } else {
        text(x, y, "wlan0", TEXT2, 1);
        text(x + 90, y, state[0] ? state : (ctrl ? "IDLE" : "SUPPLICANT DOWN"),
             joining ? AMBER : MAGENTA, 1);
        if (joining) text(x + 90 + (int)strlen(state[0] ? state : "IDLE") * FONT_W + 16, y, join_ssid, TEXT2, 1);
    }

    /* --- the one action button, top right --- */
    int bw = 150, bh = 40, bx = cx + cw - bw - 18, by = cy + 10;
    if (!up || !ctrl) {
        if (!stack_starting) {
            net_button(bx, by, bw, bh, "BRING UP", AMBER);
            net_btn[0] = bx; net_btn[1] = by; net_btn[2] = bw; net_btn[3] = bh; net_btn_mode = 1;
        }
    } else {
        net_button(bx, by, bw, bh, scan_inflight ? "SCANNING." : "RESCAN", CYAN);
        if (!scan_inflight) { net_btn[0] = bx; net_btn[1] = by; net_btn[2] = bw; net_btn[3] = bh; net_btn_mode = 2; }
    }
    y += lh + 4;
    hline(x, y, cw - 40, mix(BG, MAGENTA, 60)); y += 8;

    if (!up) {
        y += 6;
        if (stack_starting) {
            /* live tail of the bring-up log */
            text(x, y, "running " LAB "/wifi-up2.sh", TEXT2, 1); y += lh;
            FILE *f = fopen("/tmp/wifi-up2.log", "r");
            if (f) {
                char tail[6][100]; int nt = 0;
                while (fgets(line, sizeof line, f)) {
                    line[strcspn(line, "\n")] = 0;
                    if (!line[0]) continue;
                    snprintf(tail[nt % 6], 100, "%.99s", line); nt++;
                }
                fclose(f);
                int from = nt > 6 ? nt - 6 : 0;
                for (int i = from; i < nt; i++) { text(x, y, tail[i % 6], TEXTDIM, 1); y += FONT_H + 4; }
            }
        } else {
            text(x, y, "consys / wlan driver not up on this boot.", TEXT2, 1); y += lh;
            text(x, y, "tap BRING UP to run the full stack bring-up", TEXT2, 1); y += lh;
            text(x, y, "(firmware stage > wmt_loader > wmtctl2 > wpa_supplicant)", TEXTDIM, 1);
        }
        return;
    }

    /* --- scan list --- */
    snprintf(line, sizeof line, "%-26s %8s   %s", "SSID", "SIGNAL", "SECURITY");
    text(x, y, line, TEXT2, 1); y += lh - 2;
    hline(x, y - 4, cw - 40, mix(BG, MAGENTA, 40));
    net_rowh = lh + 6;
    int maxrows = (cy + ch - 30 - y) / net_rowh; if (maxrows > 10) maxrows = 10;
    if (!nap) {
        text(x, y + 8, scan_inflight ? "scanning ..." : "no scan results — tap RESCAN", TEXTDIM, 1);
    }
    for (int i = 0; i < nap && i < maxrows; i++) {
        int ry = y + i * net_rowh;
        int cur = cssid[0] && !strcmp(aps[i].ssid, cssid);
        if (cur) fill(x - 6, ry - 3, cw - 34, net_rowh - 2, mix(BG, GREEN, 16));
        int sig = aps[i].rssi;                          /* dBm to 0..4 bars */
        int bars = sig > -45 ? 4 : sig > -55 ? 3 : sig > -67 ? 2 : sig > -75 ? 1 : 0;
        for (int b = 0; b < 4; b++)
            fill(x + b * 7, ry + FONT_H - 3 - b * 3, 5, 3 + b * 3,
                 b <= bars ? (cur ? GREEN : CYAN) : mix(BG, BORDER, 90));
        snprintf(line, sizeof line, "%-26.26s %5d dBm  %s", aps[i].ssid, sig, aps[i].wpa ? "WPA2" : "open");
        (cur ? textg : text)(x + 38, ry, line, cur ? GREEN : TEXT, 1);
        net_rowy[net_rown++] = ry;
    }
    if (nap) text(x, cy + ch - 24, "tap a network to join", TEXTDIM, 1);
}

/* ---------- password keyboard (modal) ---------- */
static int kb_open, kb_page, kb_len;
static char kb_buf[64];
static const char *KB_ROWS[3][4] = {
    { "1234567890", "qwertyuiop", "asdfghjkl-", "\1zxcvbnm_\b" },
    { "1234567890", "QWERTYUIOP", "ASDFGHJKL-", "\1ZXCVBNM_\b" },
    { "!@#$%^&*()", "-_=+[]{}\\|", ";:'\",.<>/?", "\1~`      \b" },
};
static const char *KB_PAGENAME[3] = { "abc", "ABC", "#+=" };

#define KB_M 30
#define KB_GAP 8
static void kb_geom(int *ox, int *oy, int *ow, int *oh, int *kw, int *kh, int *gx, int *gy) {
    *ox = KB_M; *oy = 14; *ow = (int)xres - 2 * KB_M; *oh = (int)yres - 28;
    *kw = (*ow - 36 - 9 * KB_GAP) / 10;
    *kh = 58;
    *gx = *ox + 18 + (*ow - 36 - 10 * *kw - 9 * KB_GAP) / 2;
    *gy = *oy + 118;                       /* below title + input box */
}
/* bottom action row: CANCEL | SPACE | page | JOIN */
static void kb_actrect(int i, int *x, int *y, int *w, int *h) {
    int ox, oy, ow, oh, kw, kh, gx, gy; kb_geom(&ox, &oy, &ow, &oh, &kw, &kh, &gx, &gy);
    int ay = gy + 4 * (kh + KB_GAP) + 6;
    int seg[4] = { 2, 4, 2, 2 };          /* widths in key units */
    int xx = gx;
    for (int k = 0; k < i; k++) xx += seg[k] * kw + (seg[k]) * KB_GAP;
    *x = xx; *y = ay; *w = seg[i] * kw + (seg[i] - 1) * KB_GAP; *h = kh + 6;
}

static void draw_kb(void) {
    int ox, oy, ow, oh, kw, kh, gx, gy; kb_geom(&ox, &oy, &ow, &oh, &kw, &kh, &gx, &gy);
    fill(ox, oy, ow, oh, PANEL);
    neonbox(ox, oy, ow, oh, MAGENTA);
    corners(ox, oy, ow, oh, MAGENTA, 20);
    char title[80]; snprintf(title, sizeof title, "[ JOIN %s %s]", join_ssid, join_wpa ? "" : "(open) ");
    textg(ox + 20, oy + 12, title, MAGENTA, 1);
    text(ox + ow - 4 * FONT_W - 16, oy + 12, "[X]", CYAN, 1);

    /* input box */
    int ix = ox + 18, iy = oy + 48, iw = ow - 36, ih = 44;
    fill(ix, iy, iw, ih, BG);
    neonbox(ix, iy, iw, ih, kb_len ? CYAN : BORDER);
    char shown[70];
    snprintf(shown, sizeof shown, "%s_", kb_buf);
    text(ix + 14, iy + ih / 2 - FONT_H / 2, shown, CYANHI, 1);
    snprintf(shown, sizeof shown, "%d chars%s", kb_len, (join_wpa && kb_len && kb_len < 8) ? "  (min 8)" : "");
    text(ix + iw - (int)strlen(shown) * FONT_W - 12, iy + ih / 2 - FONT_H / 2, shown,
         (join_wpa && kb_len && kb_len < 8) ? AMBER : TEXTDIM, 1);

    /* key grid */
    for (int r = 0; r < 4; r++) {
        const char *row = KB_ROWS[kb_page][r];
        for (int c = 0; c < 10 && row[c]; c++) {
            char ch = row[c];
            if (ch == ' ') continue;
            int x = gx + c * (kw + KB_GAP), y = gy + r * (kh + KB_GAP);
            const char *lbl; char one[2] = { ch, 0 };
            uint32_t acc = CYAN;
            if (ch == '\1') { lbl = KB_PAGENAME[(kb_page + 1) % 3]; acc = AMBER; }
            else if (ch == '\b') { lbl = "DEL"; acc = REDA; }
            else lbl = one;
            fill(x, y, kw, kh, mix(BG, acc, 12));
            neonbox(x, y, kw, kh, mix(BG, acc, 120));
            text(x + kw / 2 - (int)strlen(lbl) * FONT_W / 2, y + kh / 2 - FONT_H / 2, lbl, acc, 1);
        }
    }
    /* action row */
    static const char *alabel[4] = { "CANCEL", "SPACE", "", "JOIN" };
    for (int i = 0; i < 4; i++) {
        int x, y, w, h; kb_actrect(i, &x, &y, &w, &h);
        const char *lbl = i == 2 ? KB_PAGENAME[(kb_page + 1) % 3] : alabel[i];
        uint32_t acc = i == 0 ? REDA : i == 3 ? ((join_wpa && kb_len < 8) ? TEXTDIM : GREEN) : i == 2 ? AMBER : CYAN;
        fill(x, y, w, h, mix(BG, acc, 16));
        neonbox(x, y, w, h, acc);
        corners(x, y, w, h, acc, 8);
        (i == 3 ? textg : text)(x + w / 2 - (int)strlen(lbl) * FONT_W / 2, y + h / 2 - FONT_H / 2, lbl, acc, 1);
    }
}

static int kb_tap(int sx, int sy, time_t now) {
    int ox, oy, ow, oh, kw, kh, gx, gy; kb_geom(&ox, &oy, &ow, &oh, &kw, &kh, &gx, &gy);
    if (sy < oy + 44 && sx > ox + ow - 6 * FONT_W - 16) { kb_open = 0; return 1; }   /* [X] */
    for (int i = 0; i < 4; i++) {                                                   /* action row */
        int x, y, w, h; kb_actrect(i, &x, &y, &w, &h);
        if (sx >= x && sx < x + w && sy >= y && sy < y + h) {
            if (i == 0) kb_open = 0;
            else if (i == 1) { if (kb_len < 63) { kb_buf[kb_len++] = ' '; kb_buf[kb_len] = 0; } }
            else if (i == 2) kb_page = (kb_page + 1) % 3;
            else if (!join_wpa || kb_len >= 8) { kb_open = 0; net_join(join_ssid, kb_buf, join_wpa, now); }
            return 1;
        }
    }
    for (int r = 0; r < 4; r++) {                                                   /* key grid */
        const char *row = KB_ROWS[kb_page][r];
        for (int c = 0; c < 10 && row[c]; c++) {
            if (row[c] == ' ') continue;
            int x = gx + c * (kw + KB_GAP), y = gy + r * (kh + KB_GAP);
            if (sx >= x && sx < x + kw && sy >= y && sy < y + kh) {
                char ch = row[c];
                if (ch == '\1') kb_page = (kb_page + 1) % 3;
                else if (ch == '\b') { if (kb_len) kb_buf[--kb_len] = 0; }
                else if (kb_len < 63) { kb_buf[kb_len++] = ch; kb_buf[kb_len] = 0; }
                return 1;
            }
        }
    }
    return 1;                                                                       /* modal: swallow */
}

static int net_tap(int sx, int sy, time_t now) {
    if (net_btn_mode && sx >= net_btn[0] && sx < net_btn[0] + net_btn[2] &&
        sy >= net_btn[1] && sy < net_btn[1] + net_btn[3]) {
        if (net_btn_mode == 1) {
            spawn_sh("sh " LAB "/wifi-up2.sh >/tmp/wifi-up2.log 2>&1");
            stack_starting = 1; stack_started_at = now;
            set_toast("bringing up wifi stack...", now);
        } else {
            net_send_scan(now);
            set_toast("scanning...", now);
        }
        return 1;
    }
    for (int i = 0; i < net_rown; i++) {
        if (sy >= net_rowy[i] - 3 && sy < net_rowy[i] + net_rowh - 3 && sx >= cx + 10 && sx < cx + cw - 20) {
            strncpy(join_ssid, aps[i].ssid, 32); join_ssid[32] = 0;
            join_wpa = aps[i].wpa;
            /* known network? reconnect without asking for the psk again */
            static char buf[2048]; int known = 0;
            if (wpa_cmd("LIST_NETWORKS", buf, sizeof buf) > 0) {
                for (char *p = strchr(buf, '\n'); p && *++p; ) {
                    char *nl = strchr(p, '\n'); if (nl) *nl = 0;
                    char *t1 = strchr(p, '\t');
                    if (t1) { char *t2 = strchr(t1 + 1, '\t'); if (t2) *t2 = 0;
                              if (!strcmp(t1 + 1, join_ssid)) known = 1; }
                    p = nl ? nl + 1 : NULL;
                }
            }
            if (known || !join_wpa) net_join(join_ssid, "", join_wpa, now);
            else { kb_open = 1; kb_page = 0; kb_len = 0; kb_buf[0] = 0; }
            return 1;
        }
    }
    return 0;
}

static void draw_content(int active) {
    panel(cx, cy, cw, ch, PACC[active], NULL);
    char title[32]; snprintf(title, sizeof title, "[ %s ]", PNAME[active]);
    textg(cx + 24, cy - FONT_H / 2 - 1, title, PACC[active], 1);
    switch (active) {
        case P_SYSTEM:  draw_system();  break;
        case P_PROC:    draw_proc();    break;
        case P_STORAGE: draw_storage(); break;
        case P_LOG:     draw_log();     break;
        case P_NET:     draw_net();     break;
    }
}

/* ---------- action bar ---------- */
#define ACTBARH 46
#define ACT_Y  ((int)yres - 48 - ACTBARH)

typedef const char *(*actfn)(void);
static const char *act_sync(void)    { sync(); return "disks synced"; }
static const char *act_reboot(void)  { sync(); reboot(RB_AUTOBOOT); return "rebooting..."; }

/* An action is either a TOOL (runs `cmd`, opens a scrollable output overlay) or a
   DO-action (calls `fn`). Tools have fn==NULL; do-actions have cmd==NULL. */
static struct action { const char *label; uint32_t accent; int confirm; actfn fn; const char *cmd; const char *title; } ACT[] = {
    { "DF",     CYAN,   0, 0, "df -h 2>/dev/null || df",               "[ df -h ]"     },
    { "MEM",    GREEN,  0, 0, "free 2>/dev/null; echo; sed -n 1,6p /proc/meminfo", "[ memory ]" },
    { "MOUNTS", AMBER,  0, 0, "mount",                                 "[ mounts ]"    },
    { "DMESG",  PURPLE, 0, 0, "dmesg | tail -150",                     "[ dmesg tail ]"},
    { "SYNC",   CYANHI, 0, act_sync,   0, 0 },
    { "REBOOT", REDA,   1, act_reboot, 0, 0 },
};
#define NACT ((int)(sizeof ACT / sizeof ACT[0]))

/* ---------- output overlay (tool stdout, scrollable) ---------- */
#define OV_M 34
static int ov_open, ov_scroll, ov_nlines;
static char ov_title[64];
static char ov_buf[48 * 1024];
static char *ov_lines[600];

static void run_tool(const char *title, const char *cmd) {
    strncpy(ov_title, title, sizeof ov_title - 1); ov_title[sizeof ov_title - 1] = 0;
    int n = 0;
    FILE *f = popen(cmd, "r");
    if (f) { n = fread(ov_buf, 1, sizeof ov_buf - 1, f); pclose(f); }
    ov_buf[n < 0 ? 0 : n] = 0;
    ov_nlines = 0;
    for (char *p = ov_buf; *p && ov_nlines < 600; ) {
        ov_lines[ov_nlines++] = p;
        char *nl = strchr(p, '\n');
        if (!nl) break;
        *nl = 0; p = nl + 1;
    }
    if (!ov_nlines) { ov_lines[0] = (char *)"(no output)"; ov_nlines = 1; }
    ov_scroll = 0; ov_open = 1;
}

static void ov_rect(int *x, int *y, int *w, int *h) {
    *x = OV_M; *y = 16; *w = (int)xres - 2 * OV_M; *h = (int)yres - 32;  /* full inner area (modal) */
}

static int armed = -1;          /* action index awaiting confirm, or -1 */
static time_t armed_at;
static char toast[96];
static time_t toast_until;
static void set_toast(const char *m, time_t now) {
    strncpy(toast, m, sizeof toast - 1); toast[sizeof toast - 1] = 0;
    toast_until = now + 2;
}

static void act_rect(int i, int *x, int *y, int *w, int *h) {
    int bx = cx, bw = (int)xres - 24 - cx, gap = 10;
    int bw1 = (bw - (NACT - 1) * gap) / NACT;
    *x = bx + i * (bw1 + gap); *y = ACT_Y; *w = bw1; *h = ACTBARH;
}

static int act_hittest(int sx, int sy) {
    for (int i = 0; i < NACT; i++) {
        int x, y, w, h; act_rect(i, &x, &y, &w, &h);
        if (sx >= x && sx < x + w && sy >= y && sy < y + h) return i;
    }
    return -1;
}

static void draw_actionbar(void) {
    for (int i = 0; i < NACT; i++) {
        int x, y, w, h; act_rect(i, &x, &y, &w, &h);
        int a = armed == i;
        uint32_t acc = a ? REDA : ACT[i].accent;
        fill(x, y, w, h, mix(BG, acc, a ? 34 : 18));
        neonbox(x, y, w, h, acc);
        corners(x, y, w, h, acc, 10);
        const char *lbl = a ? "CONFIRM?" : ACT[i].label;
        int tx = x + w / 2 - (int)strlen(lbl) * FONT_W / 2;
        (a ? textg : text)(tx, y + h / 2 - FONT_H / 2, lbl, acc, 1);
    }
}

static void draw_toast(time_t now) {
    if (now >= toast_until || !toast[0]) return;
    int tw = (int)strlen(toast) * FONT_W + 44, th = FONT_H + 18;
    int tx = cx + cw / 2 - tw / 2, ty = ACT_Y - th - 12;
    fill(tx, ty, tw, th, mix(BG, GREEN, 30));
    neonbox(tx, ty, tw, th, GREEN);
    corners(tx, ty, tw, th, GREEN, 10);
    textg(tx + 22, ty + 9, toast, GREEN, 1);
}

#define OV_HDR 42          /* title row height */
#define OV_FTR 26          /* footer row height */
static int ov_rows(int oh) { return (oh - OV_HDR - OV_FTR) / (FONT_H + 3); }

static void draw_overlay(void) {
    int ox, oy, ow, oh; ov_rect(&ox, &oy, &ow, &oh);
    fill(ox, oy, ow, oh, PANEL);
    neonbox(ox, oy, ow, oh, CYAN);
    corners(ox, oy, ow, oh, CYAN, 20);
    textg(ox + 20, oy + 9, ov_title, CYAN, 1);
    text(ox + ow - 4 * FONT_W - 14, oy + 9, "[X]", MAGENTA, 1);
    hline(ox + 14, oy + OV_HDR - 6, ow - 28, mix(BG, CYAN, 70));

    int bx = ox + 18, by = oy + OV_HDR, lh = FONT_H + 3, rows = ov_rows(oh);
    int maxc = (ow - 40) / FONT_W; if (maxc > 250) maxc = 250;
    char tmp[256];
    for (int i = 0; i < rows && ov_scroll + i < ov_nlines; i++) {
        char *s = ov_lines[ov_scroll + i];
        int L = (int)strlen(s); if (L > maxc) L = maxc;
        memcpy(tmp, s, L); tmp[L] = 0;
        uint32_t c = TEXT2;
        if (strstr(s, "fail") || strstr(s, "error") || strstr(s, "Error")) c = REDA;
        text(bx, by + i * lh, tmp, c, 1);
    }
    /* scrollbar */
    if (ov_nlines > rows) {
        int track = oh - OV_HDR - OV_FTR, kh = track * rows / ov_nlines;
        int ky = by + track * ov_scroll / ov_nlines;
        fill(ox + ow - 7, by, 3, track, mix(BG, CYAN, 40));
        fill(ox + ow - 7, ky, 3, kh < 10 ? 10 : kh, CYAN);
    }
    char si[96];
    if (ov_nlines > rows)
        snprintf(si, sizeof si, "%d-%d / %d   tap lower half: scroll down   upper: up   [X]/outside: close",
                 ov_scroll + 1, (ov_scroll + rows > ov_nlines ? ov_nlines : ov_scroll + rows), ov_nlines);
    else
        snprintf(si, sizeof si, "%d lines   tap [X] or outside to close", ov_nlines);
    text(bx, oy + oh - 20, si, TEXTDIM, 1);
}

static int overlay_tap(int sx, int sy) {
    int ox, oy, ow, oh; ov_rect(&ox, &oy, &ow, &oh);
    if (sx < ox || sx >= ox + ow || sy < oy || sy >= oy + oh) { ov_open = 0; return 1; }  /* outside */
    if (sy < oy + OV_HDR && sx > ox + ow - 5 * FONT_W - 14) { ov_open = 0; return 1; }     /* [X] */
    int step = ov_rows(oh) - 1; if (step < 1) step = 1;
    if (sy > oy + oh * 3 / 5) ov_scroll += step;                                            /* lower: down */
    else ov_scroll -= step;                                                                /* upper: up */
    if (ov_scroll > ov_nlines - 1) ov_scroll = ov_nlines - 1;
    if (ov_scroll < 0) ov_scroll = 0;
    return 1;
}

/* ---------- chrome ---------- */
static void tile_rect(int i, int *x, int *y, int *w, int *h) {
    *x = NAVX; *y = NAVY + i * (TILEH + TILEG); *w = NAVW; *h = TILEH;
}

static void draw_chrome(int active) {
    background();
    neonbox(10, 10, xres - 20, yres - 20, mix(BG, CYAN, 60));
    corners(10, 10, xres - 20, yres - 20, CYAN, 22);

    /* header */
    textg(24, 20, "NEONWIRE", CYAN, 2);
    textg(24 + FONT_W * 2 * 8 + 16, 20, "OS", MAGENTA, 2);
    double up = uptime_s();
    char clk[32]; snprintf(clk, sizeof clk, "up %02d:%02d:%02d",
                           (int)up/3600, ((int)up%3600)/60, (int)up%60);
    text(xres - 28 - (int)strlen(clk) * FONT_W, 26, clk, CYAN, 1);
    hline(24, 74, xres - 48, mix(BG, CYAN, 90));

    /* nav rail */
    for (int i = 0; i < P_COUNT; i++) {
        int x, y, w, h; tile_rect(i, &x, &y, &w, &h);
        int on = i == active;
        fill(x, y, w, h, on ? mix(BG, PACC[i], 26) : PANEL);
        neonbox(x, y, w, h, on ? PACC[i] : BORDER);
        if (on) { corners(x, y, w, h, PACC[i], 12); vline(x + 2, y + 3, h - 6, PACC[i]); }
        char label[24]; snprintf(label, sizeof label, "%02d %s", i, PNAME[i]);
        (on ? textg : text)(x + 18, y + h/2 - FONT_H/2, label, on ? PACC[i] : TEXT2, 1);
    }
    /* footer hint under rail */
    text(NAVX, NAVY + P_COUNT * (TILEH + TILEG) + 6, "tap a tile", TEXTDIM, 1);

    draw_content(active);
    draw_actionbar();
    draw_toast(time(NULL));

    /* bottom bar */
    int by = yres - 40;
    fill(12, by, xres - 24, 28, mix(BG, CYAN, 14));
    hline(12, by, xres - 24, CYAN);
    text(24, by + 5, "root@dl7006:~#", GREEN, 1);
    textg(xres / 2 - 9 * FONT_W, by + 5, "[ NEONWIRE OS v0.1 ]", MAGENTA, 1);

    scanlines(0, 0, xres, yres);
    if (ov_open) draw_overlay();          /* modal on top, crisp (no scanline wash) */
    if (kb_open) draw_kb();               /* password keyboard is topmost */
}

/* ---------- touch ---------- */
static int amin_x, amax_x, amin_y, amax_y, ax_code, ay_code;
static int opt_swap, opt_flipx, opt_flipy;

static int touch_open(const char *dev) {
    int fd = open(dev, O_RDONLY);
    if (fd < 0) { perror(dev); return -1; }
    struct input_absinfo ai;
    ax_code = ABS_X; ay_code = ABS_Y;
    if (ioctl(fd, EVIOCGABS(ABS_X), &ai) == 0 && ai.maximum > ai.minimum) {
        amin_x = ai.minimum; amax_x = ai.maximum;
    } else { ax_code = ABS_MT_POSITION_X;
        ioctl(fd, EVIOCGABS(ABS_MT_POSITION_X), &ai); amin_x = ai.minimum; amax_x = ai.maximum; }
    if (ioctl(fd, EVIOCGABS(ABS_Y), &ai) == 0 && ai.maximum > ai.minimum) {
        amin_y = ai.minimum; amax_y = ai.maximum;
    } else { ay_code = ABS_MT_POSITION_Y;
        ioctl(fd, EVIOCGABS(ABS_MT_POSITION_Y), &ai); amin_y = ai.minimum; amax_y = ai.maximum; }
    if (amax_x <= amin_x) amax_x = amin_x + 1;
    if (amax_y <= amin_y) amax_y = amin_y + 1;
    fprintf(stderr, "touch %s  X[%d..%d] code %d   Y[%d..%d] code %d\n",
            dev, amin_x, amax_x, ax_code, amin_y, amax_y, ay_code);
    return fd;
}

static void map_touch(int rx, int ry, int *sx, int *sy) {
    if (opt_swap) { int t = rx; rx = ry; ry = t; }
    int mx = (rx - amin_x) * (int)(xres - 1) / (amax_x - amin_x);
    int my = (ry - amin_y) * (int)(yres - 1) / (amax_y - amin_y);
    if (opt_flipx) mx = xres - 1 - mx;
    if (opt_flipy) my = yres - 1 - my;
    if (mx < 0) mx = 0;
    if (mx >= (int)xres) mx = xres - 1;
    if (my < 0) my = 0;
    if (my >= (int)yres) my = yres - 1;
    *sx = mx; *sy = my;
}

static int hittest(int sx, int sy) {
    for (int i = 0; i < P_COUNT; i++) {
        int x, y, w, h; tile_rect(i, &x, &y, &w, &h);
        if (sx >= x && sx < x + w && sy >= y && sy < y + h) return i;
    }
    return -1;
}

/* dispatch a screen-space tap; returns 1 if it changed state (needs redraw) */
static int handle_tap(int sx, int sy, time_t now, int *active) {
    if (kb_open) return kb_tap(sx, sy, now);      /* keyboard is topmost modal */
    if (ov_open) return overlay_tap(sx, sy);      /* overlay captures all taps */
    int hit = hittest(sx, sy);
    int a = hit < 0 ? act_hittest(sx, sy) : -1;
    if (hit >= 0) {                               /* nav tile */
        if (hit != *active) *active = hit;
        armed = -1; return 1;
    }
    if (a >= 0) {                                 /* action button */
        if (ACT[a].cmd) {                         /* TOOL: run command → overlay */
            run_tool(ACT[a].title, ACT[a].cmd); armed = -1;
        } else if (ACT[a].confirm && armed != a) {
            armed = a; armed_at = now; set_toast("tap again to confirm", now);
        } else {
            set_toast(ACT[a].fn(), now); armed = -1;
        }
        return 1;
    }
    if (*active == P_NET && net_tap(sx, sy, now)) return 1;   /* wifi list / buttons */
    if (armed >= 0) { armed = -1; return 1; }     /* tap elsewhere cancels */
    return 0;
}

int main(int argc, char **argv) {
    const char *shot = NULL, *dev = "/dev/input/event4";
    int active = P_SYSTEM, probe = 0;
    int taps[32][2], ntaps = 0;
    for (int i = 1; i < argc; i++) {
        if (!strcmp(argv[i], "--shot") && i+1 < argc) shot = argv[++i];
        else if (!strcmp(argv[i], "--panel") && i+1 < argc) active = atoi(argv[++i]);
        else if (!strcmp(argv[i], "--dev") && i+1 < argc) dev = argv[++i];
        else if (!strcmp(argv[i], "--probe")) probe = 1;
        else if (!strcmp(argv[i], "--evdump")) probe = 2;
        else if (!strcmp(argv[i], "--tap") && i+2 < argc && ntaps < 32) {
            taps[ntaps][0] = atoi(argv[++i]); taps[ntaps][1] = atoi(argv[++i]); ntaps++;
        }
        else if (!strcmp(argv[i], "--swap")) opt_swap = 1;
        else if (!strcmp(argv[i], "--flipx")) opt_flipx = 1;
        else if (!strcmp(argv[i], "--flipy")) opt_flipy = 1;
    }
    if (active < 0 || active >= P_COUNT) active = 0;
    signal(SIGCHLD, SIG_IGN);               /* spawned udhcpc/scripts: no zombies */

    if (fb_open() < 0) return 1;
    printf("SHOT w=%u h=%u stride=%u r=%u g=%u b=%u\n",
           xres, yres, stride, vi.red.offset, vi.green.offset, vi.blue.offset);
    cx = NAVX + NAVW + 16; cy = NAVY; cw = xres - 24 - cx; ch = ACT_Y - 12 - cy;

    /* headless tap injection (host testing over serial): repeatable --tap X Y.
       Draw before each dispatch so hit rects (wifi rows, keyboard) are current. */
    if (ntaps) {
        if (active == P_NET && wlan_present() && wpa_open_ctrl() == 0) {
            net_send_scan(time(NULL)); sleep(4); net_fetch_scan();   /* rows must exist BEFORE taps */
        }
        for (int i = 0; i < ntaps; i++) {
            draw_chrome(active);
            int t = hittest(taps[i][0], taps[i][1]), a = act_hittest(taps[i][0], taps[i][1]);
            printf("TAP (%d,%d) tile=%d action=%d\n", taps[i][0], taps[i][1], t, a);
            handle_tap(taps[i][0], taps[i][1], time(NULL), &active);
            printf("RESULT toast='%s' active=%d armed=%d kb=%d kbbuf='%s'\n",
                   toast, active, armed, kb_open, kb_buf);
        }
        net_tick(time(NULL));
        draw_chrome(active); fb_present();
        if (shot) fb_shot(shot);
        return 0;
    }

    if (shot) {
        if (active == P_NET && wlan_present() && wpa_open_ctrl() == 0) {
            net_send_scan(time(NULL)); sleep(4); net_fetch_scan();
        }
        draw_chrome(active); fb_present(); fb_shot(shot); return 0;
    }

    int tfd = touch_open(dev);
    if (probe == 2) {                       /* --evdump: print every raw event */
        struct input_event ev;
        static const char *tn[] = {"SYN","KEY","REL","ABS"};
        printf("evdump: tap the screen now...\n"); fflush(stdout);
        while (read(tfd, &ev, sizeof ev) == sizeof ev) {
            const char *t = ev.type < 4 ? tn[ev.type] : "?";
            printf("ev type=%s(%d) code=%d val=%d\n", t, ev.type, ev.code, ev.value);
            fflush(stdout);
        }
        return 0;
    }
    if (probe) {
        struct input_event ev; int rx = 0, ry = 0;
        while (read(tfd, &ev, sizeof ev) == sizeof ev) {
            if (ev.type == EV_ABS && (ev.code == ax_code || ev.code == ABS_MT_POSITION_X)) rx = ev.value;
            if (ev.type == EV_ABS && (ev.code == ay_code || ev.code == ABS_MT_POSITION_Y)) ry = ev.value;
            if (ev.type == EV_KEY && ev.code == BTN_TOUCH && ev.value == 1) {
                int sx, sy; map_touch(rx, ry, &sx, &sy);
                printf("tap raw(%d,%d) -> screen(%d,%d) tile=%d action=%d\n",
                       rx, ry, sx, sy, hittest(sx, sy), act_hittest(sx, sy));
                fflush(stdout);
            }
        }
        return 0;
    }

    draw_chrome(active); fb_present();
    struct pollfd pfd = { .fd = tfd, .events = POLLIN };
    int rx = 0, ry = 0, dirty = 0;
    /* fire on the SYN_REPORT after a BTN_TOUCH press, so X/Y are fresh.
       Fallback: type-A devices with no BTN_TOUCH arm on first MT point. */
    int pending = 0, frame_pt = 0, has_btn = 0;
    time_t last = 0;
    while (1) {
        int pr = poll(&pfd, tfd >= 0 ? 1 : 0, 1000);
        if (pr > 0 && (pfd.revents & POLLIN)) {
            struct input_event ev;
            if (read(tfd, &ev, sizeof ev) == sizeof ev) {
                if (ev.type == EV_ABS) {
                    if (ev.code == ax_code || ev.code == ABS_MT_POSITION_X) { rx = ev.value; frame_pt = 1; }
                    else if (ev.code == ay_code || ev.code == ABS_MT_POSITION_Y) { ry = ev.value; frame_pt = 1; }
                } else if (ev.type == EV_KEY && ev.code == BTN_TOUCH) {
                    has_btn = 1;
                    if (ev.value == 1) pending = 1;       /* press edge armed */
                } else if (ev.type == EV_SYN && ev.code == SYN_REPORT) {
                    if (!has_btn && frame_pt) pending = 1; /* fallback for no-BTN devices */
                    if (pending && frame_pt) {            /* fire once, with fresh coords */
                        pending = 0;
                        int sx, sy; map_touch(rx, ry, &sx, &sy);
                        fprintf(stderr, "tap raw(%d,%d) screen(%d,%d) tile=%d action=%d\n",
                                rx, ry, sx, sy, hittest(sx, sy), act_hittest(sx, sy));
                        if (handle_tap(sx, sy, time(NULL), &active)) dirty = 1;
                    }
                    frame_pt = 0;
                }
            }
        }
        time_t now = time(NULL);
        if (armed >= 0 && now - armed_at > 5) { armed = -1; dirty = 1; }  /* disarm */
        if (now != last) net_tick(now);       /* wifi scan/join/DHCP state machine */
        if (dirty || now != last) {           /* redraw on tap or once a second */
            draw_chrome(active); fb_present();
            dirty = 0; last = now;
        }
    }
    return 0;
}
