//! Canvas: low-level pixel access over the composed back buffer.
//! Colors are 0xRRGGBB at the API level; packed to the native layout on write.

use crate::fb::{Fb, FbVarScreeninfo};

pub struct Canvas<'a> {
    buf: &'a mut [u8],
    pub w: i32,
    pub h: i32,
    stride: usize,
    vi: FbVarScreeninfo,
}

impl Fb {
    pub fn canvas(&mut self) -> Canvas<'_> {
        Canvas {
            w: self.xres as i32,
            h: self.yres as i32,
            stride: self.stride as usize,
            vi: self.vi,
            buf: &mut self.back,
        }
    }
}

/// Alpha 0..255 blend of a toward b (both 0xRRGGBB).
#[inline]
pub fn mix(a: u32, b: u32, t: i32) -> u32 {
    let (ar, ag, ab) = ((a >> 16 & 0xff) as i32, (a >> 8 & 0xff) as i32, (a & 0xff) as i32);
    let (br, bg, bb) = ((b >> 16 & 0xff) as i32, (b >> 8 & 0xff) as i32, (b & 0xff) as i32);
    let r = ar + (br - ar) * t / 255;
    let g = ag + (bg - ag) * t / 255;
    let bl = ab + (bb - ab) * t / 255;
    ((r as u32) << 16) | ((g as u32) << 8) | bl as u32
}

impl Canvas<'_> {
    #[inline]
    fn pack(&self, rgb: u32) -> u32 {
        let (r, g, b) = ((rgb >> 16) & 0xff, (rgb >> 8) & 0xff, rgb & 0xff);
        let v = &self.vi;
        ((r >> (8 - v.red.length)) << v.red.offset)
            | ((g >> (8 - v.green.length)) << v.green.offset)
            | ((b >> (8 - v.blue.length)) << v.blue.offset)
            | if v.transp.length > 0 {
                (0xffu32 >> (8 - v.transp.length)) << v.transp.offset
            } else {
                0
            }
    }

    #[inline]
    fn unpack(&self, v: u32) -> u32 {
        let f = &self.vi;
        let r = (v >> f.red.offset) & ((1 << f.red.length) - 1);
        let g = (v >> f.green.offset) & ((1 << f.green.length) - 1);
        let b = (v >> f.blue.offset) & ((1 << f.blue.length) - 1);
        ((r << (8 - f.red.length)) << 16) | ((g << (8 - f.green.length)) << 8) | (b << (8 - f.blue.length))
    }

    #[inline]
    pub fn px(&mut self, x: i32, y: i32, rgb: u32) {
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            return;
        }
        let off = y as usize * self.stride + x as usize * 4;
        let p = self.pack(rgb).to_ne_bytes();
        self.buf[off..off + 4].copy_from_slice(&p);
    }

    /// Read a pixel back as 0xRRGGBB (for scanlines/pixelgrid read-modify-write).
    #[inline]
    pub fn get(&self, x: i32, y: i32) -> u32 {
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            return 0;
        }
        let off = y as usize * self.stride + x as usize * 4;
        let v = u32::from_ne_bytes(self.buf[off..off + 4].try_into().unwrap());
        self.unpack(v)
    }

    pub fn fill(&mut self, x: i32, y: i32, w: i32, h: i32, rgb: u32) {
        // clip once, then write rows directly (hot path)
        let x0 = x.max(0);
        let y0 = y.max(0);
        let x1 = (x + w).min(self.w);
        let y1 = (y + h).min(self.h);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let p = self.pack(rgb).to_ne_bytes();
        for j in y0..y1 {
            let off = j as usize * self.stride + x0 as usize * 4;
            let row = &mut self.buf[off..off + (x1 - x0) as usize * 4];
            for chunk in row.chunks_exact_mut(4) {
                chunk.copy_from_slice(&p);
            }
        }
    }

    pub fn hline(&mut self, x: i32, y: i32, w: i32, rgb: u32) {
        self.fill(x, y, w, 1, rgb);
    }

    pub fn vline(&mut self, x: i32, y: i32, h: i32, rgb: u32) {
        self.fill(x, y, 1, h, rgb);
    }
}
