//! Song playback engine: evaluates a .strudel file (neon-songs) and streams
//! the synth render to /dev/snd on its own thread. One SongPlayer = one song;
//! drop it to stop (frees hw:0,5 for the Music app's sequencer and vice versa).

use std::io;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use crate::audio::{speaker_amp, Pcm, PERIOD, RATE};

/// dirt-samples-layout WAV banks on the SD (bank dir -> s("<bank>") slots).
pub const SAMPLES_DIR: &str = "/mnt/sd/linux-lab/samples";

/// Load-stage values in `SongState::stage` (u8).
pub const STAGE_EVAL: u8 = 1;
pub const STAGE_SAMPLES: u8 = 2;
pub const STAGE_PLAYING: u8 = 3;

pub struct SongState {
    pub online: AtomicBool,   // pcm opened, rendering
    pub stage: AtomicU8,      // STAGE_* — what start-up is doing right now
    pub pos_ds: AtomicU32,    // playback position, deciseconds
    pub cycle_c: AtomicU32,   // pattern cycle × 100
    pub load_pct: AtomicU32,  // render cost as % of realtime, ~1 s window
    pub voices: AtomicU32,    // currently sounding DSP voices
    pub xruns: AtomicU32,     // recovered PCM underruns since start
    pub volume: AtomicU32,    // 0..=256, shared with UI
    pub peak_l: AtomicU32,    // block peak × 1000, pre-volume
    pub peak_r: AtomicU32,
    /// Mono downmix of the latest rendered block (PERIOD samples) for the
    /// visualizer. Swapped wholesale each block; UI copies under the lock.
    pub scope: Mutex<Vec<f32>>,
    /// Scheduled trigger events for the event-rain view; UI drains, capped.
    pub events: Mutex<Vec<neon_songs::VisEvent>>,
    pub error: Mutex<Option<String>>,
    quit: AtomicBool,
}

pub struct SongPlayer {
    pub state: Arc<SongState>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl SongPlayer {
    /// Start playing strudel source. Evaluation happens on the audio thread
    /// (measured ~5-10 ms on the A7); errors land in `state.error`.
    pub fn start(src: String, volume: u32) -> SongPlayer {
        let state = Arc::new(SongState {
            online: AtomicBool::new(false),
            stage: AtomicU8::new(STAGE_EVAL),
            pos_ds: AtomicU32::new(0),
            cycle_c: AtomicU32::new(0),
            load_pct: AtomicU32::new(0),
            voices: AtomicU32::new(0),
            xruns: AtomicU32::new(0),
            volume: AtomicU32::new(volume.min(256)),
            peak_l: AtomicU32::new(0),
            peak_r: AtomicU32::new(0),
            scope: Mutex::new(Vec::new()),
            events: Mutex::new(Vec::new()),
            error: Mutex::new(None),
            quit: AtomicBool::new(false),
        });
        let s = state.clone();
        let thread = std::thread::spawn(move || song_thread(&s, &src));
        SongPlayer { state, thread: Some(thread) }
    }
}

impl Drop for SongPlayer {
    fn drop(&mut self) {
        self.state.quit.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        speaker_amp(false);
    }
}

fn fail(state: &SongState, msg: String) {
    *state.error.lock().unwrap() = Some(msg);
}

fn song_thread(state: &SongState, src: &str) {
    let song = match neon_songs::eval_song(src) {
        Ok(s) => s,
        Err(e) => return fail(state, e),
    };
    let mut r = neon_songs::SongRenderer::new(song.bpm, RATE, 0.9);
    r.set_pattern(song.pattern);
    r.set_block_size(PERIOD);
    state.stage.store(STAGE_SAMPLES, Ordering::Relaxed);
    let banks = r.load_sd_banks(SAMPLES_DIR, 256.0);
    if !banks.is_empty() {
        eprintln!("songs: loaded SD banks: {banks:?}");
    }
    let pcm = match Pcm::open() {
        Ok(p) => p,
        Err(e) if e.kind() == io::ErrorKind::ResourceBusy => {
            return fail(state, "SINK BUSY — stop the MUSIC seq first".into())
        }
        Err(e) => return fail(state, format!("pcm: {e}")),
    };
    speaker_amp(true);
    state.stage.store(STAGE_PLAYING, Ordering::Relaxed);
    state.online.store(true, Ordering::Relaxed);

    r.begin();
    let mut ibuf = vec![0i16; PERIOD * 2];
    let mut mono = vec![0f32; PERIOD];
    let mut render_ns: u64 = 0;
    let mut audio_ns: u64 = 0;
    while !state.quit.load(Ordering::Relaxed) {
        let vol = state.volume.load(Ordering::Relaxed).min(256) as f32 / 256.0;
        let gain = 28000.0 * vol;
        let tb = std::time::Instant::now();
        let (l, rr) = r.next_block();
        let (mut pl, mut pr) = (0f32, 0f32);
        for i in 0..PERIOD {
            ibuf[i * 2] = (l[i].clamp(-1.0, 1.0) * gain) as i16;
            ibuf[i * 2 + 1] = (rr[i].clamp(-1.0, 1.0) * gain) as i16;
            pl = pl.max(l[i].abs());
            pr = pr.max(rr[i].abs());
            mono[i] = (l[i] + rr[i]) * 0.5;
        }
        state.peak_l.store((pl.min(1.0) * 1000.0) as u32, Ordering::Relaxed);
        state.peak_r.store((pr.min(1.0) * 1000.0) as u32, Ordering::Relaxed);
        {
            let mut s = state.scope.lock().unwrap();
            if s.len() != PERIOD {
                s.resize(PERIOD, 0.0);
            }
            std::mem::swap::<Vec<f32>>(&mut s, &mut mono);
        }
        let ev = r.take_events();
        if !ev.is_empty() {
            let mut q = state.events.lock().unwrap();
            q.extend(ev);
            let len = q.len();
            if len > 256 {
                q.drain(..len - 256);
            }
        }
        if mono.len() != PERIOD {
            mono.resize(PERIOD, 0.0);
        }
        render_ns += tb.elapsed().as_nanos() as u64;
        audio_ns += PERIOD as u64 * 1_000_000_000 / RATE as u64;
        let _ = pcm.write(&ibuf);

        state.pos_ds.store((r.position_secs() * 10.0) as u32, Ordering::Relaxed);
        state.cycle_c.store((r.position_cycles() * 100.0) as u32, Ordering::Relaxed);
        state.voices.store(r.active_voices() as u32, Ordering::Relaxed);
        state.xruns.store(pcm.xruns(), Ordering::Relaxed);
        // windowed load (~1 s): a lifetime average hides both spikes and creep
        if audio_ns >= 1_000_000_000 {
            state.load_pct.store((render_ns * 100 / audio_ns) as u32, Ordering::Relaxed);
            render_ns = 0;
            audio_ns = 0;
        }
    }
}
