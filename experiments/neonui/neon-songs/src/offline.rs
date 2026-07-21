//! Headless synth-only renderer for the DL7006.
//!
//! Ported from strudel-audio's `OfflineRenderer` (see upstream
//! `crates/strudel-audio/src/offline.rs`) with the soundfont/sample-manifest
//! loading stripped: NEONWIRE songs are pure synthesis (sine/saw/supersaw/
//! wavetable), so nothing needs network or sample banks. Patterns that
//! reference gm_/sample sounds simply render those layers silent.
//!
//! Adds a streaming API (`begin` + `next_block`) so the shell's audio thread
//! can drive playback block-by-block into /dev/snd instead of baking a fixed
//! duration.

use strudel_core::Pattern;
use strudel_dsp::stereo_samples::StereoSlice;
use strudel_internal::{FastMath as _, Zeroable as _, unlikely};

use crate::processor::{AudioThreadProcessor, MainThreadProcessor};

const INV_240: f64 = 1.0 / 240.0;
const SCHED_LOOKAHEAD_CYCLES: f64 = 0.15;

/// Synchronous block renderer: pattern in, stereo f32 blocks out.
pub struct SongRenderer {
    main_proc: MainThreadProcessor,
    audio_proc: AudioThreadProcessor,
    cps: f64,
    inv_cps: f64,
    sample_rate: u32,
    block_size: usize,
    cur_frame: u64,
    scheduled_to_cycles: f64,
    buf: Vec<f32>,
}

impl SongRenderer {
    #[must_use]
    pub fn new(bpm: f64, sample_rate: u32, master_gain: f32) -> Self {
        let (main_ch, audio_ch) = crate::channel::channel(master_gain);
        let main_proc = MainThreadProcessor::new(main_ch);
        let audio_proc = AudioThreadProcessor::new(f64::from(sample_rate), audio_ch);
        let cps = bpm * INV_240;
        Self {
            main_proc,
            audio_proc,
            cps,
            inv_cps: 1.0 / cps,
            sample_rate,
            block_size: 512,
            cur_frame: 0,
            scheduled_to_cycles: 0.0,
            buf: Vec::new(),
        }
    }

    /// Set the active pattern. Must be called before rendering.
    pub fn set_pattern(&mut self, pattern: Pattern) {
        self.main_proc.set_pattern(pattern);
    }

    /// Set the audio block size (frames per render call). Defaults to 512.
    pub fn set_block_size(&mut self, frames: usize) {
        self.block_size = frames.max(1);
    }

    #[must_use]
    pub const fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Current playback position in seconds.
    #[must_use]
    pub fn position_secs(&self) -> f64 {
        self.cur_frame as f64 / f64::from(self.sample_rate)
    }

    /// Current playback position in pattern cycles.
    #[must_use]
    pub fn position_cycles(&self) -> f64 {
        self.position_secs() * self.cps
    }

    /// Arm the pattern clock. Call once before the first `next_block`.
    pub fn begin(&mut self) {
        self.main_proc.set_start_time(0.0);
    }

    /// Render one block of interleaved-deinterleaved audio: the returned slice
    /// is `[L0..Ln, R0..Rn]` (two half-buffers), `block_size` frames long.
    pub fn next_block(&mut self) -> (&[f32], &[f32]) {
        let block = self.block_size;
        let next_frame = self.cur_frame + block as u64;
        let cur_secs = self.cur_frame as f64 / f64::from(self.sample_rate);
        let next_secs = next_frame as f64 / f64::from(self.sample_rate);
        let target_cycles = next_secs.fast_mul_add(self.cps, SCHED_LOOKAHEAD_CYCLES);
        let begin_cycles = self.scheduled_to_cycles.max(cur_secs * self.cps);
        if target_cycles > begin_cycles {
            self.main_proc
                .query_events_packed(begin_cycles, target_cycles, self.inv_cps);
            self.scheduled_to_cycles = target_cycles;
        }

        let buf_len = block * 2;
        if unlikely(self.buf.len() != buf_len) {
            self.buf.resize(buf_len, 0.0);
        }
        self.buf.zero_bytes();

        let mut stereo = StereoSlice::new(&mut self.buf);
        self.audio_proc.render_block(&mut stereo);

        self.cur_frame = next_frame;
        self.buf.split_at(block)
    }

    /// Bake `duration_seconds` through a callback — host-side testing helper.
    pub fn render<F>(&mut self, duration_seconds: f64, mut on_block: F)
    where
        F: FnMut(&[f32], &[f32]),
    {
        self.begin();
        let total_frames = (duration_seconds * f64::from(self.sample_rate)).round() as u64;
        while self.cur_frame < total_frames {
            let (l, r) = self.next_block();
            on_block(l, r);
        }
    }
}
