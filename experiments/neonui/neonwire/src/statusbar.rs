//! Top status bar (~44 px): back/home hotspot + app name on the left,
//! live status cluster (Wi-Fi, Tailscale, battery, mem, load, uptime) right.
//! cd-hud-bar styling: segments split by dim borders, tiny labels over values.

use neon_gfx::canvas::{mix, Canvas};
use neon_gfx::font_data::{FONT_H, FONT_W};
use neon_gfx::geom::Rect;
use neon_gfx::theme::*;

use crate::apps::{HitMap, HitId};
use crate::collectors::Snapshot;

pub const BAR_H: i32 = 44;
pub const HIT_HOME: HitId = 0xFFFF_0001;

pub fn draw(c: &mut Canvas, snap: &Snapshot, app_title: &str, accent: u32, hits: &mut HitMap) {
    let w = c.w;
    c.fill(0, 0, w, BAR_H, mix(BG, BG2, 200));
    c.hline(0, BAR_H - 2, w, mix(BG, accent, 90));
    c.hline(0, BAR_H - 1, w, accent);

    // left: home hotspot [<] + wordmark/app title
    c.textg(14, 10, "<", accent, 2);
    let title = if app_title.is_empty() { "NEONWIRE" } else { app_title };
    c.textg(44, 10, title, accent, 2);
    hits.add(Rect::new(0, 0, 44 + title.len() as i32 * FONT_W as i32 * 2, BAR_H), HIT_HOME);

    // right cluster, laid out right-to-left
    let mut x = w - 12;
    let seg = |c: &mut Canvas, x: &mut i32, label: &str, value: &str, vcol: u32| {
        let vw = value.len() as i32 * FONT_W as i32;
        let lw = label.len() as i32 * FONT_W as i32;
        let cell = vw.max(lw) + 14;
        *x -= cell;
        c.text(*x + (cell - lw) / 2, 4, label, TEXT_DIM, 1);
        c.text(*x + (cell - vw) / 2, 20, value, vcol, 1);
        *x -= 6;
        c.vline(*x + 2, 6, BAR_H - 14, mix(BG, BORDER, 160));
    };

    let up = format!("{:02}:{:02}", snap.uptime_s / 3600, (snap.uptime_s % 3600) / 60);
    seg(c, &mut x, "UP", &up, TEXT2);
    seg(c, &mut x, "LOAD", &snap.load1, TEXT2);
    let memv = if snap.mem_total_kb > 0 {
        format!("{}%", (snap.mem_total_kb - snap.mem_avail_kb) * 100 / snap.mem_total_kb)
    } else {
        "--".into()
    };
    seg(c, &mut x, "MEM", &memv, TEXT2);
    let (bv, bc) = match snap.batt_pct {
        Some(p) if snap.batt_charging => (format!("{p}%+"), GREEN),
        Some(p) if p <= 15 => (format!("{p}%"), RED),
        Some(p) => (format!("{p}%"), TEXT),
        None => ("??%".into(), TEXT_DIM),
    };
    seg(c, &mut x, "BAT", &bv, bc);
    if let Some(ip) = &snap.ts_ip {
        seg(c, &mut x, "TS", ip, PURPLE);
    }
    match (&snap.wlan_ip, snap.rssi_bars) {
        (Some(ip), bars) => {
            seg(c, &mut x, "WIFI", ip, GREEN);
            // RSSI bars glyph
            let b = bars.unwrap_or(0);
            x -= 26;
            for i in 0..4 {
                let bh = 4 + i * 4;
                let col = if i < b { GREEN } else { mix(BG, BORDER, 200) };
                c.fill(x + i * 5, 30 - bh, 4, bh, col);
            }
            x -= 6;
        }
        (None, _) => seg(c, &mut x, "WIFI", "DOWN", TEXT_DIM),
    }

    let _ = FONT_H;
}
