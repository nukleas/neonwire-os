//! Top status bar — cyberdesign `cd-hud-bar`: compact segments, accent underline.
//! Single-line only (font is 25px; a 36px bar cannot hold label-over-value).

use neon_gfx::canvas::{mix, Canvas};
use neon_gfx::font_data::{FONT_H, FONT_W};
use neon_gfx::theme::*;

use crate::apps::{HitId, HitMap};
use crate::collectors::Snapshot;
use crate::rail::RAIL_W;

pub const BAR_H: i32 = 36;
/// Kept for shell compatibility (HOME is also registered on the rail).
pub const HIT_HOME: HitId = 0xFFFF_0001;

pub fn draw(c: &mut Canvas, snap: &Snapshot, app_title: &str, accent: u32, _hits: &mut HitMap) {
    let w = c.w;
    let (fw, fh) = (FONT_W as i32, FONT_H as i32);
    let ty = (BAR_H - fh) / 2; // vertically center the single text line

    // bar background + accent bottom edge (cd-hud-bar--top)
    c.fill(0, 0, w, BAR_H, mix(BG, accent, 12));
    c.fill(0, 0, w, BAR_H - 2, mix(BG, BG2, 210));
    c.hline(0, BAR_H - 2, w, mix(BG, accent, 100));
    c.hline(0, BAR_H - 1, w, accent);

    // left: brand + context
    let mut x = 10;
    c.textg(x, ty, "NW", accent, 1);
    x += 2 * fw + 10;
    c.vline(x, 6, BAR_H - 12, mix(BG, BORDER, 180));
    x += 8;
    let title = if app_title.is_empty() { "HOME" } else { app_title };
    c.text_tracked(x, ty, title, accent, 1, 1);

    // mark rail column boundary
    c.vline(RAIL_W - 1, 0, BAR_H, mix(BG, BORDER, 160));

    // right cluster, right-to-left — "LBL val" on one baseline
    let mut rx = w - 10;
    let seg = |c: &mut Canvas, x: &mut i32, label: &str, value: &str, vcol: u32| {
        let cell = (label.len() as i32 + 1 + value.len() as i32) * fw + 14;
        *x -= cell;
        let mut cx = *x + 6;
        c.text(cx, ty, label, TEXT_DIM, 1);
        cx += label.len() as i32 * fw + fw; // one space
        c.text(cx, ty, value, vcol, 1);
        *x -= 4;
        c.vline(*x + 1, 6, BAR_H - 12, mix(BG, BORDER, 160));
    };

    let up = format!("{:02}:{:02}", snap.uptime_s / 3600, (snap.uptime_s % 3600) / 60);
    seg(c, &mut rx, "UP", &up, TEXT2);
    let (cpuv, cpuc) = match snap.cpu_pct {
        Some(p) if p >= 80 => (format!("{p}%"), AMBER),
        Some(p) => (format!("{p}%"), TEXT2),
        None => ("--".into(), TEXT_DIM),
    };
    seg(c, &mut rx, "CPU", &cpuv, cpuc);
    let memv = if snap.mem_total_kb > 0 {
        format!("{}%", (snap.mem_total_kb - snap.mem_avail_kb) * 100 / snap.mem_total_kb)
    } else {
        "--".into()
    };
    seg(c, &mut rx, "MEM", &memv, TEXT2);
    let (bv, bc) = match snap.batt_pct {
        Some(p) if snap.batt_charging => (format!("{p}%+"), GREEN),
        Some(p) if p <= 15 => (format!("{p}%"), RED),
        Some(p) => (format!("{p}%"), TEXT),
        None => ("??".into(), TEXT_DIM),
    };
    seg(c, &mut rx, "BAT", &bv, bc);
    if let Some(ip) = &snap.ts_ip {
        let short = ip.rsplit('.').next().unwrap_or(ip.as_str());
        seg(c, &mut rx, "TS", &format!(".{short}"), PURPLE);
    }
    match (&snap.wlan_ip, snap.rssi_bars) {
        (Some(ip), bars) => {
            let short = ip.rsplit('.').next().unwrap_or(ip.as_str());
            seg(c, &mut rx, "W", &format!(".{short}"), GREEN);
            // RSSI bars share the baseline row
            let b = bars.unwrap_or(0);
            rx -= 20;
            for i in 0..4 {
                let bh = 3 + i * 3;
                let col = if i < b { GREEN } else { mix(BG, BORDER, 200) };
                c.fill(rx + i * 4, ty + fh - bh - 2, 3, bh, col);
            }
            rx -= 4;
        }
        (None, _) => seg(c, &mut rx, "W", "DN", TEXT_DIM),
    }
}
