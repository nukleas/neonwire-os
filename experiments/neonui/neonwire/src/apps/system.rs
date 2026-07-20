//! SYSTEM app — M5: INFO / PROC / DISK / KLOG sub-tabs, tool buttons whose
//! stdout opens a scrollable modal overlay, and SYNC/REBOOT with the two-tap
//! arm/confirm pattern. Port of neui.c draw_proc/draw_storage/draw_log +
//! ACT[]/run_tool/draw_overlay.

use std::time::Instant;

use neon_gfx::canvas::{mix, Canvas};
use neon_gfx::font_data::{FONT_H, FONT_W};
use neon_gfx::geom::Rect;
use neon_gfx::theme::*;

use super::{App, Ctx, HitId, HitMap};

const HIT_TAB0: HitId = 0x100; // ..+3
const HIT_ACT0: HitId = 0x200; // ..+5
const HIT_OV_BG: HitId = 0x300;
const HIT_OV_X: HitId = 0x301;
const HIT_OV_UP: HitId = 0x302;
const HIT_OV_DN: HitId = 0x303;

const TABS: [&str; 4] = ["INFO", "PROC", "DISK", "KLOG"];

struct Action {
    label: &'static str,
    accent: u32,
    confirm: bool,
    cmd: Option<(&'static str, &'static str)>, // (overlay title, shell cmd)
}

const ACTIONS: [Action; 6] = [
    Action { label: "DF", accent: CYAN, confirm: false, cmd: Some(("[ df -h ]", "df -h 2>/dev/null || df")) },
    Action { label: "MEM", accent: GREEN, confirm: false, cmd: Some(("[ memory ]", "free 2>/dev/null; echo; sed -n 1,6p /proc/meminfo")) },
    Action { label: "MOUNTS", accent: AMBER, confirm: false, cmd: Some(("[ mounts ]", "mount")) },
    Action { label: "DMESG", accent: PURPLE, confirm: false, cmd: Some(("[ dmesg tail ]", "dmesg | tail -150")) },
    Action { label: "SYNC", accent: CYANHI, confirm: false, cmd: None },
    Action { label: "REBOOT", accent: RED, confirm: true, cmd: None },
];

struct Overlay {
    title: &'static str,
    lines: Vec<String>,
    scroll: usize,
}

pub struct SystemApp {
    tab: usize,
    overlay: Option<Overlay>,
    armed: Option<(usize, Instant)>,
}

impl SystemApp {
    pub fn new() -> SystemApp {
        SystemApp { tab: 0, overlay: None, armed: None }
    }

    fn run_tool(&mut self, title: &'static str, cmd: &str) {
        let out = std::process::Command::new("/bin/sh")
            .args(["-c", cmd])
            .output()
            .map(|o| {
                let mut s = String::from_utf8_lossy(&o.stdout).into_owned();
                s.push_str(&String::from_utf8_lossy(&o.stderr));
                s
            })
            .unwrap_or_else(|e| format!("spawn failed: {e}"));
        let mut lines: Vec<String> = out.lines().map(|l| l.to_string()).collect();
        if lines.is_empty() {
            lines.push("(no output)".into());
        }
        self.overlay = Some(Overlay { title, lines, scroll: 0 });
    }

    fn do_action(&mut self, i: usize, ctx: &mut Ctx) {
        match ACTIONS[i].label {
            "SYNC" => {
                unsafe { libc::sync() };
                ctx.set_toast("disks synced");
            }
            "REBOOT" => {
                unsafe {
                    libc::sync();
                    libc::reboot(libc::LINUX_REBOOT_CMD_RESTART);
                }
            }
            _ => {}
        }
    }
}

// ---- data readers (ports of the C helpers) ----

struct ProcRow {
    pid: i32,
    state: char,
    rss_kb: i64,
    name: String,
}

fn read_procs() -> Vec<ProcRow> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir("/proc") else {
        return out;
    };
    for e in rd.filter_map(|e| e.ok()) {
        let name = e.file_name();
        let Some(pid) = name.to_str().and_then(|s| s.parse::<i32>().ok()) else {
            continue;
        };
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            continue;
        };
        // "pid (comm with spaces) S ppid ..." — comm ends at the LAST ')'
        let Some(open) = stat.find('(') else { continue };
        let Some(close) = stat.rfind(')') else { continue };
        let comm = stat[open + 1..close].to_string();
        let rest: Vec<&str> = stat[close + 1..].split_whitespace().collect();
        let state = rest.first().and_then(|s| s.chars().next()).unwrap_or('?');
        // stat field 24 (rss, pages) = index 21 after the state field
        let rss_pages: i64 = rest.get(21).and_then(|v| v.parse().ok()).unwrap_or(0);
        out.push(ProcRow { pid, state, rss_kb: rss_pages * 4, name: comm });
    }
    out.sort_by_key(|p| -p.rss_kb);
    out
}

struct MountRow {
    mnt: String,
    fstype: String,
    used_mb: u64,
    total_mb: u64,
    pct: i32,
}

fn read_mounts() -> Vec<MountRow> {
    let mut out = Vec::new();
    let Ok(mounts) = std::fs::read_to_string("/proc/mounts") else {
        return out;
    };
    for line in mounts.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        let (Some(mnt), Some(ty)) = (f.get(1), f.get(2)) else {
            continue;
        };
        if !matches!(*ty, "ext4" | "vfat" | "tmpfs") {
            continue;
        }
        let c = std::ffi::CString::new(*mnt).unwrap();
        let mut vfs: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(c.as_ptr(), &mut vfs) } != 0 || vfs.f_blocks == 0 {
            continue;
        }
        let tot = vfs.f_blocks as u64 * vfs.f_frsize as u64;
        let free = vfs.f_bfree as u64 * vfs.f_frsize as u64;
        let used = tot - free;
        out.push(MountRow {
            mnt: mnt.to_string(),
            fstype: ty.to_string(),
            used_mb: used >> 20,
            total_mb: tot >> 20,
            pct: if tot > 0 { (used * 100 / tot) as i32 } else { 0 },
        });
    }
    out
}

fn read_klog() -> Vec<String> {
    let mut buf = vec![0u8; 64 * 1024];
    let n = unsafe { libc::klogctl(3, buf.as_mut_ptr() as *mut libc::c_char, buf.len() as i32) };
    if n <= 0 {
        return vec!["klogctl unavailable".into()];
    }
    String::from_utf8_lossy(&buf[..n as usize]).lines().map(|l| l.to_string()).collect()
}

impl App for SystemApp {
    fn title(&self) -> &'static str {
        "SYSTEM"
    }

    fn accent(&self) -> u32 {
        CYAN
    }

    fn tick(&mut self, _ctx: &mut Ctx) {
        // two-tap confirm disarms after 5 s, like the C UI
        if let Some((_, at)) = self.armed {
            if at.elapsed().as_secs() >= 5 {
                self.armed = None;
            }
        }
    }

    fn draw(&mut self, c: &mut Canvas, area: Rect, hits: &mut HitMap, ctx: &Ctx) {
        let (fw, fh) = (FONT_W as i32, FONT_H as i32);

        // tab strip
        let mut tx = area.x;
        for (i, name) in TABS.iter().enumerate() {
            let w = name.len() as i32 * fw + 30;
            let r = Rect::new(tx, area.y, w, 34);
            let on = i == self.tab;
            if on {
                c.fill(r.x, r.y, r.w, r.h, mix(BG, CYAN, 34));
            }
            c.neonbox(r.x, r.y, r.w, r.h, if on { CYAN } else { BORDER });
            c.text(r.x + 15, r.y + 6, name, if on { CYAN } else { TEXT2 }, 1);
            hits.add(r, HIT_TAB0 + i as u32);
            tx += w + 8;
        }

        // action bar (bottom of area)
        let abh = 44;
        let ay = area.y + area.h - abh;
        let gap = 10;
        let bw = (area.w - (ACTIONS.len() as i32 - 1) * gap) / ACTIONS.len() as i32;
        for (i, a) in ACTIONS.iter().enumerate() {
            let r = Rect::new(area.x + i as i32 * (bw + gap), ay, bw, abh);
            let armed = self.armed.map(|(ai, _)| ai == i).unwrap_or(false);
            let acc = if armed { RED } else { a.accent };
            c.fill(r.x, r.y, r.w, r.h, mix(BG, acc, if armed { 34 } else { 18 }));
            c.neonbox(r.x, r.y, r.w, r.h, acc);
            c.corners(r.x, r.y, r.w, r.h, acc, 10);
            let lbl = if armed { "CONFIRM?" } else { a.label };
            let lx = r.x + r.w / 2 - lbl.len() as i32 * fw / 2;
            if armed {
                c.textg(lx, r.y + r.h / 2 - fh / 2, lbl, acc, 1);
            } else {
                c.text(lx, r.y + r.h / 2 - fh / 2, lbl, acc, 1);
            }
            hits.add(r, HIT_ACT0 + i as u32);
        }

        // content panel between tabs and action bar
        let p = Rect::new(area.x, area.y + 42, area.w, area.h - 42 - abh - 10);
        c.panel(p.x, p.y, p.w, p.h, CYAN, &format!("SYSTEM // {}", TABS[self.tab]));
        let x = p.x + 20;
        match self.tab {
            0 => {
                // INFO
                let s = ctx.snap;
                let mut y = p.y + 24;
                let mut row = |c: &mut Canvas, k: &str, v: &str, col: u32| {
                    c.text(x, y, k, TEXT_DIM, 1);
                    c.text(x + 130, y, v, col, 1);
                    y += 30;
                };
                row(c, "HOST", &s.host, TEXT);
                row(c, "KERNEL", &s.kernel, TEXT);
                row(c, "ARCH", &s.machine, TEXT);
                let cpu = s.cpu_pct.map(|p| format!("{} cores  {}% busy", s.cpus, p))
                    .unwrap_or_else(|| format!("{} cores", s.cpus));
                row(c, "CPU", &cpu, TEXT);
                let up = format!(
                    "{}h {:02}m {:02}s",
                    s.uptime_s / 3600,
                    (s.uptime_s % 3600) / 60,
                    s.uptime_s % 60
                );
                row(c, "UPTIME", &up, GREEN);
                // loadavg on this kernel is inflated by vendor D-state kthreads; label it
                row(c, "LOADAVG", &format!("{} (raw)", s.load1), TEXT_DIM);
                let used = s.mem_total_kb - s.mem_avail_kb;
                row(c, "MEM", &format!("{} / {} MB", used / 1024, s.mem_total_kb / 1024), TEXT2);
                let pct = if s.mem_total_kb > 0 { (used * 100 / s.mem_total_kb) as i32 } else { 0 };
                c.bar(x + 130, y - 4, p.w - 200, 18, pct, CYAN);
            }
            1 => {
                // PROC
                let procs = read_procs();
                let lh = fh + 6;
                let mut y = p.y + 20;
                c.text(x, y, &format!("{:<6} {:<3} {:>9}  COMMAND", "PID", "ST", "RSS-KB"), TEXT2, 1);
                y += lh + 2;
                c.hline(x, y - 4, p.w - 40, mix(BG, GREEN, 60));
                let rows = ((p.h - 80) / lh) as usize;
                for pr in procs.iter().take(rows) {
                    let col = match pr.state {
                        'R' => GREEN,
                        'D' => AMBER,
                        _ => TEXT,
                    };
                    let line = format!("{:<6} {:<3} {:>9}  {}", pr.pid, pr.state, pr.rss_kb, pr.name);
                    c.text(x, y, &line, col, 1);
                    y += lh;
                }
                c.text(x, p.y + p.h - 26, &format!("{} processes", procs.len()), TEXT2, 1);
            }
            2 => {
                // DISK
                let mut y = p.y + 22;
                let mounts = read_mounts();
                if mounts.is_empty() {
                    c.text(x, y, "no mounted filesystems", TEXT2, 1);
                }
                for m in &mounts {
                    if y > p.y + p.h - 50 {
                        break;
                    }
                    let line = format!(
                        "{:<14} {:<6} {:>5} / {:<5} MB  {}%",
                        m.mnt, m.fstype, m.used_mb, m.total_mb, m.pct
                    );
                    c.text(x, y, &line, WHITE, 1);
                    c.bar(x, y + fh + 2, p.w - 60, 12, m.pct, AMBER);
                    y += fh + 20;
                }
            }
            _ => {
                // KLOG
                let lines = read_klog();
                let lh = fh + 3;
                let rows = ((p.h - 30) / lh) as usize;
                let start = lines.len().saturating_sub(rows);
                let maxc = ((p.w - 40) / fw) as usize;
                let mut y = p.y + 16;
                for l in &lines[start..] {
                    let mut s = l.as_str();
                    if s.starts_with('<') {
                        if let Some(g) = s.find('>') {
                            s = &s[g + 1..];
                        }
                    }
                    let col = if l.contains("fail") || l.contains("error") {
                        RED
                    } else if l.contains("WMT") || l.contains("wlan") {
                        MAGENTA
                    } else {
                        TEXT2
                    };
                    let clipped: String = s.chars().take(maxc).collect();
                    c.text(x - 4, y, &clipped, col, 1);
                    y += lh;
                }
            }
        }

        // modal overlay: full-screen rect registered LAST -> captures every tap
        if let Some(ov) = &self.overlay {
            let o = Rect::new(area.x + 10, 20, area.w - 20, area.y + area.h - 30);
            hits.add(Rect::new(0, 0, c.w, c.h), HIT_OV_BG);
            c.fill(o.x, o.y, o.w, o.h, PANEL);
            c.neonbox(o.x, o.y, o.w, o.h, CYAN);
            c.corners(o.x, o.y, o.w, o.h, CYAN, 20);
            c.textg(o.x + 20, o.y + 9, ov.title, CYAN, 1);
            c.text(o.x + o.w - 4 * fw - 14, o.y + 9, "[X]", MAGENTA, 1);
            hits.add(Rect::new(o.x + o.w - 6 * fw - 14, o.y, 6 * fw + 14, 42), HIT_OV_X);
            c.hline(o.x + 14, o.y + 36, o.w - 28, mix(BG, CYAN, 70));

            let lh = fh + 3;
            let rows = ((o.h - 42 - 26) / lh) as usize;
            let maxc = ((o.w - 40) / fw) as usize;
            let by = o.y + 42;
            for (i, l) in ov.lines.iter().skip(ov.scroll).take(rows).enumerate() {
                let col = if l.contains("fail") || l.contains("error") || l.contains("Error") {
                    RED
                } else {
                    TEXT2
                };
                let clipped: String = l.chars().take(maxc).collect();
                c.text(o.x + 18, by + i as i32 * lh, &clipped, col, 1);
            }
            // scrollbar + scroll zones (upper 2/5 up, lower 3/5 down)
            if ov.lines.len() > rows {
                let track = o.h - 42 - 26;
                let kh = (track as usize * rows / ov.lines.len()).max(10) as i32;
                let ky = by + (track as usize * ov.scroll / ov.lines.len()) as i32;
                c.fill(o.x + o.w - 7, by, 3, track, mix(BG, CYAN, 40));
                c.fill(o.x + o.w - 7, ky, 3, kh, CYAN);
                let split = o.y + o.h * 2 / 5;
                hits.add(Rect::new(o.x, o.y + 42, o.w, split - o.y - 42), HIT_OV_UP);
                hits.add(Rect::new(o.x, split, o.w, o.y + o.h - split), HIT_OV_DN);
                let info = format!(
                    "{}-{} / {}   tap lower: down   upper: up   [X]/outside: close",
                    ov.scroll + 1,
                    (ov.scroll + rows).min(ov.lines.len()),
                    ov.lines.len()
                );
                c.text(o.x + 18, o.y + o.h - 20, &info, TEXT_DIM, 1);
            } else {
                let info = format!("{} lines   tap [X] or outside to close", ov.lines.len());
                c.text(o.x + 18, o.y + o.h - 20, &info, TEXT_DIM, 1);
            }
            // re-register [X] on top of the scroll zones
            hits.add(Rect::new(o.x + o.w - 6 * fw - 14, o.y, 6 * fw + 14, 42), HIT_OV_X);
        }
    }

    fn on_tap(&mut self, id: HitId, ctx: &mut Ctx) -> bool {
        // overlay first (it registered last, so these ids only fire when open)
        if self.overlay.is_some() {
            let rows_step = 12; // approx page; fine for logs
            match id {
                HIT_OV_X | HIT_OV_BG => self.overlay = None,
                HIT_OV_UP => {
                    if let Some(ov) = &mut self.overlay {
                        ov.scroll = ov.scroll.saturating_sub(rows_step);
                    }
                }
                HIT_OV_DN => {
                    if let Some(ov) = &mut self.overlay {
                        ov.scroll = (ov.scroll + rows_step).min(ov.lines.len().saturating_sub(1));
                    }
                }
                _ => self.overlay = None,
            }
            return true;
        }
        match id {
            i if (HIT_TAB0..HIT_TAB0 + TABS.len() as u32).contains(&i) => {
                self.tab = (i - HIT_TAB0) as usize;
                self.armed = None;
                true
            }
            i if (HIT_ACT0..HIT_ACT0 + ACTIONS.len() as u32).contains(&i) => {
                let ai = (i - HIT_ACT0) as usize;
                let a = &ACTIONS[ai];
                if let Some((title, cmd)) = a.cmd {
                    self.run_tool(title, cmd);
                    self.armed = None;
                } else if a.confirm && self.armed.map(|(x, _)| x) != Some(ai) {
                    self.armed = Some((ai, Instant::now()));
                    ctx.set_toast("tap again to confirm");
                } else {
                    self.armed = None;
                    self.do_action(ai, ctx);
                }
                true
            }
            _ => {
                if self.armed.take().is_some() {
                    return true; // tap elsewhere cancels
                }
                false
            }
        }
    }
}
