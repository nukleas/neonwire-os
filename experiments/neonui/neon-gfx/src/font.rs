//! Text rendering over the baked JetBrains Mono coverage atlas.
//! Port of fbgfx.h glyph/text/textg, plus a `tracking` parameter (extra px
//! between cells) for cyberdesign's wide-tracked uppercase labels.

use crate::canvas::{mix, Canvas};
use crate::font_data::{FONT_ALPHA, FONT_FIRST, FONT_H, FONT_LAST, FONT_W};
use crate::theme::BG;

impl Canvas<'_> {
    /// One glyph, integer-scaled, alpha-blended in rgb over background colour bg.
    pub fn glyph_bg(&mut self, x: i32, y: i32, ch: u8, rgb: u32, bg: u32, scale: i32) {
        if !(FONT_FIRST..=FONT_LAST).contains(&ch) {
            return;
        }
        let g = &FONT_ALPHA[(ch - FONT_FIRST) as usize * FONT_W * FONT_H..];
        for gy in 0..FONT_H as i32 {
            for gx in 0..FONT_W as i32 {
                let a = g[(gy as usize) * FONT_W + gx as usize] as i32;
                if a == 0 {
                    continue;
                }
                let c = mix(bg, rgb, a);
                for sy in 0..scale {
                    for sx in 0..scale {
                        self.px(x + gx * scale + sx, y + gy * scale + sy, c);
                    }
                }
            }
        }
    }

    pub fn glyph(&mut self, x: i32, y: i32, ch: u8, rgb: u32, scale: i32) {
        self.glyph_bg(x, y, ch, rgb, BG, scale);
    }

    /// Draw text; returns the x cursor after the last glyph.
    pub fn text(&mut self, x: i32, y: i32, s: &str, rgb: u32, scale: i32) -> i32 {
        self.text_tracked(x, y, s, rgb, scale, 0)
    }

    /// Text with extra per-cell tracking (px) — cyberdesign wide labels.
    pub fn text_tracked(&mut self, x: i32, y: i32, s: &str, rgb: u32, scale: i32, tracking: i32) -> i32 {
        let mut cx = x;
        let mut cy = y;
        for &b in s.as_bytes() {
            if b == b'\n' {
                cy += (FONT_H as i32 + 2) * scale;
                cx = x;
                continue;
            }
            self.glyph(cx, cy, b, rgb, scale);
            cx += FONT_W as i32 * scale + tracking;
        }
        cx
    }

    /// Text with a cyberdesign-style neon bloom (0 0 6px / 12px text-shadow).
    pub fn textg(&mut self, x: i32, y: i32, s: &str, rgb: u32, scale: i32) -> i32 {
        let halo = mix(BG, rgb, 90);
        let d = scale;
        self.text(x - d, y, s, halo, scale);
        self.text(x + d, y, s, halo, scale);
        self.text(x, y - d, s, halo, scale);
        self.text(x, y + d, s, halo, scale);
        self.text(x, y, s, rgb, scale)
    }

    /// Pixel width of a string at scale (no tracking).
    pub fn text_w(s: &str, scale: i32) -> i32 {
        s.len() as i32 * FONT_W as i32 * scale
    }
}
