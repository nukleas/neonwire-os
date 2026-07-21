use core::{
    marker::PhantomData,
    ptr, slice,
    sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize},
};
use std::sync::Arc;

use strudel_dsp::{TriggerEvent, sample::Sample};
use strudel_internal::{
    atomic::{ACQUIRE, RELAXED, RELEASE, TypedAtomic, TypedAtomicPtr},
    mailbox::SingleSlotMailbox,
};

use crate::mapper::SlotId;
// Matches the WASM crate

/// Maximum events per scheduling tick.
pub(crate) const MAX_QUEUED_EVENTS: usize = u8::MAX as usize + 1;

const MASTER_HEADROOM: f32 = 0.8;
const INV_MASTER_HEADROOM: f32 = 1.0 / 0.8;

type Relaxed<T> = TypedAtomic<T, RELAXED, RELAXED, RELAXED>;

/// Relaxed peek, Release store, Acquire swap.
type PendingFlag = TypedAtomic<AtomicBool, RELAXED, RELEASE, ACQUIRE>;

/// Relaxed both sides - one-block lag is fine for UI and gain.
type BlockLagU32 = Relaxed<AtomicU32>;

type SharedF64 = TypedAtomic<AtomicU64, RELAXED, RELAXED, RELAXED>;

/// Audio-thread Acquire load; main-thread Release swap.
///
/// The audio thread Acquire-loads `samples_ptr` once per render block, creating a
/// happens-before edge with the main thread's Release swap. This chain also makes the
/// Release-stored `samples_len` transitively visible to the audio thread via a
/// subsequent Relaxed load.
type SamplePtr = TypedAtomicPtr<Option<Sample>, ACQUIRE, RELEASE, RELEASE>;

/// Main-thread Release store; audio-thread Relaxed load.
///
/// The audio thread can use Relaxed ordering because the required synchronization is
/// already provided by the Acquire load of `samples_ptr` (the Release swap of
/// `samples_ptr` happened-after the Release store of `samples_len`).
type SampleLen = TypedAtomic<AtomicUsize, RELAXED, RELEASE, RELAXED>;

type SampleGen = Relaxed<AtomicU32>;

/// An entry in the main thread's deferred-drop list.
///
/// Created whenever the main thread publishes a new sample slice allocation. Held
/// in `MainThreadProcessor::trash_pile` until the audio thread confirms, via
/// [`AudioChannel::mark_generation_processed`], that it has finished reading the
/// superseded generation. Dropped by [`MainChannel::collect_garbage`].
#[derive(Debug)]
pub(crate) struct RetiredBankEntry {
    /// Thin pointer to the retired allocation (base of the `Box<[Option<Sample>]>`).
    ptr: *mut Option<Sample>,
    /// Element count of the retired allocation.
    len: usize,
    /// The generation that replaced this allocation. Safe to drop once
    /// `processed_gen >= generation`.
    generation: u32,
}

// SAFETY: Only the main thread pushes entries into, and pops entries out of,
// `MainThreadProcessor::trash_pile`. The field is never shared between threads.
unsafe impl Send for RetiredBankEntry {}

/// Zero-length static used as the initial `samples_ptr` sentinel value.
///
/// The audio thread always receives a valid (zero-length) slice before any sample is
/// loaded - no null-pointer branch needed in the hot path.
static EMPTY_SAMPLE: Option<Sample> = None;

/// Raw atomic storage shared between the two channel handles.
///
/// **No public methods.**  All access is mediated by [`MainChannel`] or
/// [`AudioChannel`], which enforce which operations each thread may perform.
#[derive(Debug)]
struct ChannelInner {
    /// Packed events written by the main thread and read by the audio thread.
    events: SingleSlotMailbox<[TriggerEvent; MAX_QUEUED_EVENTS]>,

    /// Active voice count: audio writes (Relaxed), main reads (Relaxed) for UI.
    voice_count: BlockLagU32,

    /// Master gain bits (f32 as u32): main writes (Relaxed), audio reads (Relaxed).
    master_gain_bits: BlockLagU32,

    /// Set by main (Release); consumed by audio (Acquire) -> `DspEngine::hush()`.
    hush_pending: PendingFlag,

    /// Set by main (Release); consumed by audio (Acquire) -> `DspEngine::panic()`.
    panic_pending: PendingFlag,

    /// Set by main (Release); consumed by audio (Acquire) -> `DspEngine::clear_events()`.
    ///
    /// Unlike `hush_pending`, active voices ring out - only the scheduled event queue
    /// is cleared.
    flush_pending: PendingFlag,

    /// Audio time (f64 bits): audio writes (Relaxed), main reads (Relaxed).
    audio_time_bits: SharedF64,

    /// Thin pointer to the base of the current `Box<[Option<Sample>]>`.
    ///
    /// Ordering protocol:
    /// - Main thread writes `samples_len` with Release, then swaps `samples_ptr`
    ///   with Release.
    /// - Audio thread Acquire-loads `samples_ptr`; the Acquire synchronizes with
    ///   the Release swap, which happened-after the Release store of `samples_len`,
    ///   making `samples_len` visible via a subsequent Relaxed load.
    samples_ptr: SamplePtr,

    /// Element count of the current sample slice.
    samples_len: SampleLen,

    /// Incremented (Relaxed) by the main thread on every pointer swap.
    ///
    /// Part of the deferred-free handshake: main bumps this before retiring the old
    /// allocation; the audio thread stores the generation it just finished into
    /// `samples_processed_gen`.
    samples_current_gen: SampleGen,

    /// Relaxed-stored by the audio thread at the end of every render block.
    ///
    /// Main reads this in [`MainChannel::collect_garbage`] to determine which
    /// retired allocations are safe to drop.
    samples_processed_gen: SampleGen,
}

// SAFETY:
// - `event_input` (UnsafeCell) is protected by the Release/Acquire ordering on
//   `event_count`: MainChannel is the sole writer; AudioChannel is the sole
//   reader. The Acquire swap in `drain_events` synchronizes with the Release
//   store in `write_and_publish`, making all writes visible before any read.
//
// - `samples_ptr` (AtomicPtr via SamplePtr) is protected by its ordering constants:
//   the main thread Release-stores `samples_len` then Release-swaps `samples_ptr`;
//   the audio thread Acquire-loads `samples_ptr`. The generation handshake ensures
//   no allocation is freed while the audio thread holds a reference to it.
//
// - All other fields use atomics (Sync by definition).
unsafe impl Sync for ChannelInner {}
unsafe impl Send for ChannelInner {}

impl ChannelInner {
    fn new(initial_gain: f32) -> Arc<Self> {
        let struct_ptr = Arc::into_raw(Arc::<Self>::new_uninit())
            .cast_mut()
            .cast::<Self>();

        // SAFETY: struct_ptr points to a valid well aligned pointer, and we initialize every field
        unsafe {
            SingleSlotMailbox::init_with_repeated(
                &raw mut (*struct_ptr).events,
                TriggerEvent::DEFAULT,
            );

            (&raw mut (*struct_ptr).voice_count).write(BlockLagU32::new(0));

            (&raw mut (*struct_ptr).master_gain_bits).write(BlockLagU32::new(
                (initial_gain.clamp(0.0, 2.0) * MASTER_HEADROOM).to_bits(),
            ));

            (&raw mut (*struct_ptr).hush_pending).write(PendingFlag::new(false));
            (&raw mut (*struct_ptr).panic_pending).write(PendingFlag::new(false));
            (&raw mut (*struct_ptr).flush_pending).write(PendingFlag::new(false));
            (&raw mut (*struct_ptr).audio_time_bits).write(SharedF64::new(0_f64.to_bits()));

            (&raw mut (*struct_ptr).samples_ptr).write(SamplePtr::new(
                &raw const EMPTY_SAMPLE as *mut Option<Sample>,
            ));
            (&raw mut (*struct_ptr).samples_len).write(SampleLen::new(0));
            (&raw mut (*struct_ptr).samples_current_gen).write(SampleGen::new(0));
            (&raw mut (*struct_ptr).samples_processed_gen).write(SampleGen::new(0));

            Arc::from_raw(struct_ptr)
        }
    }
}

impl Drop for ChannelInner {
    fn drop(&mut self) {
        // Free the currently-active sample slice, unless it's still the
        // static zero-length sentinel (which must never be freed).
        let ptr = self.samples_ptr.load();
        let len = self.samples_len.load();
        let is_static = ptr::eq(ptr as *const (), &raw const EMPTY_SAMPLE as *const ());
        if !is_static && len > 0 {
            let slice: *mut [Option<Sample>] = ptr::slice_from_raw_parts_mut(ptr, len);
            // SAFETY: `ptr` was produced by `Box::into_raw` on a
            // `Box<[Option<Sample>]>` with `len` elements, in
            // `publish_sample_slice`. `ChannelInner` is only dropped once the
            // last `Arc<ChannelInner>` (shared by `MainChannel` and
            // `AudioChannel`) is gone, so no thread can still be reading
            // through `ptr`.
            unsafe { drop(Box::from_raw(slice)) };
        }
    }
}

/// Create a linked pair of channel handles.
///
/// Call this once when setting up the player. Keep the [`MainChannel`] on the
/// creating thread; move the [`AudioChannel`] into the cpal callback closure.
///
/// `initial_gain` is pre-applied so the audio thread sees the correct gain on
/// its very first block.
pub(crate) fn channel(initial_gain: f32) -> (MainChannel, AudioChannel) {
    let inner = ChannelInner::new(initial_gain);
    (
        MainChannel {
            inner: Arc::clone(&inner),
            _not_send: PhantomData,
        },
        AudioChannel { inner },
    )
}

/// Channel handle for the **main thread**.
///
/// Exposes only the operations the main thread is permitted to perform.
///
/// # `!Send`
///
/// `MainChannel` deliberately opts out of [`Send`] via [`PhantomData`].
/// This is belt-and-suspenders: [`MainThreadProcessor`] is already `!Send` due
/// to the `Rc`-based [`Pattern`], but the marker makes the constraint explicit
/// at the channel level too.
///
/// [`MainThreadProcessor`]: crate::processor::MainThreadProcessor
/// [`Pattern`]: strudel_core::Pattern
#[derive(Debug)]
pub(crate) struct MainChannel {
    inner: Arc<ChannelInner>,
    _not_send: PhantomData<*const ()>,
}

impl MainChannel {
    /// Fill `event_input` and publish the count.
    ///
    /// `writer` receives a mutable reference to the full event buffer and must
    /// return the number of events it wrote.
    #[inline]
    pub fn write_and_publish<F>(&self, writer: F)
    where
        F: FnOnce(&mut [TriggerEvent; MAX_QUEUED_EVENTS]) -> u8,
    {
        self.inner.events.write_and_publish(writer);
    }

    /// Set master gain in `[0.0, 2.0]` (Relaxed - one-block lag is fine).
    #[inline(always)]
    pub fn set_master_gain(&self, gain: f32) {
        self.inner
            .master_gain_bits
            .store((gain.clamp(0.0, 2.0) * MASTER_HEADROOM).to_bits());
    }

    /// Get the current master gain.
    #[inline(always)]
    #[must_use]
    pub fn master_gain(&self) -> f32 {
        let bits = self.inner.master_gain_bits.load();
        f32::from_bits(bits) * INV_MASTER_HEADROOM
    }

    /// Signal the audio thread to start a hush fade.
    #[inline(always)]
    pub fn request_hush(&self) {
        self.inner.hush_pending.store(true);
    }

    /// Signal the audio thread to silence all voices immediately.
    #[inline(always)]
    pub fn request_panic(&self) {
        self.inner.panic_pending.store(true);
    }

    /// Signal the audio thread to discard the scheduled event queue.
    #[inline(always)]
    pub fn request_flush(&self) {
        self.inner.flush_pending.store(true);
    }

    /// Read the active voice count (Relaxed - one-block lag is fine for UI).
    #[inline(always)]
    pub fn voice_count(&self) -> u32 {
        self.inner.voice_count.load()
    }

    /// Read the current audio clock in seconds (Relaxed).
    #[inline(always)]
    pub fn audio_time(&self) -> f64 {
        f64::from_bits(self.inner.audio_time_bits.load())
    }

    /// Store multiple samples in a single atomic pointer swap.
    ///
    /// Prefer this over repeated [`MainChannel::store_sample`] calls when loading
    /// several samples at once - it performs one grow and one swap regardless of
    /// batch size, minimizing atomic overhead and allocation churn.
    pub fn store_samples_batch(
        &self,
        entries: Vec<(SlotId, Sample)>,
        trash: &mut Vec<RetiredBankEntry>,
    ) {
        if entries.is_empty() {
            return;
        }

        let max_idx = entries.iter().map(|(id, _)| id.get()).max().unwrap_or(0) as usize;
        let (old_ptr, old_len, mut new_vec) = self.clone_and_grow_sample_slice(max_idx + 1);
        for (slot_id, sample) in entries {
            new_vec[slot_id.get() as usize] = Some(sample);
        }

        self.publish_sample_slice(new_vec, old_ptr, old_len, trash);
    }

    /// Drop every retired allocation the audio thread has confirmed it is no longer reading.
    ///
    /// Call from the main thread's ~10 Hz tick (e.g. inside `query_events_packed`).
    pub fn collect_garbage(&self, trash: &mut Vec<RetiredBankEntry>) {
        let processed_gen = self.inner.samples_processed_gen.load();
        trash.retain(|entry| {
            if entry.generation <= processed_gen {
                let slice: *mut [Option<Sample>] =
                    ptr::slice_from_raw_parts_mut(entry.ptr, entry.len);
                // SAFETY:
                // - `entry.ptr` was produced by `Box::into_raw` on a `Box<[Option<Sample>]>`
                //   with `entry.len` elements.
                // - `entry.generation <= processed_gen` guarantees the audio thread has
                //   completed every render block that could read through this pointer; it
                //   will never be loaded again.
                unsafe {
                    drop(Box::from_raw(slice));
                }
                false
            } else {
                true
            }
        });
    }

    /// Clone the current sample slice into a new `Vec` for copy-on-write mutation
    /// before the next swap.
    fn clone_and_grow_sample_slice(
        &self,
        min_required_len: usize,
    ) -> (*mut Option<Sample>, usize, Vec<Option<Sample>>) {
        let old_ptr = self.inner.samples_ptr.load();
        let old_len = self.inner.samples_len.load();

        let target_len = if min_required_len > old_len {
            min_required_len.next_power_of_two()
        } else {
            old_len
        };

        let mut new_vec = Vec::with_capacity(target_len);

        if old_len > 0 {
            // SAFETY:
            // - `old_ptr` is Acquire-loaded from `samples_ptr`, establishing a
            //   happens-before edge with the Release swap that set it. The slice
            //   is therefore valid for `old_len` elements for the duration of this call.
            // - Only the main thread (this function) ever clones or replaces the slice,
            //   so there is no concurrent write to the allocation being read here.
            let current = unsafe { slice::from_raw_parts(old_ptr, old_len) };
            new_vec.extend_from_slice(current);
        }

        if target_len > old_len {
            new_vec.resize_with(target_len, || None);
        }

        (old_ptr, old_len, new_vec)
    }

    /// Publish a new sample slice with a Release swap and retire the old allocation.
    fn publish_sample_slice(
        &self,
        new_vec: Vec<Option<Sample>>,
        old_ptr: *mut Option<Sample>,
        old_len: usize,
        trash: &mut Vec<RetiredBankEntry>,
    ) {
        let new_len = new_vec.len();
        let new_boxed: Box<[Option<Sample>]> = new_vec.into_boxed_slice();
        let new_raw = Box::into_raw(new_boxed) as *mut Option<Sample>;

        // Write length with Release BEFORE swapping the pointer so the audio thread's
        // Acquire load of `samples_ptr` transitively sees the updated length via a
        // subsequent Relaxed load.
        self.inner.samples_len.store(new_len);
        self.inner.samples_ptr.swap(new_raw);

        // Bump the generation counter. Only the main thread writes this field, so a
        // Relaxed load followed by a Relaxed store is safe (no concurrent writer).
        let generation = self.inner.samples_current_gen.load().wrapping_add(1);
        self.inner.samples_current_gen.store(generation);

        // Retire the old allocation rather than dropping it immediately.
        // The sentinel static is not heap-allocated and must never be freed.
        let is_static = ptr::eq(old_ptr as *const (), &raw const EMPTY_SAMPLE as *const ());
        if !is_static && old_len > 0 {
            trash.push(RetiredBankEntry {
                ptr: old_ptr,
                len: old_len,
                generation,
            });
        }
    }
}

/// Channel handle for the **audio callback thread**.
///
/// Exposes only the operations the audio thread is permitted to perform.
///
/// # [`Send`], not [`Sync`]
///
/// `AudioChannel` is [`Send`] so it can be moved into the `FnMut + Send + 'static`
/// closure that cpal requires. It is deliberately **not** [`Sync`] - the audio
/// callback is always single-threaded, and sharing the handle between multiple
/// threads would violate the [`UnsafeCell`] invariant on `event_input`.
#[derive(Debug)]
pub(crate) struct AudioChannel {
    inner: Arc<ChannelInner>,
}

// SAFETY: AudioChannel is only ever accessed from the single audio callback
// thread. It is Send so it can be moved into the cpal closure, but it is
// not Sync (no concurrent access from multiple threads).
unsafe impl Send for AudioChannel {}

impl AudioChannel {
    /// Drain any pending events via a closure.
    #[inline]
    pub fn drain_and_process<F>(&self, f: F)
    where
        F: FnOnce(&[TriggerEvent]),
    {
        self.inner
            .events
            .drain_and_process(|slot, count| f(&slot[..count]));
    }

    /// Read master gain (Relaxed - set by main thread, one-block lag is fine).
    #[inline(always)]
    pub fn master_gain_f32(&self) -> f32 {
        f32::from_bits(self.inner.master_gain_bits.load())
    }

    /// Consume the hush flag if set (Relaxed peek -> Acquire swap).
    /// Fast path (flag clear) = one Relaxed load, no fence.
    #[inline(always)]
    pub fn take_hush(&self) -> bool {
        self.inner.hush_pending.swap_if_set()
    }

    /// Consume the panic flag if set.
    #[inline(always)]
    pub fn take_panic(&self) -> bool {
        self.inner.panic_pending.swap_if_set()
    }

    /// Consume the flush flag if set.
    #[inline(always)]
    pub fn take_flush(&self) -> bool {
        self.inner.flush_pending.swap_if_set()
    }

    /// Publish the active voice count (Relaxed - one-block lag is fine for UI).
    #[inline(always)]
    pub fn set_voice_count(&self, n: u32) {
        self.inner.voice_count.store(n);
    }

    /// Publish the current audio clock in seconds (Relaxed).
    #[inline(always)]
    pub fn set_audio_time(&self, t: f64) {
        self.inner.audio_time_bits.store(t.to_bits());
    }

    /// Acquire a stable `&[Option<Sample>]` view for the duration of one render block.
    ///
    /// Call this at the **start** of a render block (before draining events). The
    /// reference remains valid for the entire block; the generation handshake prevents
    /// the main thread from freeing the underlying allocation until after
    /// [`AudioChannel::mark_generation_processed`] is called at the end of the block.
    ///
    /// Single `Acquire` load. Zero allocation. Safe to call every block.
    fn samples_snapshot(&self) -> &[Option<Sample>] {
        let ptr = self.inner.samples_ptr.load();
        // The Acquire load of `samples_ptr` synchronizes with the main thread's Release
        // swap, which happened-after the Release store of `samples_len`. This chain
        // guarantees `samples_len` is visible via a subsequent Relaxed load.
        let len = self.inner.samples_len.load();
        if len == 0 {
            return &[];
        }
        // SAFETY:
        // - `ptr` was Acquire-loaded, pointing to a valid `Box<[Option<Sample>]>`
        //   allocation of `len` elements produced by the main thread.
        // - The generation handshake (`mark_generation_processed`) prevents the main
        //   thread from freeing this allocation until after this render block completes.
        unsafe { slice::from_raw_parts(ptr, len) }
    }

    /// Snapshot the current generation counter at the start of a render block.
    ///
    /// Pass the returned value to [`AudioChannel::mark_generation_processed`] at the
    /// end of the same block to complete the deferred-free handshake.
    #[inline(always)]
    fn samples_current_gen(&self) -> u32 {
        self.inner.samples_current_gen.load()
    }

    /// Signal the main thread that this render block has finished reading generation `gen`.
    ///
    /// Call at the **end** of every render block with the value returned by
    /// [`AudioChannel::samples_current_gen`] at the start of that block. The main
    /// thread's [`MainChannel::collect_garbage`] uses this to determine when it is safe
    /// to free retired sample allocations.
    #[inline(always)]
    fn mark_generation_processed(&self, generation: u32) {
        self.inner.samples_processed_gen.store(generation);
    }

    /// Process the current generation.
    #[inline]
    pub fn process_generation<F>(&self, reader: F)
    where
        F: FnOnce(&[Option<Sample>]),
    {
        // Capture the generation BEFORE loading the sample pointer so we never report a
        // generation as processed before we've actually finished reading through it.
        let generation = self.samples_current_gen();
        let samples = self.samples_snapshot();

        reader(samples);

        // Signal the main thread that we have finished reading this generation; it may
        // now free any retired allocation whose generation <= this value.
        self.mark_generation_processed(generation);
    }
}

#[cfg(test)]
mod tests {
    use strudel_dsp::sample::Sample;

    use super::*;

    fn make_sample() -> Sample {
        Sample::mono(Arc::from([0.5_f32; 64]), 44100.0)
    }

    #[test]
    fn test_initial_sample_slice_empty() {
        let (_main_ch, audio_ch) = channel(1.0);
        // Should return an empty slice without segfaulting - the sentinel pointer is valid.
        let snap = audio_ch.samples_snapshot();
        assert!(snap.is_empty());
    }

    #[test]
    fn test_store_sample_visible_via_snapshot() {
        let (main_ch, audio_ch) = channel(1.0);
        let mut trash = Vec::new();
        main_ch.store_samples_batch(vec![(SlotId::new(0), make_sample())], &mut trash);
        let snap = audio_ch.samples_snapshot();
        assert!(snap[0].is_some());
    }

    #[test]
    fn test_store_sample_grows_allocation() {
        let (main_ch, audio_ch) = channel(1.0);
        let mut trash = Vec::new();
        let target_slot = SlotId::new(7);

        main_ch.store_samples_batch(vec![(target_slot, make_sample())], &mut trash);
        let snap = audio_ch.samples_snapshot();

        let target_idx = target_slot.get() as usize;

        assert!(snap.len() >= 8);
        assert!(snap[target_idx].is_some());

        for (i, sample) in snap.iter().enumerate() {
            if i == target_idx {
                continue;
            } // Skip the slot we filled
            assert!(sample.is_none(), "slot {i} should be None");
        }
    }

    #[test]
    fn test_collect_garbage_after_generation_handshake() {
        let (main_ch, audio_ch) = channel(1.0);
        let mut trash = Vec::new();

        // First store - old ptr is the static sentinel, so nothing goes to trash.
        main_ch.store_samples_batch(vec![(SlotId::new(0), make_sample())], &mut trash);
        assert!(trash.is_empty(), "static sentinel must not be retired");

        // Second store - old heap allocation enters trash.
        main_ch.store_samples_batch(vec![(SlotId::new(1), make_sample())], &mut trash);
        assert_eq!(trash.len(), 1, "one retired bank in trash");

        // GC before the audio thread has confirmed: must not drop.
        main_ch.collect_garbage(&mut trash);
        assert_eq!(trash.len(), 1, "must not drop before audio thread confirms");

        // Simulate audio thread finishing: report the current generation.
        let generation = audio_ch.samples_current_gen();
        audio_ch.mark_generation_processed(generation);

        // GC after confirmation: retired allocation should be freed.
        main_ch.collect_garbage(&mut trash);
        assert!(trash.is_empty(), "safe to drop after handshake");
    }

    #[test]
    fn test_collect_garbage_frees_multi_element_retired_bank() {
        let (main_ch, audio_ch) = channel(1.0);
        let mut trash = Vec::new();

        // First store: 4 slots at once -> retired bank will have len >= 4 after next grow.
        let entries: Vec<_> = (0..4).map(|i| (SlotId::new(i), make_sample())).collect();
        main_ch.store_samples_batch(entries, &mut trash);
        assert!(trash.is_empty(), "static sentinel must not be retired");

        // Second store, forcing a growth: the first (len >= 4) buffer is retired.
        main_ch.store_samples_batch(vec![(SlotId::new(10), make_sample())], &mut trash);
        assert_eq!(trash.len(), 1);
        assert!(
            trash[0].len >= 4,
            "retired bank should have multiple elements"
        );

        let generation = audio_ch.samples_current_gen();
        audio_ch.mark_generation_processed(generation);

        main_ch.collect_garbage(&mut trash);
        assert!(trash.is_empty());
    }

    #[test]
    fn test_batch_store_samples() {
        let (main_ch, audio_ch) = channel(1.0);
        let mut trash = Vec::new();
        let entries: Vec<_> = (0..4).map(|i| (SlotId::new(i), make_sample())).collect();
        main_ch.store_samples_batch(entries, &mut trash);
        let snap = audio_ch.samples_snapshot();
        assert!(snap.len() >= 4);

        for (i, sample) in snap.iter().enumerate() {
            assert!(sample.is_some(), "slot {i} should be Some");
        }
    }
}
