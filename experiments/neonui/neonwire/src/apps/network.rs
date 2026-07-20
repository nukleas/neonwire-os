//! NETWORK app — M4 scope: live link status readout. The full Wi-Fi manager
//! (wpa ctrl socket, scan list, keyboard) ports over in M6.

use neon_gfx::canvas::Canvas;
use neon_gfx::geom::Rect;
use neon_gfx::theme::*;

use super::{App, Ctx, HitId, HitMap};

pub struct NetworkApp;

impl App for NetworkApp {
    fn title(&self) -> &'static str {
        "NETWORK"
    }

    fn accent(&self) -> u32 {
        GREEN
    }

    fn draw(&mut self, c: &mut Canvas, area: Rect, _hits: &mut HitMap, ctx: &Ctx) {
        let s = ctx.snap;
        c.panel(area.x, area.y + 8, area.w, area.h - 16, GREEN, "NETWORK // LINKS");
        let x = area.x + 28;
        let mut y = area.y + 44;

        let mut link = |c: &mut Canvas, name: &str, ip: &Option<String>, extra: &str| {
            let (state, col) = match ip {
                Some(_) => ("ONLINE", GREEN),
                None => ("DOWN", TEXT_DIM),
            };
            c.textg(x, y, name, col, 1);
            c.text(x + 150, y, state, col, 1);
            if let Some(ip) = ip {
                c.text(x + 260, y, ip, TEXT, 1);
            }
            if !extra.is_empty() {
                c.text(x + 480, y, extra, TEXT2, 1);
            }
            y += 34;
        };
        let bars = s.rssi_bars.map(|b| format!("signal {b}/4")).unwrap_or_default();
        link(c, "WLAN0", &s.wlan_ip, &bars);
        link(c, "TAILSCALE0", &s.ts_ip, "mesh");

        y += 10;
        c.hline(x, y, area.w - 60, BORDER);
        y += 16;
        c.text(x, y, "wi-fi manager port lands in M6 —", TEXT_DIM, 1);
        y += 24;
        c.text(x, y, "scan / join / psk keyboard via wpa ctrl socket", TEXT_DIM, 1);
    }

    fn on_tap(&mut self, _id: HitId, _ctx: &mut Ctx) -> bool {
        false
    }
}
