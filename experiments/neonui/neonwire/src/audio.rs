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
    /// Underruns recovered via re-PREPARE (write is &self, hence atomic).
    xruns: AtomicU32,
}

impl Pcm {
    pub fn open() -> io::Result<Pcm> {
        let dev = std::ffi::CString::new(PCM_DEV).unwrap();
        // O_NONBLOCK: a busy pcm must fail EBUSY immediately — the kernel
        // otherwise parks the open() until the other stream closes, which
        // looked like an eternal "LOADING" hang. Cleared after open so
        // writes pace playback normally.
        let fd = unsafe { libc::open(dev.as_ptr(), libc::O_RDWR | libc::O_NONBLOCK) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        unsafe {
            let fl = libc::fcntl(fd, libc::F_GETFL);
            libc::fcntl(fd, libc::F_SETFL, fl & !libc::O_NONBLOCK);
        }
        let pcm = Pcm { fd, xruns: AtomicU32::new(0) };
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
                self.xruns.fetch_add(1, Ordering::Relaxed);
                self.prepare()?; // underrun: re-arm and retry
                continue;
            }
            return Err(e);
        }
        Ok(())
    }

    /// Underruns recovered since open.
    pub fn xruns(&self) -> u32 {
        self.xruns.load(Ordering::Relaxed)
    }
}

impl Drop for Pcm {
    fn drop(&mut self) {
        unsafe { libc::close(self.fd) };
    }
}

// ---- native ALSA control: /dev/snd/controlC0 ELEM_WRITE, no chroot ----
//
// The old amixer-in-Alpine-chroot shell-out silently no-opped whenever
// /mnt/data wasn't mounted (post-reboot: music played into a powered-off amp).
// The kernel resolves controls by name when numid==0, so one ioctl suffices.

const CTL_DEV: &str = "/dev/snd/controlC0";
const SNDRV_CTL_ELEM_IFACE_MIXER: u32 = 2;

/// The kernel's value union holds `long long[64]`, which is 8-byte aligned on
/// ARM EABI (unlike x86-32) — 4 pad bytes land after `indirect`.
#[repr(C, align(8))]
struct CtlValueUnion([u8; 512]); // enumerated = u32 item[128]

/// 3.18 snd_ctl_elem_value, arm32: id(64) + indirect(4) + pad(4) + union(512)
/// + timespec(8) + reserved(120) + tailpad(4) = 712 bytes.
#[repr(C)]
struct SndCtlElemValue {
    // snd_ctl_elem_id
    numid: u32,
    iface: u32,
    device: u32,
    subdevice: u32,
    name: [u8; 44],
    index: u32,
    indirect: u32,
    value: CtlValueUnion,
    tstamp: [libc::c_long; 2],
    reserved: [u8; 120],
}

// arm32 ABI: any drift here changes the ioctl number and the driver ENOTTYs.
// 712 verified against the device kernel with scratch ctltest.c (2026-07-20).
const _: () = assert!(std::mem::size_of::<SndCtlElemValue>() == 712);

fn ioctl_ctl_elem_write() -> libc::c_int {
    // _IOWR('U', 0x13, struct snd_ctl_elem_value)
    (3u32 << 30
        | (std::mem::size_of::<SndCtlElemValue>() as u32) << 16
        | (b'U' as u32) << 8
        | 0x13) as libc::c_int
}

pub(crate) fn ctl_set_enum(name: &str, item: u32) -> io::Result<()> {
    let dev = std::ffi::CString::new(CTL_DEV).unwrap();
    let fd = unsafe { libc::open(dev.as_ptr(), libc::O_RDWR) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut v: SndCtlElemValue = unsafe { std::mem::zeroed() };
    v.iface = SNDRV_CTL_ELEM_IFACE_MIXER;
    v.name[..name.len()].copy_from_slice(name.as_bytes());
    v.value.0[..4].copy_from_slice(&item.to_ne_bytes());
    let rc = unsafe { libc::ioctl(fd, ioctl_ctl_elem_write(), &mut v) };
    let err = io::Error::last_os_error();
    unsafe { libc::close(fd) };
    if rc < 0 {
        return Err(err);
    }
    Ok(())
}

/// Speaker amp on/off — the HAL-faithful bring-up (audio-recipe.md): ONE
/// control (`Speaker_Amp_Switch`) + PGA gain; DAPM powers the rest while
/// a stream is live on hw:0,5.
pub fn speaker_amp(on: bool) {
    if let Err(e) = ctl_set_enum("Speaker_Amp_Switch", on as u32) {
        eprintln!("mixer: Speaker_Amp_Switch {}: {e}", if on { "On" } else { "Off" });
    }
    if on {
        // enum items: MUTE,0Db,4Db,5Db,6Db,7Db,8Db,... -> '8Db' = index 6
        if let Err(e) = ctl_set_enum("Audio_Speaker_PGA_gain", 6) {
            eprintln!("mixer: Audio_Speaker_PGA_gain: {e}");
        }
    }
}

/// Built-in mic path (stock audio_device.xml `builtin_Mic_SingleMic`).
/// Bare arecord without this records near-silence; enum indices verified
/// against on-device amixer items (2026-07-21).
pub fn mic_arm() {
    // Audio_MIC1_Mode_Select: ACCMODE=0
    let _ = ctl_set_enum("Audio_MIC1_Mode_Select", 0);
    // Audio_MicSource1_Setting: ADC1=0
    let _ = ctl_set_enum("Audio_MicSource1_Setting", 0);
    // Audio_ADC_*_Sel: idle=0, AIN=1, Preamp=2
    let _ = ctl_set_enum("Audio_ADC_1_Sel", 2);
    let _ = ctl_set_enum("Audio_ADC_2_Sel", 2);
    // Audio_ADC_*_Switch: Off=0, On=1
    let _ = ctl_set_enum("Audio_ADC_1_Switch", 1);
    let _ = ctl_set_enum("Audio_ADC_2_Switch", 1);
    // Audio_Preamp*_Switch: OPEN=0, IN_ADC1=1, IN_ADC2=2, IN_ADC3=3
    let _ = ctl_set_enum("Audio_Preamp1_Switch", 1);
    let _ = ctl_set_enum("Audio_Preamp2_Switch", 3);
    // Audio_PGA*_Setting: -6,0,6,12,18,24 → 24Db=5 (weak MEMS; AGC cleans after)
    let _ = ctl_set_enum("Audio_PGA1_Setting", 5);
    let _ = ctl_set_enum("Audio_PGA2_Setting", 5);
    // Handset_PGA_GAIN: -21..9 step 2 → 9Db = index 15
    let _ = ctl_set_enum("Handset_PGA_GAIN", 15);
    // Voice_Amp_Switch: Off=0, On=1
    if let Err(e) = ctl_set_enum("Voice_Amp_Switch", 1) {
        eprintln!("mixer: Voice_Amp_Switch On: {e}");
    }
}

pub fn mic_disarm() {
    let _ = ctl_set_enum("Audio_Preamp1_Switch", 0);
    let _ = ctl_set_enum("Audio_Preamp2_Switch", 0);
    let _ = ctl_set_enum("Audio_ADC_1_Switch", 0);
    let _ = ctl_set_enum("Audio_ADC_2_Switch", 0);
    let _ = ctl_set_enum("Voice_Amp_Switch", 0);
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
    pub volume: AtomicU32, // master gain, 0..=256 (Q8)
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
    pub fn start(spec: PatternSpec, bpm: u32, volume: u32) -> Engine {
        let state = Arc::new(SeqState {
            spec: Mutex::new(spec),
            spec_gen: AtomicU32::new(0),
            bpm: AtomicU32::new(bpm),
            volume: AtomicU32::new(volume.min(256)),
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

/// One active drum voice: t = samples since trigger, lp = one-pole filter
/// state (HP by subtraction for hats/snare rattle, band color for clap).
struct Voice {
    track: usize,
    t: u32,
    lp: f32,
}

fn voice_sample(v: &mut Voice, noise: &mut Noise) -> f32 {
    const TAU: f32 = 2.0 * std::f32::consts::PI;
    let ts = v.t as f32 / RATE as f32;
    match v.track {
        // BD: 160->48 Hz sweep with ANALYTIC phase — φ = 2π∫f dt, not 2π·f(t)·t
        // (the old form warps the sweep as f changes). Click transient + soft
        // clip for punch, ~250 ms boom.
        0 => {
            let tc = 0.028; // sweep time-constant, f = 48 + 112·e^(−t/tc)
            let ph = TAU * (48.0 * ts + 112.0 * tc * (1.0 - (-ts / tc).exp()));
            let body = ph.sin() * (-ts * 9.0).exp();
            let click = 0.5 * noise.next() * (-ts * 400.0).exp();
            let x = 1.05 * body + click;
            x / (1.0 + 0.6 * x.abs())
        }
        // SD: two body modes (176 + 236 Hz) decaying fast under a highpassed
        // noise rattle that rings longer — tone thwack, noise snap.
        1 => {
            let body = ((TAU * 176.0 * ts).sin() + 0.6 * (TAU * 236.0 * ts).sin())
                * (-ts * 28.0).exp();
            let n = noise.next();
            v.lp += 0.25 * (n - v.lp);
            let rattle = (n - v.lp) * (-ts * 12.0).exp();
            0.30 * body + 0.75 * rattle
        }
        // HH: highpassed noise with a 6.3 kHz metallic shimmer, tight decay.
        2 => {
            let n = noise.next();
            v.lp += 0.10 * (n - v.lp);
            let ring = 1.0 + 0.4 * (TAU * 6300.0 * ts).sin();
            0.55 * (n - v.lp) * ring * (-ts * 55.0).exp()
        }
        // CP: three retriggered bursts 11 ms apart, then a looser tail;
        // band-colored noise (HP of a slower LP) for the "slap".
        _ => {
            let n = noise.next();
            v.lp += 0.35 * (n - v.lp);
            let env = if ts < 0.033 {
                (-(ts % 0.011) * 220.0).exp()
            } else {
                0.6 * (-(ts - 0.033) * 16.0).exp()
            };
            0.85 * (n - v.lp) * env
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

        let vol = s.volume.load(Ordering::Relaxed).min(256) as f32 / 256.0;
        let mut tg = triggers.iter().peekable();
        for fi in 0..PERIOD {
            while tg.peek().is_some_and(|(off, _)| *off == fi) {
                let (_, track) = *tg.next().unwrap();
                voices.push(Voice { track, t: 0, lp: 0.0 });
            }
            let mut acc = 0.0f32;
            for v in voices.iter_mut() {
                acc += voice_sample(v, &mut noise);
                v.t += 1;
            }
            voices.retain(|v| v.t < RATE / 3); // 330 ms voice lifetime
            let smp = (acc.clamp(-1.0, 1.0) * 24000.0 * vol) as i16;
            buf[fi * 2] = smp;
            buf[fi * 2 + 1] = smp;
        }
        let _ = pcm.write(&buf);
    }
}
