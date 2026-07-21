//! SONGS app — strudel song player. Live-evaluates .strudel files from the SD
//! (Agency OST et al) and synthesizes them on-device via neon-songs. The
//! now-playing panel shows the song's own header comment block — the tracks
//! document their form and intent in-source.

use std::sync::atomic::Ordering;

use neon_gfx::canvas::{mix, Canvas};
use neon_gfx::geom::Rect;
use neon_gfx::theme::*;

use super::{App, Ctx, HitId, HitMap};
use crate::songs::SongPlayer;

const SONGS_DIR: &str = "/mnt/sd/linux-lab/songs";
const HIT_TRACK0: HitId = 0xA100; // ..+tracks
const HIT_VOL0: HitId = 0xA020; // ..+VOL_SEGS-1
const VOL_SEGS: u32 = 12;
const MAX_DESC: usize = 14;

// ---- visualizer: 1024-pt FFT -> 16 log bands, scope, VU ----
const FFT_N: usize = 1024;
const BANDS: usize = 16;

struct Fft {
    rev: Vec<usize>,
    window: Vec<f32>,
    re: Vec<f32>,
    im: Vec<f32>,
}

impl Fft {
    fn new() -> Fft {
        let bits = FFT_N.trailing_zeros();
        let rev = (0..FFT_N).map(|i| i.reverse_bits() >> (usize::BITS - bits)).collect();
        let window = (0..FFT_N)
            .map(|i| {
                0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (FFT_N - 1) as f32).cos()
            })
            .collect();
        Fft { rev, window, re: vec![0.0; FFT_N], im: vec![0.0; FFT_N] }
    }

    /// Windowed magnitude spectrum of `input[..FFT_N]` into 16 log-spaced
    /// bands, 0..1. ~50k mults — microseconds, even on the A7.
    fn bands(&mut self, input: &[f32], out: &mut [f32; BANDS]) {
        for i in 0..FFT_N {
            self.re[self.rev[i]] = input.get(i).copied().unwrap_or(0.0) * self.window[i];
        }
        self.im.fill(0.0);
        let mut len = 2;
        while len <= FFT_N {
            let ang = -2.0 * std::f32::consts::PI / len as f32;
            let (wr, wi) = (ang.cos(), ang.sin());
            let mut start = 0;
            while start < FFT_N {
                let (mut cr, mut ci) = (1.0f32, 0.0f32);
                for k in 0..len / 2 {
                    let (ar, ai) = (self.re[start + k], self.im[start + k]);
                    let (br, bi) = (self.re[start + k + len / 2], self.im[start + k + len / 2]);
                    let (tr, ti) = (br * cr - bi * ci, br * ci + bi * cr);
                    self.re[start + k] = ar + tr;
                    self.im[start + k] = ai + ti;
                    self.re[start + k + len / 2] = ar - tr;
                    self.im[start + k + len / 2] = ai - ti;
                    let ncr = cr * wr - ci * wi;
                    ci = cr * wi + ci * wr;
                    cr = ncr;
                }
                start += len;
            }
            len <<= 1;
        }
        // log-spaced band edges: bin 1 (~43 Hz) .. bin 400 (~17 kHz)
        for b in 0..BANDS {
            let lo = (400f32.powf(b as f32 / BANDS as f32)) as usize;
            let hi = ((400f32.powf((b + 1) as f32 / BANDS as f32)) as usize).max(lo + 1);
            let mut m = 0f32;
            for bin in lo..hi.min(FFT_N / 2) {
                m = m.max(self.re[bin].hypot(self.im[bin]));
            }
            // full-scale sine under Hann ≈ N/4; -54 dB floor
            let db = 20.0 * (m / (FFT_N as f32 / 4.0) + 1e-9).log10();
            out[b] = ((db + 54.0) / 54.0).clamp(0.0, 1.0);
        }
    }
}

struct TrackMeta {
    path: String,
    title: String,
    desc: Vec<String>, // header comment block, ASCII-sanitized
}

/// Bitmap font is ASCII: swap the typographic chars the songs use, drop the rest.
fn ascii(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\u{2014}' | '\u{2013}' => out.push('-'), // em/en dash
            '\u{00b7}' | '\u{2022}' => out.push('.'), // middle dot / bullet
            '\u{2192}' => out.push_str("->"),
            c if c.is_ascii() => out.push(c),
            _ => {}
        }
    }
    out
}

fn scan_tracks() -> Vec<TrackMeta> {
    let mut paths: Vec<String> = std::fs::read_dir(SONGS_DIR)
        .map(|rd| {
            rd.filter_map(|e| {
                let p = e.ok()?.path();
                let s = p.to_str()?;
                s.ends_with(".strudel").then(|| s.to_string())
            })
            .collect()
        })
        .unwrap_or_default();
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let src = std::fs::read_to_string(&path).unwrap_or_default();
            let mut lines = src.lines();
            let title = lines
                .next()
                .and_then(|l| l.strip_prefix("//"))
                .map(|l| ascii(l.trim()))
                .filter(|l| !l.is_empty())
                .unwrap_or_else(|| {
                    path.rsplit('/').next().unwrap_or(&path).trim_end_matches(".strudel").into()
                });
            let desc: Vec<String> = lines
                .take_while(|l| l.starts_with("//"))
                .map(|l| ascii(l.trim_start_matches('/').strip_prefix(' ').unwrap_or_else(|| l.trim_start_matches('/'))))
                .take(MAX_DESC)
                .collect();
            TrackMeta { path, title, desc }
        })
        .collect()
}

pub struct SongsApp {
    tracks: Vec<TrackMeta>,
    playing: Option<usize>,
    player: Option<SongPlayer>,
    vol: u32,
    // mirrored from the player each tick
    pos_ds: u32,
    cycle_c: u32,
    load_pct: u32,
    error: Option<String>,
    // visualizer state
    fft: Fft,
    scope: Vec<f32>,
    spectrum: [f32; BANDS],
    vu: [f32; 2],
}

impl SongsApp {
    pub fn new() -> SongsApp {
        SongsApp {
            tracks: scan_tracks(),
            playing: None,
            player: None,
            vol: 208,
            pos_ds: 0,
            cycle_c: 0,
            load_pct: 0,
            error: None,
            fft: Fft::new(),
            scope: Vec::new(),
            spectrum: [0.0; BANDS],
            vu: [0.0; 2],
        }
    }

    fn stop(&mut self) {
        self.player = None; // Drop stops thread, closes pcm, amp off
        self.playing = None;
        self.pos_ds = 0;
        self.cycle_c = 0;
        self.load_pct = 0;
    }

    fn play(&mut self, idx: usize) {
        self.stop();
        self.error = None;
        let Some(t) = self.tracks.get(idx) else { return };
        match std::fs::read_to_string(&t.path) {
            Ok(src) => {
                self.player = Some(SongPlayer::start(src, self.vol));
                self.playing = Some(idx);
            }
            Err(e) => self.error = Some(format!("read: {e}")),
        }
    }
}

impl App for SongsApp {
    fn title(&self) -> &'static str {
        "SONGS"
    }

    fn accent(&self) -> u32 {
        MAGENTA
    }

    fn tick_ms(&self) -> u64 {
        if self.player.is_some() {
            80 // ~12 fps for the visualizer
        } else {
            1000
        }
    }

    fn tick(&mut self, _ctx: &mut Ctx) {
        let mut err = None;
        if let Some(p) = &self.player {
            self.pos_ds = p.state.pos_ds.load(Ordering::Relaxed);
            self.cycle_c = p.state.cycle_c.load(Ordering::Relaxed);
            self.load_pct = p.state.load_pct.load(Ordering::Relaxed);
            err = p.state.error.lock().unwrap().take();
            // visualizer inputs: copy the scope block, fold in peaks
            {
                let s = p.state.scope.lock().unwrap();
                self.scope.clear();
                self.scope.extend_from_slice(&s);
            }
            let pl = p.state.peak_l.load(Ordering::Relaxed) as f32 / 1000.0;
            let pr = p.state.peak_r.load(Ordering::Relaxed) as f32 / 1000.0;
            self.vu[0] = pl.max(self.vu[0] * 0.78);
            self.vu[1] = pr.max(self.vu[1] * 0.78);
            if self.scope.len() >= FFT_N {
                let mut bands = [0.0; BANDS];
                let scope = std::mem::take(&mut self.scope);
                self.fft.bands(&scope, &mut bands);
                self.scope = scope;
                for b in 0..BANDS {
                    self.spectrum[b] = bands[b].max(self.spectrum[b] * 0.80);
                }
            }
        }
        if let Some(e) = err {
            self.error = Some(e);
            self.stop();
        }
    }

    fn draw(&mut self, c: &mut Canvas, area: Rect, hits: &mut HitMap, _ctx: &Ctx) {
        c.panel(area.x, area.y + 8, area.w, area.h - 16, MAGENTA, "AGENCY OST // LIVE SYNTH");

        let tx = area.x + 24;
        let ty = area.y + 38;
        let list_w = (area.w * 2 / 5).clamp(280, 380);

        // track list
        if self.tracks.is_empty() {
            c.text(tx, ty, "NO SONGS ON SD", TEXT_DIM, 1);
            c.text(tx, ty + 22, SONGS_DIR, TEXT_DIM, 1);
        }
        let row_h = 30;
        for (i, t) in self.tracks.iter().enumerate() {
            let y = ty + i as i32 * row_h;
            if y + row_h > area.y + area.h - 20 {
                break;
            }
            let r = Rect::new(tx, y, list_w, row_h - 4);
            let on = self.playing == Some(i);
            if on {
                c.fill(r.x, r.y, r.w, r.h, mix(BG, MAGENTA, 45));
                c.neonbox(r.x, r.y, r.w, r.h, MAGENTA);
            }
            let marker = if on { ">" } else { " " };
            c.text(r.x + 6, r.y + 5, marker, MAGENTA, 1);
            // list shows the short name: strip the "AGENCY OST - " prefix
            let name = t.title.strip_prefix("AGENCY OST - ").unwrap_or(&t.title);
            let name: String = name.chars().take(30).collect();
            c.text(r.x + 22, r.y + 5, &name, if on { MAGENTA } else { TEXT2 }, 1);
            hits.add(r, HIT_TRACK0 + i as u32);
        }

        // now-playing panel
        let px = tx + list_w + 20;
        let pw = area.x + area.w - 24 - px;
        if pw < 120 {
            return;
        }
        let mut y = ty;
        match self.playing {
            Some(i) => {
                let t = &self.tracks[i];
                let name = t.title.strip_prefix("AGENCY OST - ").unwrap_or(&t.title);
                c.textg(px, y, &name.chars().take(34).collect::<String>(), MAGENTA, 1);
                y += 26;
                let secs = self.pos_ds / 10;
                let stat = format!(
                    "{}:{:02}  CYC {:.1}  DSP {:2}%",
                    secs / 60,
                    secs % 60,
                    self.cycle_c as f32 / 100.0,
                    self.load_pct
                );
                c.text(px, y, &stat, GREEN, 1);
                y += 26;
            }
            None => {
                c.text(px, y, "TAP A TRACK", TEXT_DIM, 1);
                y += 26;
                if let Some(e) = &self.error {
                    c.text(px, y, &ascii(e).chars().take(40).collect::<String>(), RED, 1);
                    y += 26;
                }
            }
        }

        // volume segments
        c.text(px, y + 5, "VOL", TEXT2, 1);
        let seg_w = ((pw - 50) / VOL_SEGS as i32).clamp(10, 22);
        for k in 0..VOL_SEGS {
            let r = Rect::new(px + 44 + k as i32 * seg_w, y, seg_w - 3, 24);
            let lit = self.vol * VOL_SEGS > k * 256;
            let col = if lit { mix(BG, CYAN, 60 + k as i32 * 10) } else { mix(BG, BG2, 220) };
            c.fill(r.x, r.y, r.w, r.h, col);
            c.neonbox(r.x, r.y, r.w, r.h, if lit { CYAN } else { mix(BG, BORDER, 120) });
            hits.add(r, HIT_VOL0 + k);
        }
        y += 34;

        // ---- visualizer (only while playing) ----
        if self.playing.is_some() {
            // 16-band spectrum
            let spec_h = 64;
            let bw = (pw / BANDS as i32).clamp(8, 26);
            for (b, v) in self.spectrum.iter().enumerate() {
                let bh = ((spec_h - 2) as f32 * v) as i32;
                let x = px + b as i32 * bw;
                let heat = (*v * 100.0) as i32;
                let col = mix(MAGENTA, CYAN, 100 - heat);
                c.fill(x, y + spec_h - bh, bw - 2, bh.max(1), mix(BG, col, 40 + heat * 2));
                // peak cap line
                c.hline(x, y + spec_h - bh - 1, bw - 2, col);
            }
            y += spec_h + 8;

            // oscilloscope: per-column min/max of the last block
            let scope_h = 44;
            let mid = y + scope_h / 2;
            c.hline(px, mid, pw - 8, mix(BG, GREEN, 60));
            if !self.scope.is_empty() {
                let cols = (pw - 8).max(1) as usize;
                let n = self.scope.len();
                for cx in 0..cols {
                    let s0 = cx * n / cols;
                    let s1 = ((cx + 1) * n / cols).max(s0 + 1);
                    let (mut lo, mut hi) = (0f32, 0f32);
                    for s in &self.scope[s0..s1.min(n)] {
                        lo = lo.min(*s);
                        hi = hi.max(*s);
                    }
                    let ytop = mid - (hi * (scope_h / 2 - 1) as f32) as i32;
                    let ybot = mid - (lo * (scope_h / 2 - 1) as f32) as i32;
                    c.vline(px + cx as i32, ytop, (ybot - ytop).max(1), GREEN);
                }
            }
            y += scope_h + 8;

            // stereo VU
            for (ch, lbl) in ["L", "R"].iter().enumerate() {
                c.text(px, y - 3, lbl, TEXT_DIM, 1);
                let vw = ((pw - 30) as f32 * self.vu[ch].min(1.0)) as i32;
                let col = if self.vu[ch] > 0.9 { RED } else { CYAN };
                c.fill(px + 22, y, vw.max(1), 8, mix(BG, col, 150));
                c.neonbox(px + 22, y, pw - 30, 8, mix(BG, BORDER, 140));
                y += 14;
            }
            y += 8;
        }

        // the song's own header comment block — its liner notes
        if let Some(i) = self.playing {
            let max_ch = (pw / 11).max(20) as usize;
            for line in &self.tracks[i].desc {
                if y + 18 > area.y + area.h - 16 {
                    break;
                }
                c.text(px, y, &line.chars().take(max_ch).collect::<String>(), TEXT_DIM, 1);
                y += 18;
            }
        }
    }

    fn on_tap(&mut self, id: HitId, _ctx: &mut Ctx) -> bool {
        match id {
            id if (HIT_VOL0..HIT_VOL0 + VOL_SEGS).contains(&id) => {
                let k = id - HIT_VOL0;
                let v = (k + 1) * 256 / VOL_SEGS;
                self.vol = if k == 0 && self.vol <= v { 0 } else { v };
                if let Some(p) = &self.player {
                    p.state.volume.store(self.vol, Ordering::Relaxed);
                }
                true
            }
            id if (HIT_TRACK0..HIT_TRACK0 + self.tracks.len() as u32).contains(&id) => {
                let i = (id - HIT_TRACK0) as usize;
                if self.playing == Some(i) {
                    self.stop();
                } else {
                    self.play(i);
                }
                true
            }
            _ => false,
        }
    }
}
