//! Audio output without libasound: direct /dev/snd/pcmC0D5p ioctls
//! (tinyalsa-style), plus the drum-voice engine feeding it from a thread.
//!
//! Struct layouts are copied from THIS kernel's uapi/sound/asound.h
//! (reference/upstream/kernel_amazon_mt8127-common, 3.18) — the structs grew
//! fields in later kernels, so modern headers must not be used. On 32-bit ARM
//! snd_pcm_uframes_t = unsigned long = u32.
//!
//! Device contract (verified in B1, experiments/audio/audio-recipe.md):
//! hw:0,5 = S16_LE / 2ch / 44100, period 2048 frames, buffer 4096 HARD MAX.
//! Underrun => EPIPE on WRITEI; recover with PREPARE and continue.

use std::io;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

pub const RATE: u32 = 44100;
pub const CHANNELS: usize = 2;
pub const PERIOD: usize = 2048; // frames
const PCM_DEV: &str = "/dev/snd/pcmC0D5p";

// ---- 3.18 uapi/sound/asound.h ABI ----

const SNDRV_PCM_HW_PARAM_ACCESS: usize = 0;
const SNDRV_PCM_HW_PARAM_FORMAT: usize = 1;
const SNDRV_PCM_HW_PARAM_SUBFORMAT: usize = 2;
const SNDRV_PCM_HW_PARAM_FIRST_INTERVAL: usize = 8; // SAMPLE_BITS
const SNDRV_PCM_HW_PARAM_CHANNELS: usize = 10;
const SNDRV_PCM_HW_PARAM_RATE: usize = 11;
const SNDRV_PCM_HW_PARAM_PERIOD_SIZE: usize = 13;
const SNDRV_PCM_HW_PARAM_PERIODS: usize = 15;

const SNDRV_PCM_ACCESS_RW_INTERLEAVED: u32 = 3;
const SNDRV_PCM_FORMAT_S16_LE: u32 = 2;
const SNDRV_PCM_SUBFORMAT_STD: u32 = 0;

#[repr(C)]
#[derive(Clone, Copy)]
struct SndMask {
    bits: [u32; 8], // SNDRV_MASK_MAX 256 / 32
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct SndInterval {
    min: u32,
    max: u32,
    flags: u32, // openmin/openmax/integer/empty bitfield
}

#[repr(C)]
struct SndPcmHwParams {
    flags: u32,
    masks: [SndMask; 3],
    mres: [SndMask; 5],
    intervals: [SndInterval; 12], // SAMPLE_BITS..=TICK_TIME (8..=19)
    ires: [SndInterval; 9],
    rmask: u32,
    cmask: u32,
    info: u32,
    msbits: u32,
    rate_num: u32,
    rate_den: u32,
    fifo_size: u32, // snd_pcm_uframes_t (ulong, 4 bytes on arm32)
    reserved: [u8; 64],
}

#[repr(C)]
struct SndPcmSwParams {
    tstamp_mode: i32,
    period_step: u32,
    sleep_min: u32,
    avail_min: u32,
    xfer_align: u32,
    start_threshold: u32,
    stop_threshold: u32,
    silence_threshold: u32,
    silence_size: u32,
    boundary: u32,
    reserved: [u8; 64],
}

#[repr(C)]
struct SndXferI {
    result: i32, // snd_pcm_sframes_t
    buf: *const libc::c_void,
    frames: u32, // snd_pcm_uframes_t
}

fn iowr(dir: u32, nr: u32, size: usize) -> libc::c_int {
    (dir << 30 | (size as u32) << 16 | (b'A' as u32) << 8 | nr) as libc::c_int
}

fn ioctl_hw_params() -> libc::c_int {
    iowr(3, 0x11, std::mem::size_of::<SndPcmHwParams>())
}
fn ioctl_sw_params() -> libc::c_int {
    iowr(3, 0x13, std::mem::size_of::<SndPcmSwParams>())
}
fn ioctl_prepare() -> libc::c_int {
    iowr(0, 0x40, 0)
}
fn ioctl_writei() -> libc::c_int {
    iowr(1, 0x50, std::mem::size_of::<SndXferI>())
}

pub struct Pcm {
    fd: libc::c_int,
}

impl Pcm {
    pub fn open() -> io::Result<Pcm> {
        let dev = std::ffi::CString::new(PCM_DEV).unwrap();
        let fd = unsafe { libc::open(dev.as_ptr(), libc::O_RDWR) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let pcm = Pcm { fd };
        pcm.configure()?;
        Ok(pcm)
    }

    fn configure(&self) -> io::Result<()> {
        // tinyalsa approach: start fully open (all mask bits set, intervals
        // full-range), then pin what we need; the kernel refines the rest.
        let mut hw: SndPcmHwParams = unsafe { std::mem::zeroed() };
        for m in hw.masks.iter_mut().chain(hw.mres.iter_mut()) {
            m.bits = [!0u32; 8];
        }
        for it in hw.intervals.iter_mut().chain(hw.ires.iter_mut()) {
            it.min = 0;
            it.max = !0u32;
        }
        hw.rmask = !0;

        let set_mask = |hw: &mut SndPcmHwParams, param: usize, val: u32| {
            hw.masks[param].bits = [0; 8];
            hw.masks[param].bits[(val / 32) as usize] = 1 << (val % 32);
        };
        let set_int = |hw: &mut SndPcmHwParams, param: usize, val: u32| {
            let it = &mut hw.intervals[param - SNDRV_PCM_HW_PARAM_FIRST_INTERVAL];
            it.min = val;
            it.max = val;
            it.flags = 0b100; // integer
        };
        set_mask(&mut hw, SNDRV_PCM_HW_PARAM_ACCESS, SNDRV_PCM_ACCESS_RW_INTERLEAVED);
        set_mask(&mut hw, SNDRV_PCM_HW_PARAM_FORMAT, SNDRV_PCM_FORMAT_S16_LE);
        set_mask(&mut hw, SNDRV_PCM_HW_PARAM_SUBFORMAT, SNDRV_PCM_SUBFORMAT_STD);
        set_int(&mut hw, SNDRV_PCM_HW_PARAM_CHANNELS, CHANNELS as u32);
        set_int(&mut hw, SNDRV_PCM_HW_PARAM_RATE, RATE);
        set_int(&mut hw, SNDRV_PCM_HW_PARAM_PERIOD_SIZE, PERIOD as u32);
        set_int(&mut hw, SNDRV_PCM_HW_PARAM_PERIODS, 2);

        if unsafe { libc::ioctl(self.fd, ioctl_hw_params(), &mut hw) } < 0 {
            return Err(io::Error::last_os_error());
        }

        let buffer = (PERIOD * 2) as u32;
        let mut boundary = buffer;
        while boundary < 0x4000_0000 {
            boundary *= 2;
        }
        let mut sw: SndPcmSwParams = unsafe { std::mem::zeroed() };
        sw.avail_min = PERIOD as u32;
        sw.start_threshold = buffer;
        sw.stop_threshold = boundary; // never auto-stop; we handle xruns
        sw.boundary = boundary;
        if unsafe { libc::ioctl(self.fd, ioctl_sw_params(), &mut sw) } < 0 {
            return Err(io::Error::last_os_error());
        }
        self.prepare()
    }

    fn prepare(&self) -> io::Result<()> {
        if unsafe { libc::ioctl(self.fd, ioctl_prepare()) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Blocking interleaved write of PERIOD frames; recovers from underrun.
    pub fn write(&self, frames: &[i16]) -> io::Result<()> {
        let mut xfer = SndXferI {
            result: 0,
            buf: frames.as_ptr() as *const libc::c_void,
            frames: (frames.len() / CHANNELS) as u32,
        };
        for _ in 0..3 {
            if unsafe { libc::ioctl(self.fd, ioctl_writei(), &mut xfer) } >= 0 {
                return Ok(());
            }
            let e = io::Error::last_os_error();
            if e.raw_os_error() == Some(libc::EPIPE) {
                self.prepare()?; // underrun: re-arm and retry
                continue;
            }
            return Err(e);
        }
        Ok(())
    }
}

impl Drop for Pcm {
    fn drop(&mut self) {
        unsafe { libc::close(self.fd) };
    }
}

/// Speaker amp on/off via the Alpine chroot's amixer (B1 recipe). Native
/// controlC0 ELEM ioctls can replace this later; this is honest and works.
pub fn speaker_amp(on: bool) {
    let cmd = format!(
        "amixer -c0 sset Speaker_Amp_Switch {} >/dev/null 2>&1; \
         amixer -c0 sset Audio_Speaker_PGA_gain 8Db >/dev/null 2>&1",
        if on { "On" } else { "Off" }
    );
    let _ = std::process::Command::new("/bin/sh")
        .args(["/mnt/data/alpine-enter.sh", "-c", &cmd])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

// ---- sequencer audio engine (strudel-core pattern scheduler, B3) ----

use strudel_core::{Fraction, Pattern};

pub const STEPS: usize = 16;
pub const TRACKS: usize = 4;
pub const TRACK_SOUNDS: [&str; TRACKS] = ["bd", "sd", "hh", "cp"];

/// Map a strudel sound name onto one of our synthesized drum voices.
fn sound_to_track(name: &str) -> Option<usize> {
    match name {
        "bd" | "sbd" | "kick" => Some(0),
        "sd" | "snare" | "rim" => Some(1),
        "hh" | "oh" | "hat" => Some(2),
        "cp" | "clap" => Some(3),
        _ => None,
    }
}

/// Build a Pattern from the 16-step grid: per track a fastcat of pure/silence,
/// stacked — the grid is just another strudel pattern.
pub fn grid_pattern(grid: &[[bool; STEPS]; TRACKS]) -> Pattern {
    let tracks: Vec<Pattern> = grid
        .iter()
        .enumerate()
        .map(|(t, row)| {
            let cells: Vec<Pattern> = row
                .iter()
                .map(|&on| {
                    if on {
                        strudel_core::pure(TRACK_SOUNDS[t].into())
                    } else {
                        strudel_core::silence()
                    }
                })
                .collect();
            strudel_core::fastcat(cells)
        })
        .collect();
    strudel_core::stack(tracks)
}

/// What to play — Send-able; the Pattern itself is built on the audio thread
/// (strudel Pattern holds Rc and cannot cross threads).
#[derive(Clone)]
pub enum PatternSpec {
    Grid([[bool; STEPS]; TRACKS]),
    Mini(String),
}

impl PatternSpec {
    fn build(&self) -> Pattern {
        match self {
            PatternSpec::Grid(g) => grid_pattern(g),
            PatternSpec::Mini(src) => strudel_mini::eval_str(src, 0)
                .unwrap_or_else(|_| strudel_core::silence()),
        }
    }
}

/// Shared state between the UI (writer) and the audio thread (reader).
pub struct SeqState {
    pub spec: Mutex<PatternSpec>,
    pub spec_gen: AtomicU32, // bump after changing spec
    pub bpm: AtomicU32,
    pub playing: AtomicBool,
    pub playhead: AtomicUsize, // 16th-note position within cycle, for the UI
    pub online: AtomicBool,    // pcm open succeeded
    quit: AtomicBool,
}

pub struct Engine {
    pub state: Arc<SeqState>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Engine {
    pub fn start(spec: PatternSpec, bpm: u32) -> Engine {
        let state = Arc::new(SeqState {
            spec: Mutex::new(spec),
            spec_gen: AtomicU32::new(0),
            bpm: AtomicU32::new(bpm),
            playing: AtomicBool::new(false),
            playhead: AtomicUsize::new(0),
            online: AtomicBool::new(false),
            quit: AtomicBool::new(false),
        });
        let s = state.clone();
        let thread = std::thread::spawn(move || audio_thread(s));
        Engine { state, thread: Some(thread) }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.state.quit.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        speaker_amp(false);
    }
}

/// xorshift32 noise source (no rand dep needed for drums).
struct Noise(u32);
impl Noise {
    fn next(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

/// One active drum voice: t = samples since trigger.
struct Voice {
    track: usize,
    t: u32,
}

fn voice_sample(track: usize, t: u32, noise: &mut Noise) -> f32 {
    let ts = t as f32 / RATE as f32;
    match track {
        // BD: 150->50 Hz sine sweep, exp decay ~130 ms
        0 => {
            let f = 50.0 + 100.0 * (-ts * 18.0).exp();
            let ph = 2.0 * std::f32::consts::PI * f * ts;
            0.9 * ph.sin() * (-ts * 16.0).exp()
        }
        // SD: 190 Hz tone + noise, decay ~160 ms
        1 => {
            let tone = (2.0 * std::f32::consts::PI * 190.0 * ts).sin();
            (0.35 * tone + 0.55 * noise.next()) * (-ts * 14.0).exp()
        }
        // HH: bright noise (differentiated), very short
        2 => 0.5 * (noise.next() - noise.next() * 0.5) * (-ts * 60.0).exp(),
        // CP: three noise bursts
        _ => {
            let burst = ((ts * 90.0) as u32 % 3) as f32;
            0.6 * noise.next() * (-((ts - burst * 0.011).max(0.0)) * 30.0).exp() * (-ts * 9.0).exp()
        }
    }
}

fn audio_thread(s: Arc<SeqState>) {
    let pcm = match Pcm::open() {
        Ok(p) => {
            s.online.store(true, Ordering::Relaxed);
            p
        }
        Err(e) => {
            eprintln!("pcm: {e}");
            return;
        }
    };
    let mut buf = vec![0i16; PERIOD * CHANNELS];
    let mut voices: Vec<Voice> = Vec::new();
    let mut noise = Noise(0x1234_5678);
    let mut cycle_pos: f64 = 0.0; // strudel time, in cycles (1 cycle = 1 bar)
    let mut was_playing = false;
    // (frame offset within block, track) triggers for the current block
    let mut triggers: Vec<(usize, usize)> = Vec::new();
    // thread-local pattern, rebuilt when the UI bumps spec_gen
    let mut pattern: Pattern = s.spec.lock().unwrap().build();
    let mut last_gen = s.spec_gen.load(Ordering::Acquire);

    while !s.quit.load(Ordering::Relaxed) {
        let playing = s.playing.load(Ordering::Relaxed);
        if playing && !was_playing {
            cycle_pos = 0.0;
            s.playhead.store(0, Ordering::Relaxed);
        }
        was_playing = playing;
        if !playing && voices.is_empty() {
            // keep the stream fed with silence so restart is glitch-free
            buf.fill(0);
            let _ = pcm.write(&buf);
            continue;
        }

        let gen = s.spec_gen.load(Ordering::Acquire);
        if gen != last_gen {
            last_gen = gen;
            pattern = s.spec.lock().unwrap().build();
        }

        triggers.clear();
        if playing {
            // 4 beats per cycle: cps = bpm / 240 (120 BPM -> 0.5 cps, strudel default)
            let bpm = s.bpm.load(Ordering::Relaxed).clamp(40, 300) as f64;
            let cps = bpm / 240.0;
            let block_cycles = PERIOD as f64 / RATE as f64 * cps;
            let (from, to) = (cycle_pos, cycle_pos + block_cycles);
            let haps = pattern.query_arc(Fraction::from_float(from), Fraction::from_float(to));
            for hap in &haps {
                if !hap.has_onset() {
                    continue; // continuation of a hap that started earlier
                }
                let Some(track) = hap.value.as_string().and_then(sound_to_track) else {
                    continue;
                };
                let onset = hap.whole_or_part().begin.to_f64();
                let off = ((onset - from) / block_cycles * PERIOD as f64).max(0.0) as usize;
                triggers.push((off.min(PERIOD - 1), track));
            }
            triggers.sort_unstable();
            cycle_pos = to;
            let step = ((to.fract() * STEPS as f64) as usize).min(STEPS - 1);
            s.playhead.store(step, Ordering::Relaxed);
        }

        let mut tg = triggers.iter().peekable();
        for fi in 0..PERIOD {
            while tg.peek().is_some_and(|(off, _)| *off == fi) {
                let (_, track) = *tg.next().unwrap();
                voices.push(Voice { track, t: 0 });
            }
            let mut acc = 0.0f32;
            for v in voices.iter_mut() {
                acc += voice_sample(v.track, v.t, &mut noise);
                v.t += 1;
            }
            voices.retain(|v| v.t < RATE / 3); // 330 ms voice lifetime
            let smp = (acc.clamp(-1.0, 1.0) * 24000.0) as i16;
            buf[fi * 2] = smp;
            buf[fi * 2 + 1] = smp;
        }
        let _ = pcm.write(&buf);
    }
}
