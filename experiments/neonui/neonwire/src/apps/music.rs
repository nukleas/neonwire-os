//! MUSIC app — step sequencer with REAL audio: the B2 PCM engine renders drum
//! voices straight to /dev/snd/pcmC0D5p on its own thread. Strudel pattern
//! eval layers on in B3.

use std::sync::atomic::Ordering;

use neon_gfx::canvas::{mix, Canvas};
use neon_gfx::geom::Rect;
use neon_gfx::theme::*;

use super::{App, Ctx, HitId, HitMap};
use crate::audio::{speaker_amp, Engine, PatternSpec, STEPS, TRACKS};

/// Mini-notation presets (validated against strudel-mini's parser). GRID is
/// slot 0 and compiles the touch grid into a pattern via strudel combinators.
const PRESETS: [(&str, &str); 4] = [
    ("GRID", ""),
    ("HOUSE", "bd*4, [~ hh]*4, ~ sd ~ sd"),
    ("EUCLID", "bd(3,8), hh*8, ~ cp ~ [cp cp]"),
    ("BREAK", "bd sd [bd bd] sd, hh*16"),
];
const TRACK_NAMES: [&str; TRACKS] = ["BD", "SD", "HH", "CP"];
const TRACK_COLS: [u32; TRACKS] = [AMBER, MAGENTA, CYAN, GREEN];

const HIT_PLAY: HitId = 0x9000;
const HIT_BPM_DN: HitId = 0x9001;
const HIT_BPM_UP: HitId = 0x9002;
const HIT_PRESET0: HitId = 0x9010; // ..+3
// step cells: id = track * 16 + step

pub struct MusicApp {
    grid: [[bool; STEPS]; TRACKS],
    playing: bool,
    bpm: u32,
    playhead: usize,
    preset: usize,
    engine: Option<Engine>,
}

impl MusicApp {
    pub fn new() -> MusicApp {
        let mut grid = [[false; STEPS]; TRACKS];
        // seed a classic pattern so the app opens alive
        for s in (0..STEPS).step_by(4) {
            grid[0][s] = true;
        }
        grid[1][4] = true;
        grid[1][12] = true;
        for s in (2..STEPS).step_by(2) {
            grid[2][s] = true;
        }
        MusicApp { grid, playing: false, bpm: 120, playhead: 0, preset: 0, engine: None }
    }

    fn spec(&self) -> PatternSpec {
        if self.preset == 0 {
            PatternSpec::Grid(self.grid)
        } else {
            PatternSpec::Mini(PRESETS[self.preset].1.to_string())
        }
    }

    fn sync_engine(&mut self) {
        if let Some(e) = &self.engine {
            *e.state.spec.lock().unwrap() = self.spec();
            e.state.spec_gen.fetch_add(1, Ordering::Release);
            e.state.bpm.store(self.bpm, Ordering::Relaxed);
            e.state.playing.store(self.playing, Ordering::Relaxed);
        }
    }
}

impl App for MusicApp {
    fn title(&self) -> &'static str {
        "MUSIC"
    }

    fn accent(&self) -> u32 {
        AMBER
    }

    fn tick_ms(&self) -> u64 {
        if self.playing {
            // UI refresh only; audio timing lives on the engine thread
            (60_000 / self.bpm / 4).clamp(40, 250) as u64
        } else {
            1000
        }
    }

    fn tick(&mut self, _ctx: &mut Ctx) {
        // playhead is owned by the audio thread; UI just mirrors it
        if let Some(e) = &self.engine {
            self.playhead = e.state.playhead.load(Ordering::Relaxed);
        }
    }

    fn draw(&mut self, c: &mut Canvas, area: Rect, hits: &mut HitMap, _ctx: &Ctx) {
        c.panel(area.x, area.y + 8, area.w, area.h - 16, AMBER, "STRUDEL // SEQ");

        // transport
        let tx = area.x + 24;
        let ty = area.y + 34;
        let play = Rect::new(tx, ty, 110, 36);
        let (pl, pc) = if self.playing { ("STOP", RED) } else { ("PLAY", GREEN) };
        c.fill(play.x, play.y, play.w, play.h, mix(BG, pc, 30));
        c.neonbox(play.x, play.y, play.w, play.h, pc);
        c.textg(play.x + 24, play.y + 6, pl, pc, 1);
        hits.add(play, HIT_PLAY);

        let bd = Rect::new(tx + 130, ty, 36, 36);
        let bu = Rect::new(tx + 260, ty, 36, 36);
        c.neonbox(bd.x, bd.y, bd.w, bd.h, BORDER);
        c.text(bd.x + 13, bd.y + 6, "-", TEXT, 2);
        c.neonbox(bu.x, bu.y, bu.w, bu.h, BORDER);
        c.text(bu.x + 11, bu.y + 6, "+", TEXT, 2);
        hits.add(bd, HIT_BPM_DN);
        hits.add(bu, HIT_BPM_UP);
        let bpm = format!("{} BPM", self.bpm);
        c.text(tx + 178, ty + 10, &bpm, AMBER, 1);
        let (sink, sc) = match &self.engine {
            Some(e) if e.state.online.load(Ordering::Relaxed) => ("SINK hw:0,5 LIVE", GREEN),
            Some(_) => ("SINK OPENING...", AMBER),
            None => ("SINK IDLE", TEXT_DIM),
        };
        c.text(tx + 320, ty + 10, sink, sc, 1);

        // preset chips + active notation string
        let py = ty + 46;
        let mut px = tx;
        for (i, (name, _)) in PRESETS.iter().enumerate() {
            let w = name.len() as i32 * 11 + 26;
            let r = Rect::new(px, py, w, 28);
            let on = i == self.preset;
            let col = if on { AMBER } else { BORDER };
            if on {
                c.fill(r.x, r.y, r.w, r.h, mix(BG, AMBER, 40));
            }
            c.neonbox(r.x, r.y, r.w, r.h, col);
            c.text(r.x + 13, r.y + 4, name, if on { AMBER } else { TEXT2 }, 1);
            hits.add(r, HIT_PRESET0 + i as u32);
            px += w + 10;
        }
        let notation = if self.preset == 0 { "<touch grid>" } else { PRESETS[self.preset].1 };
        c.text(px + 14, py + 6, notation, GREEN, 1);

        // step grid
        let gx = area.x + 24;
        let gy = ty + 90;
        let gw = area.w - 60;
        let cell = (gw - 60) / STEPS as i32;
        let ch = ((area.h - (gy - area.y) - 30) / TRACKS as i32).min(56);
        for t in 0..TRACKS {
            let y = gy + t as i32 * ch;
            c.text(gx, y + ch / 2 - 10, TRACK_NAMES[t], TRACK_COLS[t], 1);
            for s in 0..STEPS {
                let x = gx + 40 + s as i32 * cell;
                let r = Rect::new(x, y, cell - 4, ch - 6);
                let on = self.grid[t][s];
                let is_ph = self.playing && s == self.playhead;
                let base = if on { mix(BG, TRACK_COLS[t], 150) } else { mix(BG, BG2, 220) };
                let col = if is_ph { mix(base, WHITE, 70) } else { base };
                c.fill(r.x, r.y, r.w, r.h, col);
                let edge = if is_ph {
                    WHITE
                } else if on {
                    TRACK_COLS[t]
                } else if s % 4 == 0 {
                    mix(BG, BORDER, 220)
                } else {
                    mix(BG, BORDER, 120)
                };
                c.neonbox(r.x, r.y, r.w, r.h, edge);
                hits.add(r, (t * STEPS + s) as HitId);
            }
        }
    }

    fn on_tap(&mut self, id: HitId, _ctx: &mut Ctx) -> bool {
        match id {
            HIT_PLAY => {
                self.playing = !self.playing;
                if self.playing {
                    if self.engine.is_none() {
                        self.engine = Some(Engine::start(self.spec(), self.bpm));
                    }
                    speaker_amp(true);
                } else {
                    self.playhead = 0;
                    speaker_amp(false);
                }
                self.sync_engine();
                true
            }
            HIT_BPM_DN => {
                self.bpm = self.bpm.saturating_sub(5).max(40);
                self.sync_engine();
                true
            }
            HIT_BPM_UP => {
                self.bpm = (self.bpm + 5).min(300);
                self.sync_engine();
                true
            }
            id if (HIT_PRESET0..HIT_PRESET0 + PRESETS.len() as u32).contains(&id) => {
                self.preset = (id - HIT_PRESET0) as usize;
                self.sync_engine();
                true
            }
            id if (id as usize) < TRACKS * STEPS => {
                let (t, s) = (id as usize / STEPS, id as usize % STEPS);
                self.grid[t][s] = !self.grid[t][s];
                self.preset = 0; // editing the grid selects GRID mode
                self.sync_engine();
                true
            }
            _ => false,
        }
    }
}
