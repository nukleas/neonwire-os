//! Home screen: 2x2 grid of app tiles.

use neon_gfx::canvas::{mix, Canvas};
use neon_gfx::font_data::FONT_W;
use neon_gfx::geom::Rect;
use neon_gfx::theme::*;

use super::{Ctx, HitMap};

pub struct TileDef {
    pub title: &'static str,
    pub sub: &'static str,
    pub accent: u32,
}

pub const TILES: [TileDef; 4] = [
    TileDef { title: "SYSTEM", sub: "STATS / PROC / LOGS", accent: CYAN },
    TileDef { title: "NETWORK", sub: "WIFI / TAILSCALE", accent: GREEN },
    TileDef { title: "CAMERA", sub: "SP2509 OPTICS", accent: MAGENTA },
    TileDef { title: "MUSIC", sub: "STRUDEL SEQ", accent: AMBER },
];

pub struct Home;

impl Home {
    pub fn draw(&self, c: &mut Canvas, area: Rect, hits: &mut HitMap, ctx: &Ctx) {
        // wordmark block
        c.textg(area.x + 8, area.y + 10, "NEONWIRE OS", CYAN, 3);
        c.text_tracked(area.x + 10, area.y + 92, "DL7006 // MT8127 // RUST SHELL", TEXT_MUTED, 1, 3);
        let host = format!("root@{}", ctx.snap.host);
        c.text(area.x + 10, area.y + 116, &host, GREEN, 1);

        // 2x2 tile grid
        let gy = area.y + 150;
        let gh = area.h - 160;
        let (tw, th) = ((area.w - 24) / 2, (gh - 24) / 2);
        for (i, t) in TILES.iter().enumerate() {
            let x = area.x + (i as i32 % 2) * (tw + 24);
            let y = gy + (i as i32 / 2) * (th + 24);
            c.fill(x, y, tw, th, PANEL);
            c.neonbox(x, y, tw, th, mix(BG, t.accent, 90));
            c.corners(x, y, tw, th, t.accent, 18);
            c.textg(x + 24, y + th / 2 - 28, t.title, t.accent, 2);
            c.text_tracked(x + 26, y + th / 2 + 14, t.sub, TEXT_DIM, 1, 2);
            let idx = format!("0{}", i + 1);
            c.text(x + tw - 2 * FONT_W as i32 - 14, y + 12, &idx, mix(BG, t.accent, 140), 1);
            hits.add(Rect::new(x, y, tw, th), i as u32);
        }
    }
}
