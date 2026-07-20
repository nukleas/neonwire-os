//! Reusable widgets. First resident: TextPrompt — the 3-page on-screen
//! keyboard modal (port of neui.c draw_kb/kb_tap), generalized so Network
//! uses it for PSKs and future apps (strudel live-code input) can too.

use neon_gfx::canvas::{mix, Canvas};
use neon_gfx::font_data::{FONT_H, FONT_W};
use neon_gfx::geom::Rect;
use neon_gfx::theme::*;

use crate::apps::{HitId, HitMap};

// HitId block reserved for the prompt (registered last => modal capture)
pub const KB_BASE: HitId = 0xE000;
const KB_KEY0: HitId = KB_BASE; // + r*16 + c
const KB_X: HitId = KB_BASE + 0x100;
const KB_CANCEL: HitId = KB_BASE + 0x101;
const KB_SPACE: HitId = KB_BASE + 0x102;
const KB_PAGE: HitId = KB_BASE + 0x103;
const KB_SUBMIT: HitId = KB_BASE + 0x104;
const KB_BG: HitId = KB_BASE + 0x105;

const PAGES: [[&str; 4]; 3] = [
    ["1234567890", "qwertyuiop", "asdfghjkl-", "\x01zxcvbnm_\x08"],
    ["1234567890", "QWERTYUIOP", "ASDFGHJKL-", "\x01ZXCVBNM_\x08"],
    ["!@#$%^&*()", "-_=+[]{}\\|", ";:'\",.<>/?", "\x01~`      \x08"],
];
const PAGE_NAMES: [&str; 3] = ["abc", "ABC", "#+="];
const GAP: i32 = 8;

pub enum PromptResult {
    Open,
    Cancelled,
    Submitted(String),
}

pub struct TextPrompt {
    pub title: String,
    pub buf: String,
    pub min_len: usize, // 0 = no minimum; submit disabled below this
    page: usize,
    submit_label: &'static str,
}

impl TextPrompt {
    pub fn new(title: impl Into<String>, min_len: usize, submit_label: &'static str) -> TextPrompt {
        TextPrompt { title: title.into(), buf: String::new(), min_len, page: 0, submit_label }
    }

    fn geom(&self, c: &Canvas) -> (Rect, i32, i32, i32, i32) {
        let o = Rect::new(30, 14, c.w - 60, c.h - 28);
        let kw = (o.w - 36 - 9 * GAP) / 10;
        let kh = 58;
        let gx = o.x + 18 + (o.w - 36 - 10 * kw - 9 * GAP) / 2;
        let gy = o.y + 118;
        (o, kw, kh, gx, gy)
    }

    fn act_rect(&self, c: &Canvas, i: usize) -> Rect {
        let (_, kw, kh, gx, gy) = self.geom(c);
        let ay = gy + 4 * (kh + GAP) + 6;
        let seg = [2i32, 4, 2, 2];
        let mut x = gx;
        for k in 0..i {
            x += seg[k] * kw + seg[k] * GAP;
        }
        Rect::new(x, ay, seg[i] * kw + (seg[i] - 1) * GAP, kh + 6)
    }

    pub fn draw(&self, c: &mut Canvas, hits: &mut HitMap) {
        let (fw, fh) = (FONT_W as i32, FONT_H as i32);
        let (o, kw, kh, gx, gy) = self.geom(c);
        hits.add(Rect::new(0, 0, c.w, c.h), KB_BG); // modal: swallow everything

        c.fill(o.x, o.y, o.w, o.h, PANEL);
        c.neonbox(o.x, o.y, o.w, o.h, MAGENTA);
        c.corners(o.x, o.y, o.w, o.h, MAGENTA, 20);
        c.textg(o.x + 20, o.y + 12, &self.title, MAGENTA, 1);
        c.text(o.x + o.w - 4 * fw - 16, o.y + 12, "[X]", CYAN, 1);
        hits.add(Rect::new(o.x + o.w - 6 * fw - 16, o.y, 6 * fw + 16, 44), KB_X);

        // input box
        let (ix, iy, iw, ih) = (o.x + 18, o.y + 48, o.w - 36, 44);
        c.fill(ix, iy, iw, ih, BG);
        c.neonbox(ix, iy, iw, ih, if self.buf.is_empty() { BORDER } else { CYAN });
        let shown = format!("{}_", self.buf);
        c.text(ix + 14, iy + ih / 2 - fh / 2, &shown, CYANHI, 1);
        let short = self.min_len > 0 && !self.buf.is_empty() && self.buf.len() < self.min_len;
        let count = format!(
            "{} chars{}",
            self.buf.len(),
            if short { format!("  (min {})", self.min_len) } else { String::new() }
        );
        c.text(
            ix + iw - count.len() as i32 * fw - 12,
            iy + ih / 2 - fh / 2,
            &count,
            if short { AMBER } else { TEXT_DIM },
            1,
        );

        // key grid
        for (r, row) in PAGES[self.page].iter().enumerate() {
            for (col, ch) in row.bytes().enumerate() {
                if ch == b' ' {
                    continue;
                }
                let x = gx + col as i32 * (kw + GAP);
                let y = gy + r as i32 * (kh + GAP);
                let (lbl, acc): (String, u32) = match ch {
                    0x01 => (PAGE_NAMES[(self.page + 1) % 3].into(), AMBER),
                    0x08 => ("DEL".into(), RED),
                    ch => ((ch as char).to_string(), CYAN),
                };
                c.fill(x, y, kw, kh, mix(BG, acc, 12));
                c.neonbox(x, y, kw, kh, mix(BG, acc, 120));
                c.text(x + kw / 2 - lbl.len() as i32 * fw / 2, y + kh / 2 - fh / 2, &lbl, acc, 1);
                hits.add(Rect::new(x, y, kw, kh), KB_KEY0 + (r * 16 + col) as u32);
            }
        }

        // action row: CANCEL | SPACE | page | SUBMIT
        let submit_dim = self.min_len > 0 && self.buf.len() < self.min_len;
        let labels = ["CANCEL", "SPACE", PAGE_NAMES[(self.page + 1) % 3], self.submit_label];
        let accs = [RED, CYAN, AMBER, if submit_dim { TEXT_DIM } else { GREEN }];
        let ids = [KB_CANCEL, KB_SPACE, KB_PAGE, KB_SUBMIT];
        for i in 0..4 {
            let r = self.act_rect(c, i);
            c.fill(r.x, r.y, r.w, r.h, mix(BG, accs[i], 16));
            c.neonbox(r.x, r.y, r.w, r.h, accs[i]);
            c.corners(r.x, r.y, r.w, r.h, accs[i], 8);
            let lx = r.x + r.w / 2 - labels[i].len() as i32 * fw / 2;
            if i == 3 {
                c.textg(lx, r.y + r.h / 2 - fh / 2, labels[i], accs[i], 1);
            } else {
                c.text(lx, r.y + r.h / 2 - fh / 2, labels[i], accs[i], 1);
            }
            hits.add(r, ids[i]);
        }
    }

    /// Feed a HitId; only KB_* ids are meaningful (the modal owns all taps).
    pub fn on_tap(&mut self, id: HitId) -> PromptResult {
        match id {
            KB_X | KB_CANCEL => PromptResult::Cancelled,
            KB_SPACE => {
                if self.buf.len() < 63 {
                    self.buf.push(' ');
                }
                PromptResult::Open
            }
            KB_PAGE => {
                self.page = (self.page + 1) % 3;
                PromptResult::Open
            }
            KB_SUBMIT => {
                if self.min_len > 0 && self.buf.len() < self.min_len {
                    PromptResult::Open
                } else {
                    PromptResult::Submitted(self.buf.clone())
                }
            }
            id if (KB_KEY0..KB_KEY0 + 0x100).contains(&id) => {
                let (r, col) = (((id - KB_KEY0) / 16) as usize, ((id - KB_KEY0) % 16) as usize);
                if let Some(ch) = PAGES[self.page].get(r).and_then(|row| row.bytes().nth(col)) {
                    match ch {
                        0x01 => self.page = (self.page + 1) % 3,
                        0x08 => {
                            self.buf.pop();
                        }
                        ch => {
                            if self.buf.len() < 63 {
                                self.buf.push(ch as char);
                            }
                        }
                    }
                }
                PromptResult::Open
            }
            _ => PromptResult::Open, // background tap: swallow, stay open
        }
    }
}
