//! SONGS app — strudel song player. Live-evaluates .strudel files from the SD
//! (Agency OST et al) and synthesizes them on-device via neon-songs.
//!
//! Library is foldered: each subdir of /mnt/sd/linux-lab/songs is an
//! accordion section (exactly one open, so the list always fits the panel).
//! The now-playing panel runs an event rain (pattern triggers falling by
//! pitch, colored by sound), a 16-band spectrum, oscilloscope and VU — all
//! fed from the audio thread's taps, ~12 fps, negligible next to the DSP.

use std::sync::atomic::Ordering;

use neon_gfx::canvas::{mix, Canvas};
use neon_gfx::geom::Rect;
use neon_gfx::theme::*;

use super::{App, Ctx, HitId, HitMap};
use crate::songs::SongPlayer;

const SONGS_DIR: &str = "/mnt/sd/linux-lab/songs";
const HIT_TRACK0: HitId = 0xA100; // + folder*64 + track
const HIT_FOLDER0: HitId = 0xA300; // ..+folders
const HIT_VOL0: HitId = 0xA020; // ..+VOL_SEGS-1
const VOL_SEGS: u32 = 12;
const MAX_DESC: usize = 14;

// ---- visualizer: 1024-pt FFT -> 16 log bands, event rain, scope, VU ----
const FFT_N: usize = 1024;
const BANDS: usize = 16;
const RAIN_FALL_SECS: f32 = 1.5;
const RAIN_COLORS: [u32; 6] = [CYAN, MAGENTA, GREEN, AMBER, GOLD, BLUE];

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

/// One falling event in the rain view.
struct Drop {
    t: f32, // fire time, renderer seconds
    x01: f32,
    gain: f32,
    color: u32,
}

struct TrackMeta {
    path: String,
    title: String,
    desc: Vec<String>, // header comment block, ASCII-sanitized
}

struct Folder {
    name: String,
    tracks: Vec<TrackMeta>,
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

fn read_track(path: String) -> TrackMeta {
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
}

fn strudels_in(dir: &str) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| {
                let p = e.ok()?.path();
                let s = p.to_str()?;
                (p.is_file() && s.ends_with(".strudel")).then(|| s.to_string())
            })
            .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

fn scan_folders() -> Vec<Folder> {
    let mut folders = Vec::new();
    let mut subdirs: Vec<String> = std::fs::read_dir(SONGS_DIR)
        .map(|rd| {
            rd.filter_map(|e| {
                let p = e.ok()?.path();
                p.is_dir().then(|| p.to_str().map(String::from))?
            })
            .collect()
        })
        .unwrap_or_default();
    subdirs.sort();
    for d in subdirs {
        let tracks: Vec<TrackMeta> = strudels_in(&d).into_iter().map(read_track).collect();
        if !tracks.is_empty() {
            let name = d.rsplit('/').next().unwrap_or(&d).to_ascii_uppercase();
            folders.push(Folder { name, tracks });
        }
    }
    // loose files in the root form a trailing MISC section
    let loose: Vec<TrackMeta> = strudels_in(SONGS_DIR).into_iter().map(read_track).collect();
    if !loose.is_empty() {
        folders.push(Folder { name: "MISC".into(), tracks: loose });
    }
    folders
}

pub struct SongsApp {
    folders: Vec<Folder>,
    /// Library scan is deferred to first on_enter: it reads every song file
    /// off the SD, and doing that in new() slowed boot-to-face for everyone.
    scanned: bool,
    open: usize,
    playing: Option<(usize, usize)>,
    player: Option<SongPlayer>,
    vol: u32,
    // mirrored from the player each tick
    pos_ds: u32,
    cycle_c: u32,
    load_pct: u32,
    stage: u8,
    anim: u32, // tick counter for loading animation
    error: Option<String>,
    // visualizer state
    fft: Fft,
    scope: Vec<f32>,
    spectrum: [f32; BANDS],
    vu: [f32; 2],
    drops: Vec<Drop>,
}

impl SongsApp {
    pub fn new() -> SongsApp {
        SongsApp {
            folders: Vec::new(),
            scanned: false,
            open: 0,
            playing: None,
            player: None,
            vol: 208,
            pos_ds: 0,
            cycle_c: 0,
            load_pct: 0,
            stage: 0,
            anim: 0,
            error: None,
            fft: Fft::new(),
            scope: Vec::new(),
            spectrum: [0.0; BANDS],
            vu: [0.0; 2],
            drops: Vec::new(),
        }
    }

    fn stop(&mut self) {
        self.player = None; // Drop stops thread, closes pcm, amp off
        self.playing = None;
        self.pos_ds = 0;
        self.cycle_c = 0;
        self.load_pct = 0;
        self.drops.clear();
    }

    fn play(&mut self, f: usize, t: usize) {
        self.stop();
        self.error = None;
        let Some(track) = self.folders.get(f).and_then(|fo| fo.tracks.get(t)) else { return };
        match std::fs::read_to_string(&track.path) {
            Ok(src) => {
                self.player = Some(SongPlayer::start(src, self.vol));
                self.playing = Some((f, t));
            }
            Err(e) => self.error = Some(format!("read: {e}")),
        }
    }

    fn now_secs(&self) -> f32 {
        self.pos_ds as f32 / 10.0
    }
}

impl App for SongsApp {
    fn title(&self) -> &'static str {
        "SONGS"
    }

    fn accent(&self) -> u32 {
        MAGENTA
    }

    fn on_enter(&mut self) {
        if !self.scanned {
            self.folders = scan_folders();
            self.scanned = true;
            // pre-warm the sample-bank cache off-thread while the user is
            // still looking at the track list — first play starts instantly
            std::thread::spawn(|| neon_songs::sdbank::warm_all(crate::songs::SAMPLES_DIR));
        }
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
        self.anim = self.anim.wrapping_add(1);
        if let Some(p) = &self.player {
            self.pos_ds = p.state.pos_ds.load(Ordering::Relaxed);
            self.cycle_c = p.state.cycle_c.load(Ordering::Relaxed);
            self.load_pct = p.state.load_pct.load(Ordering::Relaxed);
            self.stage = p.state.stage.load(Ordering::Relaxed);
            err = p.state.error.lock().unwrap().take();
            // visualizer inputs
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
            // event rain intake
            let events = std::mem::take(&mut *p.state.events.lock().unwrap());
            for e in events {
                // pitched events spread by note (24..96); unpitched by tag hash
                let x01 = if e.note.is_finite() && e.note > 0.0 {
                    ((e.note - 24.0) / 72.0).clamp(0.0, 1.0)
                } else {
                    (e.tag % 97) as f32 / 97.0
                };
                self.drops.push(Drop {
                    t: e.time as f32,
                    x01,
                    gain: e.gain.clamp(0.1, 1.2),
                    color: RAIN_COLORS[(e.tag % RAIN_COLORS.len() as u32) as usize],
                });
            }
            let now = self.now_secs();
            self.drops.retain(|d| now - d.t < RAIN_FALL_SECS && d.t - now < 2.0);
            if self.drops.len() > 200 {
                let excess = self.drops.len() - 200;
                self.drops.drain(..excess);
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
        let list_bot = area.y + area.h - 20;

        // foldered track list — accordion: exactly one folder open
        if self.folders.is_empty() {
            c.text(tx, ty, "NO SONGS ON SD", TEXT_DIM, 1);
            c.text(tx, ty + 22, SONGS_DIR, TEXT_DIM, 1);
        }
        let row_h = 28;
        let mut y = ty;
        for (fi, folder) in self.folders.iter().enumerate() {
            if y + row_h > list_bot {
                break;
            }
            let fr = Rect::new(tx, y, list_w, row_h - 4);
            let open = fi == self.open;
            let mark = if open { "v" } else { ">" };
            c.fill(fr.x, fr.y, fr.w, fr.h, mix(BG, BG2, 160));
            c.neonbox(fr.x, fr.y, fr.w, fr.h, if open { mix(BG, MAGENTA, 140) } else { mix(BG, BORDER, 140) });
            c.text(fr.x + 8, fr.y + 4, mark, MAGENTA, 1);
            let label = format!("{} ({})", folder.name, folder.tracks.len());
            c.text(fr.x + 26, fr.y + 4, &label.chars().take(26).collect::<String>(), if open { MAGENTA } else { TEXT2 }, 1);
            hits.add(fr, HIT_FOLDER0 + fi as u32);
            y += row_h;
            if !open {
                continue;
            }
            for (ti, t) in folder.tracks.iter().enumerate() {
                if y + row_h > list_bot {
                    break;
                }
                let r = Rect::new(tx + 10, y, list_w - 10, row_h - 4);
                let on = self.playing == Some((fi, ti));
                if on {
                    c.fill(r.x, r.y, r.w, r.h, mix(BG, MAGENTA, 45));
                    c.neonbox(r.x, r.y, r.w, r.h, MAGENTA);
                }
                c.text(r.x + 6, r.y + 4, if on { ">" } else { " " }, MAGENTA, 1);
                let name = t.title.strip_prefix("AGENCY OST - ").unwrap_or(&t.title);
                let name: String = name.chars().take(28).collect();
                c.text(r.x + 20, r.y + 4, &name, if on { MAGENTA } else { TEXT2 }, 1);
                hits.add(r, HIT_TRACK0 + (fi * 64 + ti) as u32);
                y += row_h;
            }
        }

        // now-playing panel
        let px = tx + list_w + 20;
        let pw = area.x + area.w - 24 - px;
        if pw < 120 {
            return;
        }
        let mut y = ty;
        match self.playing {
            Some((f, t)) => {
                let tr = &self.folders[f].tracks[t];
                let name = tr.title.strip_prefix("AGENCY OST - ").unwrap_or(&tr.title);
                c.textg(px, y, &name.chars().take(34).collect::<String>(), MAGENTA, 1);
                y += 26;
                if self.stage < crate::songs::STAGE_PLAYING {
                    // still starting up — say what and animate
                    let what = match self.stage {
                        crate::songs::STAGE_SAMPLES => "LOADING SAMPLES",
                        _ => "EVALUATING PATTERN",
                    };
                    let dots = ".".repeat((self.anim as usize % 4) + 1);
                    c.text(px, y, &format!("{what}{dots}"), AMBER, 1);
                    // sweeping activity bar
                    let bw = (pw - 8).max(40);
                    let seg = 46;
                    let pos = (self.anim as i32 * 9) % (bw + seg) - seg; // -seg..bw
                    let x0 = pos.max(0);
                    let x1 = (pos + seg).min(bw);
                    c.neonbox(px, y + 22, bw, 6, mix(BG, AMBER, 90));
                    if x1 > x0 {
                        c.fill(px + x0, y + 23, x1 - x0, 4, mix(BG, AMBER, 170));
                    }
                    y += 34;
                } else {
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
            // EVENT RAIN — triggers fall by pitch, colored by sound
            let rain_h = 96;
            let rw = pw - 8;
            c.neonbox(px, y, rw, rain_h, mix(BG, MAGENTA, 60));
            let now = self.now_secs();
            for d in &self.drops {
                let age = now - d.t;
                if !(0.0..RAIN_FALL_SECS).contains(&age) {
                    continue;
                }
                let fy = y + 2 + (age / RAIN_FALL_SECS * (rain_h - 6) as f32) as i32;
                let fx = px + 3 + (d.x01 * (rw - 8) as f32) as i32;
                let bright = ((1.0 - age / RAIN_FALL_SECS) * d.gain.min(1.0) * 100.0) as i32;
                // head + fading trail upward
                c.fill(fx, fy, 3, 3, mix(BG, WHITE, 120 + bright));
                c.vline(fx + 1, (fy - 10).max(y + 2), 10.min(fy - y - 2), mix(BG, d.color, 60 + bright * 2));
            }
            y += rain_h + 8;

            // 16-band spectrum
            let spec_h = 50;
            let bw = (pw / BANDS as i32).clamp(8, 26);
            for (b, v) in self.spectrum.iter().enumerate() {
                let bh = ((spec_h - 2) as f32 * v) as i32;
                let x = px + b as i32 * bw;
                let heat = (*v * 100.0) as i32;
                let col = mix(MAGENTA, CYAN, 100 - heat);
                c.fill(x, y + spec_h - bh, bw - 2, bh.max(1), mix(BG, col, 40 + heat * 2));
                c.hline(x, y + spec_h - bh - 1, bw - 2, col);
            }
            y += spec_h + 8;

            // oscilloscope: per-column min/max of the last block
            let scope_h = 36;
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
            y += 6;
        }

        // the song's own header comment block — its liner notes
        if let Some((f, t)) = self.playing {
            let max_ch = (pw / 11).max(20) as usize;
            for line in &self.folders[f].tracks[t].desc {
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
            id if (HIT_FOLDER0..HIT_FOLDER0 + self.folders.len() as u32).contains(&id) => {
                self.open = (id - HIT_FOLDER0) as usize;
                true
            }
            id if (HIT_TRACK0..HIT_TRACK0 + 64 * 16).contains(&id) => {
                let flat = (id - HIT_TRACK0) as usize;
                let (f, t) = (flat / 64, flat % 64);
                if self.playing == Some((f, t)) {
                    self.stop();
                } else {
                    self.play(f, t);
                }
                true
            }
            _ => false,
        }
    }
}
