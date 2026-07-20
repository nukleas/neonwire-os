//! Composite drawing primitives: neon boxes, panels, bars, CRT overlays.
//! Straight port of fbgfx.h; geometry and blend factors kept identical so the
//! M2 test card can be compared against the C renderer pixel-for-pixel.

use crate::canvas::{mix, Canvas};
use crate::theme::{BG, BORDER, CYANHI, GRID, MAGENTA, PANEL};

impl Canvas<'_> {
    /// Neon rectangle: bright edge + dim inner halo.
    pub fn neonbox(&mut self, x: i32, y: i32, w: i32, h: i32, rgb: u32) {
        let halo = mix(BG, rgb, 70);
        self.hline(x, y, w, rgb);
        self.hline(x, y + h - 1, w, rgb);
        self.vline(x, y, h, rgb);
        self.vline(x + w - 1, y, h, rgb);
        self.hline(x + 1, y + 1, w - 2, halo);
        self.hline(x + 1, y + h - 2, w - 2, halo);
        self.vline(x + 1, y + 1, h - 2, halo);
        self.vline(x + w - 2, y + 1, h - 2, halo);
    }

    /// L-shaped corner brackets, brighter than the panel border.
    pub fn corners(&mut self, x: i32, y: i32, w: i32, h: i32, rgb: u32, len: i32) {
        self.hline(x, y, len, rgb);
        self.vline(x, y, len, rgb);
        self.hline(x + w - len, y, len, rgb);
        self.vline(x + w - 1, y, len, rgb);
        self.hline(x, y + h - 1, len, rgb);
        self.vline(x, y + h - len, len, rgb);
        self.hline(x + w - len, y + h - 1, len, rgb);
        self.vline(x + w - 1, y + h - len, len, rgb);
    }

    /// cyberdesign panel: fill + dim border + bright corner brackets + title tab.
    pub fn panel(&mut self, x: i32, y: i32, w: i32, h: i32, accent: u32, title: &str) {
        self.fill(x, y, w, h, PANEL);
        self.neonbox(x, y, w, h, BORDER);
        self.corners(x, y, w, h, accent, 16);
        if !title.is_empty() {
            self.textg(x + 24, y - (crate::font_data::FONT_H as i32) / 2 - 1, title, accent, 1);
        }
    }

    /// Horizontal meter, accent→magenta sweep with a bright cap.
    pub fn bar(&mut self, x: i32, y: i32, w: i32, h: i32, pct: i32, rgb: u32) {
        self.fill(x, y, w, h, mix(BG, BORDER, 60));
        self.neonbox(x, y, w, h, BORDER);
        let pct = pct.clamp(0, 100);
        let fillw = (w - 4) * pct / 100;
        for i in 0..fillw {
            self.vline(x + 2 + i, y + 2, h - 4, mix(rgb, MAGENTA, i * 255 / (w - 4)));
        }
        if fillw > 0 {
            self.vline(x + 1 + fillw, y + 2, h - 4, CYANHI);
        }
    }

    /// CRT scanline overlay: darken 2 of every 4 rows (~18%), like cd-scanline.
    pub fn scanlines(&mut self, x: i32, y: i32, w: i32, h: i32) {
        for j in y..y + h {
            if (j & 3) < 2 {
                continue;
            }
            for i in x..x + w {
                let c = self.get(i, j);
                self.px(i, j, mix(c, 0x000000, 46));
            }
        }
    }

    /// Faint accent pixel grid every 24px.
    pub fn pixelgrid(&mut self) {
        let (w, h) = (self.w, self.h);
        for y in (0..h).step_by(24) {
            for x in 0..w {
                let c = self.get(x, y);
                self.px(x, y, mix(c, GRID, 14));
            }
        }
        for x in (0..w).step_by(24) {
            for y in 0..h {
                let c = self.get(x, y);
                self.px(x, y, mix(c, GRID, 14));
            }
        }
    }

    /// Vertical gradient background + pixel grid.
    pub fn background(&mut self) {
        let (w, h) = (self.w, self.h);
        for y in 0..h {
            self.hline(0, y, w, mix(BG, 0x070a12, y * 255 / h));
        }
        self.pixelgrid();
    }
}
