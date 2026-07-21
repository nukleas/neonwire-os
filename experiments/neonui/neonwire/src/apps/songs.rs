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
            250
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
