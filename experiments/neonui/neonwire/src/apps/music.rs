//! MUSIC app — step sequencer with REAL audio: the B2 PCM engine renders drum
//! voices straight to /dev/snd/pcmC0D5p on its own thread. Strudel pattern
//! eval layers on in B3.

use std::sync::atomic::Ordering;

use neon_gfx::canvas::{mix, Canvas};
use neon_gfx::geom::Rect;
use neon_gfx::theme::*;

use super::{App, ControlResult, Ctx, HitId, HitMap};
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
const HIT_VOL0: HitId = 0x9020; // ..+VOL_SEGS-1
const HIT_SAVE_MODE: HitId = 0x9030;
const HIT_SLOT0: HitId = 0x9031; // ..+SLOTS-1
// step cells: id = track * 16 + step

const VOL_SEGS: u32 = 12;
const SLOTS: usize = 3;
/// Persisted on the SD next to the other lab state; survives reboots.
const AUTO_PATH: &str = "/mnt/sd/linux-lab/music-state.json";
fn slot_path(i: usize) -> String {
    format!("/mnt/sd/linux-lab/music-slot{}.json", i + 1)
}

/// Everything worth keeping across app close / reboot.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct SavedState {
    bpm: u32,
    vol: u32,
    preset: usize,
    grid: [[bool; STEPS]; TRACKS],
}

fn load_state(path: &str) -> Option<SavedState> {
    let raw = std::fs::read(path).ok()?;
    serde_json::from_slice(&raw).ok()
}

pub struct MusicApp {
    grid: [[bool; STEPS]; TRACKS],
    playing: bool,
    bpm: u32,
    vol: u32, // 0..=256 master gain
    playhead: usize,
    preset: usize,
    save_armed: bool,
    /// Debounced auto-save: SD vfat writes can spike tens of ms, so edits
    /// mark dirty and tick()/on_leave() flush instead of writing per tap.
    dirty_at: Option<std::time::Instant>,
    engine: Option<Engine>,
}

impl MusicApp {
    pub fn new() -> MusicApp {
        let mut app = MusicApp {
            grid: [[false; STEPS]; TRACKS],
            playing: false,
            bpm: 120,
            vol: 208,
            playhead: 0,
            preset: 0,
            save_armed: false,
            dirty_at: None,
            engine: None,
        };
        if let Some(s) = load_state(AUTO_PATH) {
            app.apply(&s);
        } else {
            // seed a classic pattern so the app opens alive
            for s in (0..STEPS).step_by(4) {
                app.grid[0][s] = true;
            }
            app.grid[1][4] = true;
            app.grid[1][12] = true;
            for s in (2..STEPS).step_by(2) {
                app.grid[2][s] = true;
            }
        }
        app
    }

    fn snapshot(&self) -> SavedState {
        SavedState { bpm: self.bpm, vol: self.vol, preset: self.preset, grid: self.grid }
    }

    fn apply(&mut self, s: &SavedState) {
        self.grid = s.grid;
        self.bpm = s.bpm.clamp(40, 300);
        self.vol = s.vol.min(256);
        self.preset = s.preset.min(PRESETS.len() - 1);
    }

    /// Fire-and-forget persist; a failed SD write must never break the app.
    fn save(&self, path: &str) {
        if let Ok(bytes) = serde_json::to_vec(&self.snapshot()) {
            if let Err(e) = std::fs::write(path, bytes) {
                eprintln!("music: save {path}: {e}");
            }
        }
    }

    fn mark_dirty(&mut self) {
        self.dirty_at = Some(std::time::Instant::now());
    }

    fn flush_save(&mut self) {
        if self.dirty_at.take().is_some() {
            self.save(AUTO_PATH);
        }
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
            e.state.volume.store(self.vol, Ordering::Relaxed);
            e.state.playing.store(self.playing, Ordering::Relaxed);
        }
    }

    fn set_playing(&mut self, on: bool) {
        if on == self.playing {
            return;
        }
        self.playing = on;
        if self.playing {
            if self.engine.is_none() {
                self.engine = Some(Engine::start(self.spec(), self.bpm, self.vol));
            }
            speaker_amp(true);
        } else {
            self.playhead = 0;
            self.engine = None;
            speaker_amp(false);
        }
        self.sync_engine();
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
        // debounced auto-save: flush once edits settle
        if self.dirty_at.is_some_and(|t| t.elapsed().as_millis() >= 600) {
            self.flush_save();
        }
    }

    fn on_leave(&mut self) {
        self.flush_save();
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

        // volume + pattern slots row
        let vy = py + 38;
        c.text(tx, vy + 6, "VOL", TEXT2, 1);
        let seg_w = 20;
        for k in 0..VOL_SEGS {
            let r = Rect::new(tx + 44 + k as i32 * seg_w, vy, seg_w - 3, 26);
            let lit = self.vol * VOL_SEGS > k * 256;
            let col = if lit { mix(BG, CYAN, 60 + k as i32 * 10) } else { mix(BG, BG2, 220) };
            c.fill(r.x, r.y, r.w, r.h, col);
            c.neonbox(r.x, r.y, r.w, r.h, if lit { CYAN } else { mix(BG, BORDER, 120) });
            hits.add(r, HIT_VOL0 + k);
        }
        let sx = tx + 44 + VOL_SEGS as i32 * seg_w + 30;
        let save = Rect::new(sx, vy, 66, 26);
        let sc2 = if self.save_armed { RED } else { BORDER };
        if self.save_armed {
            c.fill(save.x, save.y, save.w, save.h, mix(BG, RED, 50));
        }
        c.neonbox(save.x, save.y, save.w, save.h, sc2);
        c.text(save.x + 11, save.y + 4, "SAVE", if self.save_armed { RED } else { TEXT2 }, 1);
        hits.add(save, HIT_SAVE_MODE);
        for i in 0..SLOTS {
            let r = Rect::new(sx + 76 + i as i32 * 36, vy, 30, 26);
            let exists = std::fs::metadata(slot_path(i)).is_ok();
            let col = if self.save_armed {
                RED
            } else if exists {
                AMBER
            } else {
                mix(BG, BORDER, 120)
            };
            c.neonbox(r.x, r.y, r.w, r.h, col);
            let tcol = if exists || self.save_armed { TEXT } else { TEXT_DIM };
            c.text(r.x + 10, r.y + 4, &format!("{}", i + 1), tcol, 1);
            hits.add(r, HIT_SLOT0 + i as u32);
        }

        // step grid
        let gx = area.x + 24;
        let gy = ty + 124;
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

    fn control(&mut self, op: &str, arg: &str, _ctx: &mut Ctx) -> ControlResult {
        let op = op.to_ascii_lowercase();
        match op.as_str() {
            "play" | "start" => {
                self.set_playing(true);
                ControlResult::Ok(format!("music playing bpm={} vol={}", self.bpm, self.vol))
            }
            "stop" => {
                self.set_playing(false);
                ControlResult::Ok("music stopped".into())
            }
            "toggle" => {
                self.set_playing(!self.playing);
                ControlResult::Ok(if self.playing {
                    "music playing".into()
                } else {
                    "music stopped".into()
                })
            }
            "bpm" => match arg.trim().parse::<u32>() {
                Ok(n) => {
                    self.bpm = n.clamp(40, 300);
                    self.sync_engine();
                    self.mark_dirty();
                    ControlResult::Ok(format!("music bpm={}", self.bpm))
                }
                Err(_) => ControlResult::Err("music bpm needs a number".into()),
            },
            "vol" | "volume" => match arg.trim().parse::<u32>() {
                Ok(n) => {
                    self.vol = n.min(256);
                    self.sync_engine();
                    self.mark_dirty();
                    ControlResult::Ok(format!("music vol={}", self.vol))
                }
                Err(_) => ControlResult::Err("music vol needs 0..=256".into()),
            },
            "preset" => {
                let a = arg.trim().to_ascii_lowercase();
                let idx = if let Ok(n) = a.parse::<usize>() {
                    n.min(PRESETS.len() - 1)
                } else {
                    PRESETS
                        .iter()
                        .position(|(n, _)| n.eq_ignore_ascii_case(&a))
                        .unwrap_or(usize::MAX)
                };
                if idx >= PRESETS.len() {
                    return ControlResult::Err(format!(
                        "presets: {}",
                        PRESETS.iter().map(|(n, _)| *n).collect::<Vec<_>>().join("|")
                    ));
                }
                self.preset = idx;
                self.sync_engine();
                self.mark_dirty();
                ControlResult::Ok(format!("music preset={}", PRESETS[idx].0))
            }
            "status" | "" => ControlResult::Ok(format!(
                "music playing={} bpm={} vol={} preset={}",
                self.playing,
                self.bpm,
                self.vol,
                PRESETS[self.preset].0
            )),
            _ => ControlResult::Unhandled,
        }
    }

    fn on_tap(&mut self, id: HitId, _ctx: &mut Ctx) -> bool {
        match id {
            HIT_PLAY => {
                self.set_playing(!self.playing);
                true
            }
            HIT_BPM_DN => {
                self.bpm = self.bpm.saturating_sub(5).max(40);
                self.sync_engine();
                self.mark_dirty();
                true
            }
            HIT_BPM_UP => {
                self.bpm = (self.bpm + 5).min(300);
                self.sync_engine();
                self.mark_dirty();
                true
            }
            id if (HIT_VOL0..HIT_VOL0 + VOL_SEGS).contains(&id) => {
                let k = id - HIT_VOL0;
                // tapping the lone lit first segment again = mute
                let v = (k + 1) * 256 / VOL_SEGS;
                self.vol = if k == 0 && self.vol <= v { 0 } else { v };
                self.sync_engine();
                self.mark_dirty();
                true
            }
            HIT_SAVE_MODE => {
                self.save_armed = !self.save_armed;
                true
            }
            id if (HIT_SLOT0..HIT_SLOT0 + SLOTS as u32).contains(&id) => {
                let i = (id - HIT_SLOT0) as usize;
                if self.save_armed {
                    self.save(&slot_path(i));
                    self.save_armed = false;
                } else if let Some(s) = load_state(&slot_path(i)) {
                    self.apply(&s);
                    self.sync_engine();
                    self.mark_dirty();
                }
                true
            }
            id if (HIT_PRESET0..HIT_PRESET0 + PRESETS.len() as u32).contains(&id) => {
                self.preset = (id - HIT_PRESET0) as usize;
                self.sync_engine();
                self.mark_dirty();
                true
            }
            id if (id as usize) < TRACKS * STEPS => {
                let (t, s) = (id as usize / STEPS, id as usize % STEPS);
                self.grid[t][s] = !self.grid[t][s];
                self.preset = 0; // editing the grid selects GRID mode
                self.sync_engine();
                self.mark_dirty();
                true
            }
            _ => false,
        }
    }
}
