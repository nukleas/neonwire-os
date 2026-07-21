//! Song playback engine: evaluates a .strudel file (neon-songs) and streams
//! the synth render to /dev/snd on its own thread. One SongPlayer = one song;
//! drop it to stop (frees hw:0,5 for the Music app's sequencer and vice versa).

use std::io;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use crate::audio::{speaker_amp, Pcm, PERIOD, RATE};

/// dirt-samples-layout WAV banks on the SD (bank dir -> s("<bank>") slots).
pub const SAMPLES_DIR: &str = "/mnt/sd/linux-lab/samples";

pub struct SongState {
    pub online: AtomicBool,   // pcm opened, rendering
    pub pos_ds: AtomicU32,    // playback position, deciseconds
    pub cycle_c: AtomicU32,   // pattern cycle × 100
    pub load_pct: AtomicU32,  // render cost as % of realtime
    pub volume: AtomicU32,    // 0..=256, shared with UI
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
            pos_ds: AtomicU32::new(0),
            cycle_c: AtomicU32::new(0),
            load_pct: AtomicU32::new(0),
            volume: AtomicU32::new(volume.min(256)),
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
    state.online.store(true, Ordering::Relaxed);

    r.begin();
    let mut ibuf = vec![0i16; PERIOD * 2];
    let mut render_ns: u64 = 0;
    let mut audio_ns: u64 = 0;
    while !state.quit.load(Ordering::Relaxed) {
        let vol = state.volume.load(Ordering::Relaxed).min(256) as f32 / 256.0;
        let gain = 28000.0 * vol;
        let tb = std::time::Instant::now();
        let (l, rr) = r.next_block();
        for i in 0..PERIOD {
            ibuf[i * 2] = (l[i].clamp(-1.0, 1.0) * gain) as i16;
            ibuf[i * 2 + 1] = (rr[i].clamp(-1.0, 1.0) * gain) as i16;
        }
        render_ns += tb.elapsed().as_nanos() as u64;
        audio_ns += PERIOD as u64 * 1_000_000_000 / RATE as u64;
        let _ = pcm.write(&ibuf);

        state.pos_ds.store((r.position_secs() * 10.0) as u32, Ordering::Relaxed);
        state.cycle_c.store((r.position_cycles() * 100.0) as u32, Ordering::Relaxed);
        if audio_ns > 0 {
            state.load_pct.store((render_ns * 100 / audio_ns) as u32, Ordering::Relaxed);
        }
    }
}
