//! Left navigation rail — cyberdesign `cd-sidebar` / neuromancer left panel.
//! Always visible: app switch without bouncing through a sparse home grid.

use neon_gfx::canvas::{mix, Canvas};
use neon_gfx::font_data::{FONT_H, FONT_W};
use neon_gfx::geom::Rect;
use neon_gfx::theme::*;

use crate::apps::home::TILES;
use crate::apps::{HitId, HitMap};
use crate::statusbar::HIT_HOME;

/// Width of the permanent left rail (px).
pub const RAIL_W: i32 = 112;

/// Hit base for app rows: HIT_RAIL_APP0 + app_index.
pub const HIT_RAIL_APP0: HitId = 0xFFFF_0010;

/// One glyph-tall row + padding. Font is 25px — never stack two labels here.
const ROW_H: i32 = 38;

/// Draw the rail. `active`: None = HOME, Some(i) = apps[i].
pub fn draw(c: &mut Canvas, active: Option<usize>, hits: &mut HitMap) {
    let (fw, fh) = (FONT_W as i32, FONT_H as i32);
    let h = c.h;
    let bar = crate::statusbar::BAR_H;

    // rail body under the status bar
    c.fill(0, bar, RAIL_W, h - bar, mix(BG, BG2, 220));
    c.vline(RAIL_W - 1, bar, h - bar, mix(BG, BORDER, 200));
    c.vline(RAIL_W - 2, bar, h - bar, mix(BG, CYAN, 30));

    // brand: single line (no secondary under-title — it collided with NAV)
    let mut y = bar + 8;
    c.text_tracked(8, y, "NEON", CYAN, 1, 1);
    y += fh + 4;
    c.hline(8, y, RAIL_W - 16, mix(BG, CYAN, 50));
    y += 6;

    // section: NAV
    c.text(8, y, "> NAV", mix(BG, CYAN, 160), 1);
    y += fh + 2;

    // HOME — single line "01 HOME"
    y = draw_link(c, hits, y, "01", "HOME", CYAN, active.is_none(), HIT_HOME, fw, fh);

    // apps
    for (i, t) in TILES.iter().enumerate() {
        let tag = match i {
            0 => "SYS",
            1 => "NET",
            2 => "HA",
            3 => "FIL",
            4 => "INT",
            5 => "CAM",
            6 => "MUS",
            7 => "SNG",
            8 => "AI",
            _ => "APP",
        };
        // short label always — full titles overflow the rail width
        let idx = format!("{:02}", i + 2);
        y = draw_link(
            c,
            hits,
            y,
            &idx,
            tag,
            t.accent,
            active == Some(i),
            HIT_RAIL_APP0 + i as u32,
            fw,
            fh,
        );
        if y > h - 50 {
            break;
        }
    }

    // footer live mark
    let fy = h - 32;
    c.hline(8, fy - 6, RAIL_W - 16, mix(BG, BORDER, 160));
    c.fill(10, fy + 8, 7, 7, GREEN);
    c.text(22, fy + 2, "LIVE", GREEN, 1);
}

/// Single-line nav row: dim index + label, vertically centered. No secondary text.
fn draw_link(
    c: &mut Canvas,
    hits: &mut HitMap,
    y: i32,
    idx: &str,
    label: &str,
    accent: u32,
    on: bool,
    hit: HitId,
    fw: i32,
    fh: i32,
) -> i32 {
    let r = Rect::new(0, y, RAIL_W - 1, ROW_H);
    if on {
        c.fill(r.x, r.y, r.w, r.h, mix(BG, accent, 28));
        // left accent bar (neuromancer entity.is-active)
        c.fill(0, r.y + 4, 3, r.h - 8, accent);
        c.fill(3, r.y + 4, 1, r.h - 8, mix(BG, accent, 120));
    }
    let ty = r.y + (r.h - fh) / 2;
    let icol = if on { mix(BG, accent, 170) } else { TEXT_DIM };
    let lcol = if on { accent } else { TEXT2 };
    c.text(10, ty, idx, icol, 1);
    let lx = 10 + idx.len() as i32 * fw + 6;
    if on {
        c.textg(lx, ty, label, lcol, 1);
    } else {
        c.text(lx, ty, label, lcol, 1);
    }
    if on {
        c.text(RAIL_W - fw - 6, ty, ">", accent, 1);
    }
    hits.add(r, hit);
    y + ROW_H + 2
}
