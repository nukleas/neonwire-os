use core::num::NonZeroU8;

use strudel_core::{ContextKey, Fraction, Pattern, Value, ValueTypeTag};
use strudel_dsp::{
    DistortionType, DspEngine, IrIndex, OrbitBusIndex, SampleRate, SoundId, SynthKind,
    TriggerEvent, U4Base1, VowelIndex, stereo_samples::StereoSlice,
};
use strudel_internal::FastMath as _;
use strudel_music_theory::MidiNoteNumber;
use strudel_soundfont::{GmInstrument, GmSampleIndex, GmSampleKey, GmSampleMask};

use crate::{
    channel::{AudioChannel, MainChannel, RetiredBankEntry},
    mapper::{PitchedSampleKey, SampleKey, SampleMapper, SampleSlotIdx},
};

/// The result from [`MainThreadProcessor::query_missing_banks`].
pub struct MissingBanks {
    pub gm_sample_mask: GmSampleMask,
    pub manifest_banks: Vec<Box<str>>,
}

/// Pattern scheduler living on the **main** (or caller) thread.
#[derive(Debug)]
pub struct MainThreadProcessor {
    /// Current active pattern. `Rc`-based, stays on the creating thread.
    pub(crate) pattern: Option<Pattern>,
    /// Audio-clock time (seconds) at which cycle 0 maps to.
    start_time: f64,
    /// Shared channel to the main thread.
    pub(crate) channel: MainChannel,
    /// Sample name/index/note -> slot_id mapper. Main thread only.
    pub(crate) mapper: SampleMapper,
    /// Old allocations waiting to be dropped after the audio thread moves on.
    trash_pile: Vec<RetiredBankEntry>,
    /// NEONWIRE addition (not upstream): a tee of scheduled events for the
    /// event-rain visualizer. Drained by SongRenderer each block; capped so an
    /// undrained tap can't grow unbounded.
    pub vis_tap: Vec<VisEvent>,
}

/// A scheduled trigger, reduced to what the visualizer needs.
#[derive(Clone, Copy, Debug)]
pub struct VisEvent {
    /// Onset in seconds on the renderer clock.
    pub time: f64,
    /// MIDI-ish note if the event has one, else f32::NEG_INFINITY.
    pub note: f32,
    pub gain: f32,
    /// FNV-1a hash of the sound/bank name (0 = unnamed).
    pub tag: u32,
}

impl MainThreadProcessor {
    /// Create a new processor connected to `channel`.
    pub(crate) fn new(channel: MainChannel) -> Self {
        Self {
            pattern: None,
            start_time: 0.0,
            channel,
            mapper: SampleMapper::new(),
            trash_pile: Vec::new(),
            vis_tap: Vec::new(),
        }
    }

    /// Replace the active pattern. Hot-swap safe; takes effect on the next
    /// [`Self::query_events_packed`] call.
    pub fn set_pattern(&mut self, pattern: Pattern) {
        self.pattern = Some(pattern);
    }

    /// Clear the active pattern. Future ticks will produce no events.
    pub fn clear_pattern(&mut self) {
        self.pattern = None;
    }

    /// Set the audio-clock anchor (seconds) for cycle 0.
    ///
    /// Call this whenever playback starts or the tempo changes.
    pub const fn set_start_time(&mut self, time: f64) {
        self.start_time = time;
    }

    /// Query the active pattern for `[begin, end)` cycles and pack the resulting
    /// [`TriggerEvent`]s into `Channel::event_input`, then Release-store the
    /// count to `Channel::event_count`.
    ///
    /// The audio thread will Acquire-swap `event_count` at the start of its
    /// next render block, making all writes visible before it reads them.
    ///
    /// Also runs deferred sample-allocation GC (~10 Hz cadence).
    pub fn query_events_packed(&mut self, begin: f64, end: f64, inv_cps: f64) {
        // Deferred GC (~10 Hz): drop retired sample allocations the audio thread
        // has confirmed it is done reading.
        self.channel.collect_garbage(&mut self.trash_pile);

        let Some(ref pattern) = self.pattern else {
            return;
        };

        let haps = pattern.query_arc(Fraction::from_float(begin), Fraction::from_float(end));
        let mut count = 0;

        self.channel.write_and_publish(|event_buf| {
            for (hap, event) in haps
                .iter()
                .filter(|h| h.has_onset())
                .zip(event_buf.iter_mut())
            {
                let time_span = hap.whole_or_part();
                let onset = time_span.begin.to_f64();
                let end_f = time_span.end.to_f64();
                let duration = ((end_f - onset) * inv_cps).max(0.01);
                let time = onset.fast_mul_add(inv_cps, self.start_time);

                let mut p = TriggerEvent::DEFAULT;
                p.time = time;
                p.duration = duration as f32;

                apply_value_to_event(hap, &mut p);
                for (key, value) in &hap.context {
                    apply_control_to_event(*key, value, &mut p);
                }

                // NEONWIRE vis tap (see VisEvent)
                if self.vis_tap.len() < 512 {
                    let tag = hap.sound_with_index().map_or(0, |(bank, _)| {
                        bank.bytes()
                            .fold(0x811c_9dc5u32, |h, b| (h ^ u32::from(b)).wrapping_mul(0x0100_0193))
                    });
                    self.vis_tap.push(VisEvent { time, note: p.note, gain: p.gain, tag });
                }

                if p.s == SoundId::Sample
                    && let Some(key) = resolve_sample_key(hap)
                {
                    let midi = (p.note != f32::NEG_INFINITY)
                        .then_some(p.note as u8)
                        .and_then(MidiNoteNumber::new_checked);
                    if let Some(r) = self.mapper.resolve(PitchedSampleKey {
                        identity: key,
                        pitch: midi,
                        zone_info: None,
                    }) {
                        p.sample_id = r.slot_id.get();
                        p.sample_ratio = r.pitch_ratio;
                    }
                }

                *event = p;
                count += 1;
            }

            TriggerEvent::sort_by_time(&mut event_buf[..usize::from(count)]);

            count
        });
    }

    #[must_use]
    pub fn query_missing_banks(&self, begin: Fraction, end: Fraction) -> MissingBanks {
        let mut res = MissingBanks {
            gm_sample_mask: GmSampleMask::new(),
            manifest_banks: Vec::new(),
        };
        let Some(ref pattern) = self.pattern else {
            return res;
        };

        let haps = pattern.query_arc(begin, end);

        for hap in haps.iter().filter(|h| h.has_onset()) {
            let Some(SampleKey {
                bank: base_bank,
                index: sample_idx,
            }) = resolve_sample_key(hap)
            else {
                continue;
            };

            if let Ok(instrument) = base_bank.parse::<GmInstrument>() {
                if let Ok(idx) = GmSampleIndex::try_from(sample_idx) {
                    res.gm_sample_mask.insert(GmSampleKey::new(instrument, idx));
                }
            } else if base_bank.parse::<SynthKind>().is_err()
                && !res.manifest_banks.iter().any(|s| &**s == base_bank)
            {
                res.manifest_banks.push(base_bank.into());
            }
        }
        res
    }

    /// Register and store a batch of `(name, Sample)` pairs in a single atomic swap.
    pub fn register_and_store_batch<'a>(
        &mut self,
        entries: impl IntoIterator<Item = (PitchedSampleKey<'a>, strudel_dsp::sample::Sample)>,
    ) -> usize {
        let resolved: Vec<_> = entries
            .into_iter()
            .map(|(name, sample)| {
                let slot_id = self.mapper.register(name);
                (slot_id, sample)
            })
            .collect();
        let count = resolved.len();
        self.channel
            .store_samples_batch(resolved, &mut self.trash_pile);
        count
    }

    /// Set master gain in the range `[0.0, 2.0]`.
    #[inline]
    pub fn set_master_gain(&mut self, gain: f32) {
        self.channel.set_master_gain(gain);
    }

    #[inline(always)]
    #[must_use]
    pub fn current_gain(&self) -> f32 {
        self.channel.master_gain()
    }

    /// Hush: stop all active voices and clear queued events.
    #[inline]
    pub fn hush(&mut self) {
        self.channel.request_hush();
    }

    /// Panic: silence all voices immediately without release envelopes.
    #[inline]
    pub fn panic_all(&mut self) {
        self.channel.request_panic();
    }

    /// Flush: clear the scheduled event queue; active voices ring out naturally.
    ///
    /// Use this when changing tempo so stale pre-scheduled events are discarded
    /// without cutting off notes that are already sounding.
    #[inline]
    pub fn flush_pending(&mut self) {
        self.channel.request_flush();
    }

    /// Read the active voice count written by the audio thread.
    #[inline]
    #[must_use]
    pub fn active_voices(&self) -> u32 {
        self.channel.voice_count()
    }

    /// Read the current audio clock in seconds.
    #[inline]
    #[must_use]
    pub fn audio_time(&self) -> f64 {
        self.channel.audio_time()
    }
}

/// DSP engine running **exclusively** on the audio callback thread.
///
/// Analogous to `WorkletProcessor` in `strudel-audio-wasm`. Owns the
/// [`DspEngine`] without any lock - there is no mutex in the render path.
pub struct AudioThreadProcessor {
    /// DSP engine - no mutex because this is the sole owner on the audio thread.
    engine: DspEngine,
    /// Shared channel (Arc clone; main thread holds the other).
    channel: AudioChannel,
    /// Monotonically increasing audio time in seconds (derived from sample count).
    current_time: f64,
    /// Audio sample rate (Hz).
    sample_rate: SampleRate,
}

impl AudioThreadProcessor {
    /// Create a new processor. `channel` must be the [`AudioChannel`] half of the
    /// pair given to the corresponding [`MainThreadProcessor`].
    pub(crate) fn new(sample_rate: f64, channel: AudioChannel) -> Self {
        let sample_rate = SampleRate::new(sample_rate);
        Self {
            engine: DspEngine::new(sample_rate),
            channel,
            current_time: 0.0,
            sample_rate,
        }
    }

    /// Render one block of audio in place.
    ///
    /// This is the innermost loop of the audio thread and contains **no mutexes**
    /// on the main event / control path.
    ///
    /// Hot path:
    /// 1. Snapshot current generation + sample slice (one Acquire load, zero allocation).
    /// 2. Drain events (all have pre-resolved `sample_id`/`sample_ratio`).
    /// 3. Apply controls (gain, hush, panic, flush).
    /// 4. Render block.
    /// 5. Signal main thread that we've finished reading this generation.
    #[inline]
    pub fn render_block(&mut self, stereo: &mut StereoSlice<'_>) {
        self.channel.process_generation(|samples| {
            self.channel.drain_and_process(|events| {
                self.engine.schedule_batch(events, samples);
            });

            let gain = self.channel.master_gain_f32();
            self.engine.set_master_gain_unchecked(gain);

            if self.channel.take_hush() {
                self.engine.hush();
            } else if self.channel.take_panic() {
                self.engine.panic();
            }

            if self.channel.take_flush() {
                self.engine.clear_events();
            }

            self.engine.set_current_time(self.current_time);
            self.engine.render_block(samples, stereo);

            let block_size = stereo.channel_len().get();
            self.current_time =
                (block_size as f64).fast_mul_add(self.sample_rate.inv(), self.current_time);

            self.channel
                .set_voice_count(self.engine.active_voices() as u32);
            self.channel.set_audio_time(self.current_time);
        });
    }

    /// Current audio time (seconds) as tracked by this processor.
    #[must_use]
    pub const fn current_time(&self) -> f64 {
        self.current_time
    }

    /// Audio sample rate.
    #[must_use]
    pub const fn sample_rate(&self) -> f64 {
        self.sample_rate.base()
    }

    /// Number of currently active DSP voices.
    #[must_use]
    pub fn active_voices(&self) -> usize {
        self.engine.active_voices()
    }

    /// Number of queued (scheduled but not yet sounding) events.
    #[must_use]
    pub const fn pending_events(&self) -> usize {
        self.engine.pending_events()
    }

    /// Direct access to the DSP engine for advanced configuration
    /// (e.g. `set_delay`). Only call from the audio thread.
    pub const fn engine_mut(&mut self) -> &mut DspEngine {
        &mut self.engine
    }
}

pub fn apply_value_to_event(hap: &strudel_core::Hap<Value>, event: &mut TriggerEvent) {
    match &hap.value {
        Value::String(s) => {
            let is_sound = hap
                .context
                .get(&ContextKey::Type)
                .and_then(Value::as_type_tag)
                == Some(ValueTypeTag::Sound);

            if is_sound {
                event.s = SoundId::classify(s);
            } else if let Ok(midi) = s.parse::<MidiNoteNumber>().map(f32::from) {
                event.note = midi;
            } else {
                event.s = SoundId::classify(s);
            }
        }
        Value::Number(n) => {
            event.note = *n as f32;
        }
        _ => {}
    }
}

pub fn apply_control_to_event(key: ContextKey, value: &Value, event: &mut TriggerEvent) {
    macro_rules! num {
        ($event:expr, $field:ident) => {
            if let Value::Number(n) = value {
                $event.$field = *n as _;
            }
        };
    }

    match key {
        ContextKey::Sound => {
            if let Value::String(s) = value {
                event.s = SoundId::classify(s);
            }
        }
        ContextKey::Note => {
            if let Value::Number(n) = value {
                event.note = *n as f32;
            } else if let Value::String(s) = value
                && let Ok(midi) = s.parse::<MidiNoteNumber>().map(f32::from)
            {
                event.note = midi;
            }
        }
        ContextKey::Frequency => num!(event, freq),
        ContextKey::Gain => num!(event, gain),
        ContextKey::Amp => num!(event, gain),
        ContextKey::Pan => num!(event, pan),
        ContextKey::Attack => num!(event, attack),
        ContextKey::Decay => num!(event, decay),
        ContextKey::Sustain => num!(event, sustain),
        ContextKey::Release => num!(event, release),
        ContextKey::Cut => num!(event, cut),
        ContextKey::Orbit => {
            if let Value::Number(n) = value {
                event.orbit = OrbitBusIndex::new_checked(*n as u8).unwrap_or(OrbitBusIndex::N0);
            }
        }
        ContextKey::Speed => num!(event, speed),
        ContextKey::Begin => num!(event, begin),
        ContextKey::End => num!(event, end),
        ContextKey::Loop => {
            if let Value::Number(n) = value {
                event.loop_ = Some(*n != 0.0);
            }
        }
        ContextKey::Cutoff => num!(event, cutoff),
        ContextKey::Resonance => num!(event, resonance),
        ContextKey::LpEnv => num!(event, lpenv),
        ContextKey::LpAttack => num!(event, lpattack),
        ContextKey::LpDecay => num!(event, lpdecay),
        ContextKey::LpSustain => num!(event, lpsustain),
        ContextKey::LpRelease => num!(event, lprelease),
        ContextKey::Hpf => num!(event, hcutoff),
        ContextKey::Hresonance => num!(event, hresonance),
        ContextKey::HpEnv => num!(event, hpenv),
        ContextKey::HpAttack => num!(event, hpattack),
        ContextKey::HpDecay => num!(event, hpdecay),
        ContextKey::HpSustain => num!(event, hpsustain),
        ContextKey::HpRelease => num!(event, hprelease),
        ContextKey::Bpf => num!(event, bandf),
        ContextKey::BandQ => num!(event, bandq),
        ContextKey::BpEnv => num!(event, bpenv),
        ContextKey::BpAttack => num!(event, bpattack),
        ContextKey::BpDecay => num!(event, bpdecay),
        ContextKey::BpSustain => num!(event, bpsustain),
        ContextKey::BpRelease => num!(event, bprelease),
        ContextKey::PEnv => num!(event, penv),
        ContextKey::PAttack => num!(event, pattack),
        ContextKey::PDecay => num!(event, pdecay),
        ContextKey::PSustain => num!(event, psustain),
        ContextKey::PRelease => num!(event, prelease),
        ContextKey::Anchor => num!(event, panchor),
        ContextKey::Delay => num!(event, delay),
        ContextKey::DelayTime => num!(event, delaytime),
        ContextKey::DelayFeedback => num!(event, delayfeedback),
        ContextKey::Room => num!(event, room),
        ContextKey::RoomSize => num!(event, roomsize),
        ContextKey::Distort => num!(event, distort),
        ContextKey::DistortType => {
            if let Value::Number(n) = value {
                event.distorttype = DistortionType::from_type_index(*n as i32);
            }
        }
        ContextKey::Shape => num!(event, shape),
        ContextKey::Crush => {
            if let Value::Number(n) = value {
                event.crush = U4Base1::new_checked(*n as u8);
            }
        }
        ContextKey::Coarse => {
            if let Value::Number(n) = value {
                event.coarse = NonZeroU8::new(*n as u8);
            }
        }
        ContextKey::PhaserRate => num!(event, phaserrate),
        ContextKey::PhaserDepth => num!(event, phaserdepth),
        ContextKey::Tremolo => num!(event, tremolo),
        ContextKey::TremoloDepth => num!(event, tremolodepth),
        ContextKey::Vib => num!(event, vib),
        ContextKey::VibMod => num!(event, vibmod),
        ContextKey::Chorus => num!(event, chorus),
        ContextKey::ChorusSpeed => num!(event, chorusspeed),
        ContextKey::Vowel => {
            if let Value::Number(n) = value {
                event.vowel = VowelIndex::new_checked(*n as u8);
            }
        }
        ContextKey::GrainSize => num!(event, grainsize),
        ContextKey::Scatter => num!(event, scatter),
        ContextKey::Ir => {
            if let Value::Number(n) = value {
                event.ir = IrIndex::new_checked(*n as u8);
            }
        }
        ContextKey::FmIndex => num!(event, fm),
        ContextKey::FmRatio => num!(event, fmh),
        ContextKey::FmEnv => num!(event, fmenv),
        ContextKey::FmAttack => num!(event, fmattack),
        ContextKey::FmDecay => num!(event, fmdecay),
        ContextKey::FmSustain => num!(event, fmsustain),
        ContextKey::FmRelease => num!(event, fmrelease),
        ContextKey::RoomDamp => num!(event, roomdamp),
        ContextKey::Detune => num!(event, detune),
        ContextKey::Unison => {
            if let Value::Number(n) = value {
                event.unison = U4Base1::new_checked(*n as u8);
            }
        }
        ContextKey::Spread => num!(event, spread),
        ContextKey::Width => num!(event, width),
        ContextKey::PwmRate => num!(event, pwmrate),
        ContextKey::PwmDepth => num!(event, pwmdepth),
        ContextKey::Velocity => num!(event, velocity),
        ContextKey::Clip => num!(event, clip),

        // no-ops, avoid using _ => {}
        // so we can identify what variants may be missing from TriggerEvent
        // and explicitly add new variants here since it would otherwise fail to compile
        ContextKey::Bank => {}
        ContextKey::Chord => {}
        ContextKey::Color => {}
        ContextKey::Dictionary => {}
        ContextKey::DistortVol => {}
        ContextKey::DuckAtt => {}
        ContextKey::DuckDepth => {}
        ContextKey::DuckOns => {}
        ContextKey::DuckOrbit => {}
        ContextKey::DuckRel => {}
        ContextKey::Duration => {}
        ContextKey::Locations => {}
        ContextKey::SectionLoc => {}
        ContextKey::N => {}
        ContextKey::Scale => {}
        ContextKey::ShapeVol => {}
        ContextKey::Tags => {}
        ContextKey::Target => {}
        ContextKey::TremoloShape => {}
        ContextKey::TremoloSkew => {}
        ContextKey::TremoloPhase => {}
        ContextKey::Type => {}
        ContextKey::Unit => {}
    }
}

#[inline]
#[must_use]
pub fn resolve_sample_key(hap: &strudel_core::Hap<Value>) -> Option<SampleKey<'_>> {
    hap.sound_with_index().map(|(bank, index)| SampleKey {
        bank,
        index: SampleSlotIdx::new(index),
    })
}
