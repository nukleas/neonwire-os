//! Main-thread sample mapper.
//!
//! Maps sample requests (bank name + index + optional pitch) to stable
//! `SlotId`s that index into the shared `[Option<Sample>]` slice read by the
//! audio thread.

use core::fmt;

use rustc_hash::FxHashMap;
use strudel_internal::FastMath as _;
use strudel_music_theory::MidiNoteNumber;
use strudel_soundfont::{GmSampleIndex, GmSampleKey};

/// A bank id (stable index assigned by insertion order).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct BankId(u32);

impl BankId {
    #[inline]
    const fn increment(&mut self) {
        self.0 += 1;
    }
}

/// Index of a sample within a bank (e.g. `bd:2` -> index 2).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct SampleSlotIdx(u32);

impl SampleSlotIdx {
    pub const ZERO: Self = Self(0);

    #[inline]
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    #[inline]
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl TryFrom<SampleSlotIdx> for GmSampleIndex {
    type Error = ();

    #[inline]
    fn try_from(value: SampleSlotIdx) -> Result<Self, Self::Error> {
        Self::try_from(value.0)
    }
}

/// A slot id: a flat index into the caller-owned `[Option<Sample>]` slice.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct SlotId(u32);

impl SlotId {
    #[cfg(test)]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    #[inline]
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    #[inline]
    const fn increment(&mut self) {
        self.0 += 1;
    }
}

/// A standard sample key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SampleKey<'a> {
    /// The bank name.
    pub bank: &'a str,
    /// The sample index (defaults to 0).
    pub index: SampleSlotIdx,
}

impl<'a> SampleKey<'a> {
    /// Creates a new sample key.
    #[inline]
    #[must_use]
    pub const fn new(bank: &'a str, index: SampleSlotIdx) -> Self {
        Self { bank, index }
    }
}

impl<'a> From<&'a str> for SampleKey<'a> {
    /// Converts a bank name to a sample key with a sample index of 0.
    #[inline]
    fn from(bank: &'a str) -> Self {
        Self::new(bank, SampleSlotIdx::ZERO)
    }
}

impl From<GmSampleKey> for SampleKey<'_> {
    /// Converts a [`GmSampleKey`] to a sample key
    /// with the bank name set to the instrument's [`base_bank_name`] and equivalent sample index.
    ///
    /// [`base_bank_name`]: strudel_soundfont::GmInstrument::base_bank_name
    #[inline]
    fn from(value: GmSampleKey) -> Self {
        Self::new(
            value.instrument.base_bank_name(),
            SampleSlotIdx::new(u32::from(value.sample_index)),
        )
    }
}

impl fmt::Display for SampleKey<'_> {
    /// Displays the sample key in standard strudel mini-notation as `bank:index`.
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.bank)?;
        f.write_str(":")?;
        self.index.get().fmt(f)
    }
}

/// A composite key: which bank + which sample index within it.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
struct SampleId {
    bank: BankId,
    index: SampleSlotIdx,
}

/// A composite key: a specific MIDI note within a specific sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct NoteSampleId {
    sample: SampleId,
    note: MidiNoteNumber,
}

/// Zone information (soundfont key-range + detune).
#[derive(Debug, Clone, Copy)]
pub struct ZoneInfo {
    /// Lowest MIDI note this zone covers.
    pub key_range_low: MidiNoteNumber,
    /// Highest MIDI note this zone covers.
    pub key_range_high: MidiNoteNumber,
    /// Exact recording pitch in cents.
    pub base_detune_cents: f32,
}

/// A resolved slot with the zone info that produced it.
#[derive(Debug, Clone, Copy)]
struct ZoneEntry {
    /// The slot id.
    slot_id: SlotId,
    /// The zone information.
    info: ZoneInfo,
}

/// A sample key with an optional pitch and zone info.
#[derive(Debug, Clone, Copy)]
pub struct PitchedSampleKey<'a> {
    /// Identity of the sample.
    pub identity: SampleKey<'a>,
    /// Optional pitch at playback time.
    pub pitch: Option<MidiNoteNumber>,
    /// Optional zone info.
    pub zone_info: Option<ZoneInfo>,
}

impl<'a> PitchedSampleKey<'a> {
    #[inline]
    #[must_use]
    pub const fn unpitched(bank: &'a str, index: SampleSlotIdx) -> Self {
        Self {
            identity: SampleKey { bank, index },
            pitch: None,
            zone_info: None,
        }
    }

    #[inline]
    #[must_use]
    pub const fn pitched(bank: &'a str, index: SampleSlotIdx, note: MidiNoteNumber) -> Self {
        Self {
            identity: SampleKey { bank, index },
            pitch: Some(note),
            zone_info: None,
        }
    }

    #[inline]
    #[must_use]
    pub const fn ranged(
        bank: &'a str,
        index: SampleSlotIdx,
        note: MidiNoteNumber,
        zone_info: ZoneInfo,
    ) -> Self {
        Self {
            identity: SampleKey { bank, index },
            pitch: Some(note),
            zone_info: Some(zone_info),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ResolvedSample {
    /// Flat index into the caller-owned `[Option<Sample>]` slice.
    pub slot_id: SlotId,
    /// Pre-computed pitch ratio. 1.0 = exact match.
    pub pitch_ratio: f32,
}

impl ResolvedSample {
    /// Creates a new resolved sample with a `pitch_ratio` of 1.0.
    #[inline]
    #[must_use]
    pub const fn from_exact_match(slot_id: SlotId) -> Self {
        Self {
            slot_id,
            pitch_ratio: 1.0,
        }
    }
}

#[inline]
#[must_use]
pub fn pitch_ratio_f32(from_midi: i32, to_midi: i32) -> f32 {
    if from_midi == to_midi {
        return 1.0;
    }
    const INV_12: f64 = 1.0 / 12.0;
    ((f64::from(to_midi - from_midi)) * INV_12).fast_exp2() as f32
}

#[inline]
#[must_use]
pub fn pitch_ratio_from_cents(target_midi: MidiNoteNumber, base_detune_cents: f32) -> f32 {
    const INV_1200: f64 = 1.0 / 1200.0;
    let diff = f64::from(target_midi).fast_mul_add(100.0, -f64::from(base_detune_cents));
    (diff * INV_1200).fast_exp2() as f32
}

/// Tracks soundfont key-range zones for ranged sample resolution.
#[derive(Debug, Default)]
struct ZoneIndex {
    zone_ranges: FxHashMap<SampleId, Vec<ZoneEntry>>,
}

impl ZoneIndex {
    fn register(&mut self, sample: SampleId, slot_id: SlotId, info: ZoneInfo) {
        let zones = self.zone_ranges.entry(sample).or_default();
        if !zones.iter().any(|z| z.slot_id == slot_id) {
            zones.push(ZoneEntry { slot_id, info });
        }
    }

    fn resolve(&self, sample: SampleId, note: MidiNoteNumber) -> Option<ResolvedSample> {
        let zones = self.zone_ranges.get(&sample)?;
        for zone in zones {
            if note.get() >= zone.info.key_range_low.get()
                && note.get() <= zone.info.key_range_high.get() + 1
            {
                return Some(ResolvedSample {
                    slot_id: zone.slot_id,
                    pitch_ratio: pitch_ratio_from_cents(note, zone.info.base_detune_cents),
                });
            }
        }
        None
    }
}

/// A structure responsible for maintaining a map of all registered samples.
#[derive(Debug, Default)]
pub struct SampleMapper {
    /// Bank name -> bank id.
    bank_ids: FxHashMap<Box<str>, BankId>,
    /// Next bank id.
    next_bank_id: BankId,

    /// Note-mapped samples.
    note_slots: FxHashMap<NoteSampleId, SlotId>,

    /// Per sample sorted note list.
    ///
    /// Kept sorted for O(n) nearest-neighbor.
    note_lists: FxHashMap<SampleId, Vec<(MidiNoteNumber, SlotId)>>,

    /// Default sample per (bank, index).
    index_defaults: FxHashMap<SampleId, SlotId>,

    /// Next slot id.
    next_slot_id: SlotId,

    /// Soundfont key-range zone resolution.
    zones: ZoneIndex,
}

impl SampleMapper {
    /// Creates a new mapper with a default state.
    #[inline]
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn get_or_insert_bank_id(&mut self, bank: &str) -> BankId {
        if let Some(&id) = self.bank_ids.get(bank) {
            return id;
        }
        let id = self.next_bank_id;
        self.bank_ids.insert(bank.into(), id);
        self.next_bank_id.increment();
        id
    }

    fn sample_id(&mut self, key: SampleKey) -> SampleId {
        SampleId {
            bank: self.get_or_insert_bank_id(key.bank),
            index: key.index,
        }
    }

    /// Register a sample.
    pub fn register(&mut self, key: PitchedSampleKey) -> SlotId {
        if let Some(note) = key.pitch {
            if let Some(zone) = key.zone_info {
                self.register_ranged(key.identity, note, zone)
            } else {
                self.register_pitched(key.identity, note)
            }
        } else {
            self.register_standard(key.identity)
        }
    }

    /// Register a standard sample.
    pub fn register_standard(&mut self, key: SampleKey) -> SlotId {
        let sample_id = self.sample_id(key);

        if let Some(&slot) = self.index_defaults.get(&sample_id) {
            return slot;
        }

        let slot = self.alloc_slot();
        self.index_defaults.insert(sample_id, slot);
        slot
    }

    /// Register a pitched sample.
    pub fn register_pitched(&mut self, key: SampleKey, note: MidiNoteNumber) -> SlotId {
        let sample_id = self.sample_id(key);
        let note_id = NoteSampleId {
            sample: sample_id,
            note,
        };

        if let Some(&id) = self.note_slots.get(&note_id) {
            return id;
        }

        let slot = self.alloc_slot();
        self.note_slots.insert(note_id, slot);

        let list = self.note_lists.entry(sample_id).or_default();
        let pos = list.partition_point(|(n, _)| *n < note);
        list.insert(pos, (note, slot));

        self.index_defaults.entry(sample_id).or_insert(slot);

        slot
    }

    /// Register a ranged (soundfont zone) sample.
    pub fn register_ranged(
        &mut self,
        key: SampleKey,
        note: MidiNoteNumber,
        zone_info: ZoneInfo,
    ) -> SlotId {
        let slot = self.register_pitched(key, note);
        let sample_id = self.sample_id(key);
        self.zones.register(sample_id, slot, zone_info);
        slot
    }

    /// Resolve a request into a playable sample.
    #[must_use]
    pub fn resolve(&self, key: PitchedSampleKey) -> Option<ResolvedSample> {
        let bank_id = *self.bank_ids.get(key.identity.bank)?;
        let sample_id = SampleId {
            bank: bank_id,
            index: key.identity.index,
        };

        if let Some(target_note) = key.pitch {
            // Soundfont zone key ranges first (first match wins).
            if let Some(resolved) = self.zones.resolve(sample_id, target_note) {
                return Some(resolved);
            }

            // Exact match.
            if let Some(&slot_id) = self.note_slots.get(&NoteSampleId {
                sample: sample_id,
                note: target_note,
            }) {
                return Some(ResolvedSample::from_exact_match(slot_id));
            }

            // Nearest neighbor.
            if let Some(list) = self.note_lists.get(&sample_id)
                && !list.is_empty()
            {
                let target = i32::from(target_note);

                let (source_note, slot_id) = list
                    .iter()
                    .min_by_key(|(n, _)| (i32::from(*n) - target).abs())
                    .copied()
                    .unwrap();

                let pitch_ratio = pitch_ratio_f32(i32::from(source_note), target);

                return Some(ResolvedSample {
                    slot_id,
                    pitch_ratio,
                });
            }
        }

        let slot_id = *self.index_defaults.get(&sample_id)?;
        Some(ResolvedSample::from_exact_match(slot_id))
    }

    const fn alloc_slot(&mut self) -> SlotId {
        let id = self.next_slot_id;
        self.next_slot_id.increment();
        id
    }
}

#[cfg(test)]
mod tests {
    use strudel_music_theory::{Octave, PitchClass, WesternPitch};

    use super::*;

    #[test]
    fn test_basic_registration() {
        let mut m = SampleMapper::new();
        let s0 = m.register_standard(SampleKey::from("bd"));
        assert_eq!(s0.get(), 0);
        assert_eq!(m.register_standard(SampleKey::from("bd")).get(), 0);
    }

    #[test]
    fn test_index_mapping() {
        let mut m = SampleMapper::new();
        let s0 = m.register_standard(SampleKey::from("bd"));
        let s_base = m.register_standard(SampleKey::from("bd"));
        let s1 = m.register_standard(SampleKey::from("bd:1"));

        assert_eq!(s0, s_base);
        assert_ne!(s0, s1);
    }

    #[test]
    fn test_pitched_exact() {
        let mut m = SampleMapper::new();

        let c4 = MidiNoteNumber::from_pitch(WesternPitch::new(PitchClass::C, Octave::Oct4));
        let key = SampleKey::from("piano");

        m.register_pitched(key, c4);

        let resolved = m
            .resolve(PitchedSampleKey {
                identity: key,
                pitch: Some(c4),
                zone_info: None,
            })
            .unwrap();

        assert_eq!(resolved.pitch_ratio, 1.0);
    }

    #[test]
    fn test_nearest_neighbour() {
        let mut m = SampleMapper::new();

        let c4 = MidiNoteNumber::from_pitch(WesternPitch::new(PitchClass::C, Octave::Oct4));
        let d4 = MidiNoteNumber::from_pitch(WesternPitch::new(PitchClass::D, Octave::Oct4));

        let key = SampleKey::from("piano");

        let slot = m.register_pitched(key, c4);

        let resolved = m
            .resolve(PitchedSampleKey {
                identity: key,
                pitch: Some(d4),
                zone_info: None,
            })
            .unwrap();

        assert_eq!(resolved.slot_id, slot);

        let diff = f64::from(d4.get() - c4.get());
        let expected = (diff / 12.0).fast_exp2() as f32;

        assert!((resolved.pitch_ratio - expected).abs() < 1e-5);
    }
}
