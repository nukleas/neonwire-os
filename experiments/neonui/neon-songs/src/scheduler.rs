//! Scheduling state: timing math only, no DSP, no Pattern.
//!
//! `SchedulerState` tracks the audio-clock anchor, BPM/CPS, and the
//! "scheduled up to" cursor.  It tells [Player] when and what cycle range
//! to pass to [MainThreadProcessor::query_events_packed], mirroring the
//! role of the JavaScript `PatternScheduler` in the WASM front-end.
//!
//! [Player]: crate::Player
//! [MainThreadProcessor::query_events_packed]: crate::MainThreadProcessor::query_events_packed

use strudel_dsp::Cps;
use strudel_internal::FastMath as _;

const INV_240: f64 = 1.0 / 240.0;

/// Timing state for the scheduler loop (~10 Hz main-thread tick).
///
/// Tracks BPM, CPS, the audio-clock anchor for cycle 0, and the "scheduled up
/// to" cursor.  Does not hold any pattern or DSP state - those live in
/// [`MainThreadProcessor`] and [`AudioThreadProcessor`] respectively.
#[derive(Debug)]
pub(crate) struct SchedulerState {
    /// Current tempo in beats per minute.
    bpm: f64,
    /// Derived cycles per second.
    cps: Cps,
    /// Audio-clock time (seconds) that aligns with cycle 0.
    pub start_time: f64,
    /// Upper edge of the last cycle range submitted to `query_events_packed`.
    /// Everything below this cursor is already scheduled.
    pub scheduled_to: f64,
    /// Cycle position saved at pause time, so resume can reconstruct `start_time`.
    paused_cycle: f64,
}

impl SchedulerState {
    /// How many cycles ahead to schedule (~0.15 cycles @ 120 BPM ≈ 300 ms).
    const LOOKAHEAD: f64 = 0.15;

    pub fn new(bpm: f64) -> Self {
        let cps = Cps::new(Self::bpm_to_cps(bpm));
        Self {
            bpm,
            cps,
            start_time: 0.0,
            scheduled_to: 0.0,
            paused_cycle: 0.0,
        }
    }

    #[inline]
    fn bpm_to_cps(bpm: f64) -> f64 {
        bpm * INV_240
    }

    /// Current cycle position derived from `audio_time` and the anchor clock.
    #[inline]
    pub fn current_cycle(&self, audio_time: f64) -> f64 {
        (audio_time - self.start_time) * self.cps
    }

    /// Prepare for playback start.  Sets `start_time` to `now` and resets the
    /// scheduler cursor.
    pub const fn start(&mut self, audio_now: f64) {
        self.start_time = audio_now;
        self.scheduled_to = 0.0;
        self.paused_cycle = 0.0;
    }

    /// Save cycle position for pause; the audio clock keeps running.
    pub fn pause(&mut self, audio_now: f64) {
        self.paused_cycle = self.current_cycle(audio_now);
    }

    /// Reconstruct `start_time` from the saved cycle position so cycle numbering
    /// is continuous after resume.
    pub fn resume(&mut self, audio_now: f64) {
        self.start_time = self.paused_cycle.fast_mul_add(-self.cps.inv(), audio_now);
        self.scheduled_to = self.paused_cycle;
    }

    /// Update BPM.  Adjusts `start_time` so cycle numbering is continuous.
    ///
    /// Returns `true` if the BPM actually changed (useful to decide whether to
    /// flush pre-scheduled events from the DSP engine).
    pub fn set_bpm(&mut self, bpm: f64, audio_now: f64) -> bool {
        let bpm = bpm.clamp(10.0, 999.0);
        if (bpm - self.bpm).abs() < f64::EPSILON {
            return false;
        }

        // Snapshot current cycle under the old CPS before changing it.
        let current_cycle = self.current_cycle(audio_now);

        self.bpm = bpm;
        self.cps = Cps::new(Self::bpm_to_cps(bpm));

        // Reconstruct start_time so the same cycle maps to audio_now under new CPS.
        self.start_time = current_cycle.fast_mul_add(-self.cps.inv(), audio_now);
        // Cycle-space cursor is still valid (cycle position didn't jump).
        self.scheduled_to = current_cycle;

        true
    }

    /// Current BPM.
    #[must_use]
    pub const fn bpm(&self) -> f64 {
        self.bpm
    }

    /// Compute the next `(begin, end, inv_cps)` range to schedule, if any.
    ///
    /// Returns `Some((begin_cycles, end_cycles, inv_cps))` when the lookahead
    /// window has advanced past the `scheduled_to` cursor.  The caller should
    /// then pass these values to [`MainThreadProcessor::query_events_packed`].
    ///
    /// Returns `None` if the buffer is still full (no scheduling needed yet).
    ///
    /// Also snaps `scheduled_to` forward to `current_cycle` if pattern-hot-swap
    /// caused the lookahead window to slip behind the play head.
    pub fn advance_tick(&mut self, audio_now: f64) -> Option<(f64, f64, f64)> {
        let current_cycle = self.current_cycle(audio_now);

        // After a hot-swap we may be behind; snap forward.
        if current_cycle > self.scheduled_to {
            self.scheduled_to = current_cycle;
        }

        let query_end = current_cycle + Self::LOOKAHEAD;
        (query_end > self.scheduled_to).then(|| {
            let begin = self.scheduled_to;
            self.scheduled_to = query_end;
            (begin, query_end, self.cps.inv())
        })
    }

    /// Force the scheduler cursor back to `current_cycle` so the next
    /// `advance_tick` starts scheduling from *now*.
    ///
    /// Call this after a hot-swap pattern so new events are queued without
    /// waiting for the old lookahead window to drain.
    pub fn kick(&mut self, audio_now: f64) {
        self.scheduled_to = self.current_cycle(audio_now);
    }
}
