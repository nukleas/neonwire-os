//! Home = dense cyberdesign dashboard (not a sparse tile launcher).
//! App switching lives on the left rail; this screen is telemetry + overview.

use neon_gfx::canvas::{mix, Canvas};
use neon_gfx::font_data::{FONT_H, FONT_W};
use neon_gfx::geom::Rect;
use neon_gfx::theme::*;

use super::{Ctx, HitMap};

/// Order must match `Shell::apps` indices (rail + any residual tile hits).
pub struct TileDef {
    pub title: &'static str,
    /// Kept for future rail tooltips / module detail; not drawn (25px font).
    #[allow(dead_code)]
    pub sub: &'static str,
    pub accent: u32,
}

pub const TILES: [TileDef; 7] = [
    TileDef { title: "SYSTEM", sub: "STATS / PROC / LOGS", accent: CYAN },
    TileDef { title: "NETWORK", sub: "WIFI / TAILSCALE", accent: GREEN },
    TileDef { title: "HOUSE", sub: "HOME ASSISTANT", accent: GREEN },
    TileDef { title: "FILES", sub: "SD / LAB / PREVIEW", accent: BLUE },
    TileDef { title: "INTEL", sub: "OCINT FEED", accent: AMBER },
    TileDef { title: "CAMERA", sub: "SP2509 OPTICS", accent: MAGENTA },
    TileDef { title: "MUSIC", sub: "STRUDEL SEQ", accent: GOLD },
];

/// Hit ids for the compact app launcher row on the home dashboard.
pub const HIT_LAUNCH0: u32 = 0xA000;

pub struct Home;

impl Home {
    pub fn draw(&self, c: &mut Canvas, area: Rect, hits: &mut HitMap, ctx: &Ctx) {
        let (fw, fh) = (FONT_W as i32, FONT_H as i32);
        let s = ctx.snap;
        let pad = 10;
        let mut y = area.y + 4;

        // --- title strip (cd-section-header) ---
        c.text(area.x + pad, y, "> OVERVIEW", CYAN, 1);
        let host = if s.host.is_empty() { "dl7006" } else { s.host.as_str() };
        let meta = format!("root@{host}  k {}", s.kernel);
        c.text(area.x + area.w - meta.len() as i32 * fw - pad, y, &meta, TEXT_DIM, 1);
        y += fh + 4;
        c.hline(area.x + pad, y, area.w - pad * 2, mix(BG, CYAN, 55));
        y += 8;

        // --- stat grid (cd-stat / glow cards) ---
        // Font is 25px: a card only fits label + value cleanly. Sub rides the
        // value line as a trailing dim suffix when width allows.
        let stats = build_stats(s);
        let cols = 4i32;
        let gap = 6;
        let sw = (area.w - pad * 2 - gap * (cols - 1)) / cols;
        // 2 lines of 25 + 8 pad top/bottom + 4 gap between ≈ 62
        let sh = 62;
        for (i, st) in stats.iter().enumerate() {
            let col = i as i32 % cols;
            let row = i as i32 / cols;
            let x = area.x + pad + col * (sw + gap);
            let sy = y + row * (sh + gap);
            draw_stat(c, x, sy, sw, sh, st);
        }
        let stat_rows = ((stats.len() as i32) + cols - 1) / cols;
        y += stat_rows * (sh + gap) + 4;

        // --- subsystems (single-line rows: label | value | optional bar) ---
        c.text(area.x + pad, y, "> SUBSYSTEMS", MAGENTA, 1);
        y += fh + 2;
        c.hline(area.x + pad, y, area.w - pad * 2, mix(BG, MAGENTA, 45));
        y += 6;

        let subs = subsystem_rows(s);
        let row_h = fh + 8; // one text line + pad
        for (label, value, col, bar_pct) in &subs {
            // leave room for the module launcher strip (~one text line + chrome)
            if y + row_h > area.y + area.h - 56 {
                break;
            }
            let ty = y + (row_h - fh) / 2;
            c.fill(area.x + pad, y + 2, 2, row_h - 4, *col);
            c.text(area.x + pad + 10, ty, label, TEXT_DIM, 1);
            c.text(area.x + pad + 10 + 10 * fw, ty, value, *col, 1);
            if let Some(pct) = bar_pct {
                let bx = area.x + area.w - pad - 120;
                c.bar(bx, y + (row_h - 12) / 2, 110, 12, *pct, *col);
            }
            y += row_h;
        }
        y += 4;

        // --- module launcher: single-line buttons (title only) ---
        c.text(area.x + pad, y, "> MODULES", AMBER, 1);
        y += fh + 2;
        c.hline(area.x + pad, y, area.w - pad * 2, mix(BG, AMBER, 45));
        y += 6;

        let n = TILES.len() as i32;
        let tw = (area.w - pad * 2 - gap * (n - 1)) / n;
        // one glyph line + neonbox padding — never stack title + sub at 25px font
        let th = (area.y + area.h - y - 4).clamp(fh + 14, fh + 22);
        for (i, t) in TILES.iter().enumerate() {
            let x = area.x + pad + i as i32 * (tw + gap);
            let r = Rect::new(x, y, tw, th);
            c.fill(r.x, r.y, r.w, r.h, PANEL);
            c.neonbox(r.x, r.y, r.w, r.h, mix(BG, t.accent, 90));
            c.corners(r.x, r.y, r.w, r.h, t.accent, 8);
            // single centered line: "01 SYSTEM" (drop sub — it collided)
            let line = format!("{:02} {}", i + 1, t.title);
            let maxc = ((r.w - 12) / fw).max(4) as usize;
            let shown: String = line.chars().take(maxc).collect();
            let lx = r.x + (r.w - shown.len() as i32 * fw) / 2;
            let ly = r.y + (r.h - fh) / 2;
            c.textg(lx, ly, &shown, t.accent, 1);
            hits.add(r, HIT_LAUNCH0 + i as u32);
        }
    }
}

struct StatCard {
    label: &'static str,
    value: String,
    sub: String,
    accent: u32,
}

fn build_stats(s: &crate::collectors::Snapshot) -> Vec<StatCard> {
    let cpu = s.cpu_pct.map(|p| format!("{p}%")).unwrap_or_else(|| "--".into());
    let cpu_a = match s.cpu_pct {
        Some(p) if p >= 80 => AMBER,
        Some(_) => GREEN,
        None => TEXT_DIM,
    };
    let mem_pct = if s.mem_total_kb > 0 {
        ((s.mem_total_kb - s.mem_avail_kb) * 100 / s.mem_total_kb) as i32
    } else {
        0
    };
    let mem = if s.mem_total_kb > 0 {
        format!("{mem_pct}%")
    } else {
        "--".into()
    };
    let mem_sub = if s.mem_total_kb > 0 {
        format!(
            "{}/{}M",
            (s.mem_total_kb - s.mem_avail_kb) / 1024,
            s.mem_total_kb / 1024
        )
    } else {
        "n/a".into()
    };
    let (bat, bat_a, bat_sub) = match s.batt_pct {
        Some(p) if s.batt_charging => (format!("{p}%"), GREEN, "CHG".into()),
        Some(p) if p <= 15 => (format!("{p}%"), RED, "LOW".into()),
        Some(p) => (format!("{p}%"), TEXT, "DSG".into()),
        None => ("??".into(), TEXT_DIM, "n/a".into()),
    };
    let (wifi, wifi_a, wifi_sub) = match &s.wlan_ip {
        Some(ip) => {
            let short = ip.rsplit('.').next().unwrap_or(ip);
            (format!(".{short}"), GREEN, "UP".into())
        }
        None => ("DOWN".into(), RED, "wlan0".into()),
    };
    let up = format!("{:02}:{:02}", s.uptime_s / 3600, (s.uptime_s % 3600) / 60);
    let ts = s.ts_ip.as_deref().map(|ip| {
        let tail = ip.rsplit('.').next().unwrap_or(ip);
        format!(".{tail}")
    });

    let mut v = vec![
        StatCard { label: "CPU", value: cpu, sub: format!("{}c load {}", s.cpus, s.load1), accent: cpu_a },
        StatCard { label: "MEM", value: mem, sub: mem_sub, accent: if mem_pct >= 85 { AMBER } else { CYAN } },
        StatCard { label: "BAT", value: bat, sub: bat_sub, accent: bat_a },
        StatCard { label: "WIFI", value: wifi, sub: wifi_sub, accent: wifi_a },
        StatCard { label: "UP", value: up, sub: "hh:mm".into(), accent: TEXT2 },
        StatCard {
            label: "TS",
            value: ts.unwrap_or_else(|| "OFF".into()),
            sub: if s.ts_ip.is_some() { "tailnet".into() } else { "down".into() },
            accent: if s.ts_ip.is_some() { PURPLE } else { TEXT_DIM },
        },
        StatCard {
            label: "HOST",
            value: s.host.chars().take(8).collect(),
            sub: s.machine.clone(),
            accent: CYANHI,
        },
        StatCard {
            label: "KERN",
            value: s.kernel.chars().take(8).collect(),
            sub: "3.18 l1".into(),
            accent: TEXT2,
        },
    ];
    // keep 8 = 2x4
    v.truncate(8);
    v
}

fn draw_stat(c: &mut Canvas, x: i32, y: i32, w: i32, h: i32, st: &StatCard) {
    let (fw, fh) = (FONT_W as i32, FONT_H as i32);
    c.fill(x, y, w, h, PANEL);
    c.neonbox(x, y, w, h, mix(BG, st.accent, 70));
    c.corners(x, y, w, h, st.accent, 8);
    // Two lines only: LABEL on top, VALUE below. Sub is appended to the value
    // line when it fits — never a third stacked row (25px font can't).
    let ly = y + 4;
    c.text_tracked(x + 8, ly, st.label, TEXT_DIM, 1, 2);
    let vy = y + 4 + fh; // exactly one glyph below the label
    // if the card is short, clamp value into remaining space
    let vy = vy.min(y + h - fh - 4);
    c.textg(x + 8, vy, &st.value, st.accent, 1);
    // trailing sub on the same baseline as value, right-aligned if room
    let v_end = x + 8 + st.value.len() as i32 * fw + 8;
    let sub_w = st.sub.len() as i32 * fw;
    if v_end + sub_w + 6 < x + w && !st.sub.is_empty() {
        c.text(x + w - sub_w - 8, vy, &st.sub, TEXT_MUTED, 1);
    }
}

fn subsystem_rows(s: &crate::collectors::Snapshot) -> Vec<(String, String, u32, Option<i32>)> {
    let mut rows = Vec::new();
    match &s.wlan_ip {
        Some(ip) => rows.push(("WLAN0".into(), format!("ONLINE  {ip}"), GREEN, s.rssi_bars.map(|b| b * 25))),
        None => rows.push(("WLAN0".into(), "STACK DOWN".into(), RED, None)),
    }
    match &s.ts_ip {
        Some(ip) => rows.push(("TAILSCALE".into(), format!("NODE  {ip}"), PURPLE, None)),
        None => rows.push(("TAILSCALE".into(), "OFF / NO IP".into(), TEXT_DIM, None)),
    }
    let mem_pct = if s.mem_total_kb > 0 {
        ((s.mem_total_kb - s.mem_avail_kb) * 100 / s.mem_total_kb) as i32
    } else {
        0
    };
    rows.push((
        "MEMORY".into(),
        format!(
            "{} / {} MB",
            (s.mem_total_kb - s.mem_avail_kb) / 1024,
            s.mem_total_kb / 1024
        ),
        if mem_pct >= 85 { AMBER } else { CYAN },
        Some(mem_pct),
    ));
    match s.batt_pct {
        Some(p) => {
            let st = if s.batt_charging { "CHARGING" } else { "DISCHARGE" };
            let col = if p <= 15 && !s.batt_charging { RED } else if s.batt_charging { GREEN } else { TEXT };
            rows.push(("POWER".into(), format!("{p}%  {st}"), col, Some(p)));
        }
        None => rows.push(("POWER".into(), "UNKNOWN".into(), TEXT_DIM, None)),
    }
    rows.push(("DISPLAY".into(), "mtkfb 1024x600  PAN".into(), BLUE, None));
    rows.push(("AUDIO".into(), "hw:0,5  S16/44.1k".into(), AMBER, None));
    rows
}
