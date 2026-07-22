//! In-process voice: mic capture + xAI STT + wake-word helpers.
//!
//! Replaces the Alpine/shell stack (arecord/curl/voice-stt.sh). Capture uses
//! the same 3.18 PCM ioctl ABI as `audio.rs` playback; mixer via `mic_arm()`.

use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::audio;

/// Serialize capture — wake scan + TALK must not open pcmC0D1c together (EBUSY → silence).
static CAP_LOCK: Mutex<()> = Mutex::new(());

const CAP_DEV: &str = "/dev/snd/pcmC0D1c"; // MultiMedia1_Capture
const CAP_RATE: u32 = 16_000;
const CAP_CH: usize = 1;
const CAP_PERIOD: usize = 1024;
const XAI_KEY_PATH: &str = "/mnt/sd/linux-lab/xai.key";
const STT_URL: &str = "https://api.x.ai/v1/stt";

// ---- PCM capture (mirror of audio::Pcm write path) ----

const SNDRV_PCM_HW_PARAM_ACCESS: usize = 0;
const SNDRV_PCM_HW_PARAM_FORMAT: usize = 1;
const SNDRV_PCM_HW_PARAM_SUBFORMAT: usize = 2;
const SNDRV_PCM_HW_PARAM_FIRST_INTERVAL: usize = 8;
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
    bits: [u32; 8],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct SndInterval {
    min: u32,
    max: u32,
    flags: u32,
}

#[repr(C)]
struct SndPcmHwParams {
    flags: u32,
    masks: [SndMask; 3],
    mres: [SndMask; 5],
    intervals: [SndInterval; 12],
    ires: [SndInterval; 9],
    rmask: u32,
    cmask: u32,
    info: u32,
    msbits: u32,
    rate_num: u32,
    rate_den: u32,
    fifo_size: u32,
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
    result: i32,
    buf: *mut libc::c_void,
    frames: u32,
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
fn ioctl_start() -> libc::c_int {
    iowr(0, 0x42, 0)
}
fn ioctl_readi() -> libc::c_int {
    // _IOR('A', 0x51, struct snd_xferi) — NOT _IOWR (that ENOTTYs on this 3.18).
    iowr(2, 0x51, std::mem::size_of::<SndXferI>())
}

struct CapPcm {
    fd: libc::c_int,
}

impl CapPcm {
    fn open(rate: u32, ch: u32, period: u32) -> io::Result<CapPcm> {
        let dev = std::ffi::CString::new(CAP_DEV).unwrap();
        // Capture nodes often want RDWR (tinyalsa); try RDWR then RDONLY.
        let mut fd = unsafe { libc::open(dev.as_ptr(), libc::O_RDWR | libc::O_NONBLOCK) };
        if fd < 0 {
            fd = unsafe { libc::open(dev.as_ptr(), libc::O_RDONLY | libc::O_NONBLOCK) };
        }
        if fd < 0 {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!("open {CAP_DEV}: {}", io::Error::last_os_error()),
            ));
        }
        unsafe {
            let fl = libc::fcntl(fd, libc::F_GETFL);
            libc::fcntl(fd, libc::F_SETFL, fl & !libc::O_NONBLOCK);
        }
        let p = CapPcm { fd };
        p.configure(rate, ch, period)?;
        Ok(p)
    }

    fn configure(&self, rate: u32, ch: u32, period: u32) -> io::Result<()> {
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
            it.flags = 0b100;
        };
        set_mask(&mut hw, SNDRV_PCM_HW_PARAM_ACCESS, SNDRV_PCM_ACCESS_RW_INTERLEAVED);
        set_mask(&mut hw, SNDRV_PCM_HW_PARAM_FORMAT, SNDRV_PCM_FORMAT_S16_LE);
        set_mask(&mut hw, SNDRV_PCM_HW_PARAM_SUBFORMAT, SNDRV_PCM_SUBFORMAT_STD);
        set_int(&mut hw, SNDRV_PCM_HW_PARAM_CHANNELS, ch);
        set_int(&mut hw, SNDRV_PCM_HW_PARAM_RATE, rate);
        set_int(&mut hw, SNDRV_PCM_HW_PARAM_PERIOD_SIZE, period);
        set_int(&mut hw, SNDRV_PCM_HW_PARAM_PERIODS, 4);

        if unsafe { libc::ioctl(self.fd, ioctl_hw_params(), &mut hw) } < 0 {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!("HW_PARAMS: {}", io::Error::last_os_error()),
            ));
        }
        let buffer = period * 4;
        let mut boundary = buffer;
        while boundary < 0x4000_0000 {
            boundary *= 2;
        }
        let mut sw: SndPcmSwParams = unsafe { std::mem::zeroed() };
        sw.avail_min = period;
        sw.start_threshold = 1;
        sw.stop_threshold = boundary;
        sw.boundary = boundary;
        if unsafe { libc::ioctl(self.fd, ioctl_sw_params(), &mut sw) } < 0 {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!("SW_PARAMS: {}", io::Error::last_os_error()),
            ));
        }
        if unsafe { libc::ioctl(self.fd, ioctl_prepare()) } < 0 {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!("PREPARE: {}", io::Error::last_os_error()),
            ));
        }
        // Some devices auto-start on first READI; START is best-effort.
        let _ = unsafe { libc::ioctl(self.fd, ioctl_start()) };
        Ok(())
    }

    fn read_period(&self, buf: &mut [i16], ch: usize) -> io::Result<usize> {
        let mut xfer = SndXferI {
            result: 0,
            buf: buf.as_mut_ptr() as *mut libc::c_void,
            frames: (buf.len() / ch.max(1)) as u32,
        };
        for _ in 0..5 {
            let rc = unsafe { libc::ioctl(self.fd, ioctl_readi(), &mut xfer) };
            if rc >= 0 {
                return Ok(xfer.result.max(0) as usize);
            }
            let e = io::Error::last_os_error();
            let en = e.raw_os_error();
            if en == Some(libc::EPIPE) || en == Some(libc::ESTRPIPE) || en == Some(libc::EAGAIN) {
                let _ = unsafe { libc::ioctl(self.fd, ioctl_prepare()) };
                let _ = unsafe { libc::ioctl(self.fd, ioctl_start()) };
                continue;
            }
            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!("READI: {e}"),
            ));
        }
        Ok(0)
    }
}

impl Drop for CapPcm {
    fn drop(&mut self) {
        unsafe { libc::close(self.fd) };
    }
}

/// Samples to discard after ADC power-on (turn-on pop saturates the first few 100ms).
const CAP_DROP_MS: u32 = 400;
/// How long after PCM starts before we re-toggle ADC (lets hw_params set UL rate).
const CAP_ADC_ARM_MS: u64 = 80;

/// Record mono 16 kHz S16LE for `secs`.
///
/// **Critical on MT6323/MT8127:** `TurnOnADcPower*` latches the UL sample rate
/// from the *currently open* PCM. Arming ADC *before* open (or leaving it on
/// from a prior capture) yields digital zeros at 16 kHz. Sequence that works:
/// mic path config → ADC Off → open PCM → ADC On → drop first ~400 ms click.
pub fn record_pcm(secs: u32) -> io::Result<(Vec<i16>, i16)> {
    let _guard = CAP_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    // Path / gain (not ADC power).
    audio::mic_arm();
    mic_arm_amixer_path();
    // Force cold ADC so the next On runs TurnOnADcPower with this stream's rate.
    mic_adc(false);

    // Native first: we can open PCM then arm ADC in-process (no race).
    match record_pcm_native_live_arm(secs, CAP_RATE, CAP_CH as u32, CAP_PERIOD as u32) {
        Ok((s, p)) if p > 40 => {
            eprintln!("voice: native peak={p} samples={}", s.len());
            return Ok((s, p));
        }
        Ok((s, p)) => eprintln!("voice: native quiet peak={p} samples={}", s.len()),
        Err(e) => eprintln!("voice: native capture failed ({e}); trying arecord"),
    }

    // arecord fallback with delayed ADC On.
    mic_adc(false);
    match record_pcm_arecord_live_arm(secs) {
        Ok((s, p)) => {
            eprintln!("voice: arecord peak={p} samples={}", s.len());
            Ok((s, p))
        }
        Err(e) => {
            eprintln!("voice: arecord failed ({e})");
            Err(e)
        }
    }
}

fn ensure_alpine_dev() {
    let alp = "/mnt/data/alpine";
    for d in ["proc", "sys", "dev"] {
        let path = format!("{alp}/{d}");
        let _ = std::process::Command::new("mount")
            .args(["--bind", &format!("/{d}"), &path])
            .status();
    }
}

fn amixer_sset(name: &str, val: &str) {
    ensure_alpine_dev();
    let _ = std::process::Command::new("chroot")
        .args([
            "/mnt/data/alpine",
            "/usr/bin/amixer",
            "-c0",
            "sset",
            name,
            val,
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

/// Mixer path without powering ADC (rate latch happens on ADC On).
fn mic_arm_amixer_path() {
    let sets = [
        ("Audio_MIC1_Mode_Select", "ACCMODE"),
        ("Audio_MicSource1_Setting", "ADC1"),
        ("Audio_ADC_1_Sel", "Preamp"),
        ("Audio_ADC_2_Sel", "Preamp"),
        ("Audio_Preamp1_Switch", "IN_ADC1"),
        ("Audio_Preamp2_Switch", "IN_ADC3"),
        ("Audio_PGA1_Setting", "24Db"),
        ("Audio_PGA2_Setting", "24Db"),
        ("Handset_PGA_GAIN", "9Db"),
        ("Voice_Amp_Switch", "On"),
    ];
    for (name, val) in sets {
        amixer_sset(name, val);
    }
}

/// ADC power On/Off — dual native ctl + amixer so DAPM actually runs.
fn mic_adc(on: bool) {
    let item = on as u32;
    let _ = audio::ctl_set_enum("Audio_ADC_1_Switch", item);
    let _ = audio::ctl_set_enum("Audio_ADC_2_Switch", item);
    if on {
        let _ = audio::ctl_set_enum("Audio_Preamp1_Switch", 1); // IN_ADC1
        let _ = audio::ctl_set_enum("Audio_Preamp2_Switch", 3); // IN_ADC3
    }
    amixer_sset("Audio_ADC_1_Switch", if on { "On" } else { "Off" });
    amixer_sset("Audio_ADC_2_Switch", if on { "On" } else { "Off" });
    if on {
        amixer_sset("Audio_Preamp1_Switch", "IN_ADC1");
        amixer_sset("Audio_Preamp2_Switch", "IN_ADC3");
    }
}

/// Remove DC and soft-clip AGC to target peak (MEMS on this board ~500–1500 raw).
fn agc_inplace(samples: &mut [i16], target: i16) -> i16 {
    if samples.is_empty() {
        return 0;
    }
    // DC remove — otherwise AGC latches on bias and speech stays buried.
    let sum: i64 = samples.iter().map(|&s| s as i64).sum();
    let mean = (sum / samples.len() as i64) as i16;
    if mean.abs() > 2 {
        for s in samples.iter_mut() {
            *s = s.saturating_sub(mean);
        }
    }
    let peak = samples
        .iter()
        .map(|s| s.saturating_abs())
        .max()
        .unwrap_or(0);
    if peak < 30 {
        return peak;
    }
    let gain = (target as f32 / peak as f32).clamp(1.0, 64.0);
    if gain <= 1.05 {
        return peak;
    }
    let mut new_peak = 0i16;
    for s in samples.iter_mut() {
        let v = (*s as f32 * gain).round().clamp(-32767.0, 32767.0) as i16;
        let a = v.saturating_abs();
        if a > new_peak {
            new_peak = a;
        }
        *s = v;
    }
    eprintln!("voice: AGC x{gain:.1} peak {peak} → {new_peak} (dc={mean})");
    new_peak
}

/// Native capture: open PCM first, then ADC On (rate latch), drop click, collect.
fn record_pcm_native_live_arm(
    secs: u32,
    rate: u32,
    ch: u32,
    period: u32,
) -> io::Result<(Vec<i16>, i16)> {
    let pcm = CapPcm::open(rate, ch, period)?;
    // Stream is live — now power ADC so TurnOnADcPower sees mBlockSampleRate.
    std::thread::sleep(Duration::from_millis(CAP_ADC_ARM_MS));
    mic_adc(true);

    let drop_n = (rate as usize) * CAP_DROP_MS as usize / 1000 * ch as usize;
    let total = (rate as usize) * secs as usize * ch as usize;
    let mut out = Vec::with_capacity(total);
    let mut period_buf = vec![0i16; period as usize * ch as usize];
    let mut dropped = 0usize;
    let mut peak: i16 = 0;
    let deadline =
        std::time::Instant::now() + Duration::from_secs(secs as u64 + 2) + Duration::from_millis(CAP_DROP_MS as u64);
    while out.len() < total {
        if std::time::Instant::now() > deadline {
            break;
        }
        let n = pcm.read_period(&mut period_buf, ch as usize)?;
        if n == 0 {
            std::thread::sleep(Duration::from_millis(5));
            continue;
        }
        let slice = &period_buf[..n * ch as usize];
        if dropped < drop_n {
            let take = (drop_n - dropped).min(slice.len());
            dropped += take;
            if take < slice.len() {
                // remainder of this period is keep
                let keep = &slice[take..];
                for &s in keep {
                    let a = s.saturating_abs();
                    if a > peak {
                        peak = a;
                    }
                }
                out.extend_from_slice(keep);
            }
            continue;
        }
        for &s in slice {
            let a = s.saturating_abs();
            if a > peak {
                peak = a;
            }
        }
        out.extend_from_slice(slice);
        if out.len() > total {
            out.truncate(total);
        }
    }
    if out.is_empty() {
        return Err(io::Error::new(io::ErrorKind::Other, "no frames read"));
    }
    Ok((out, peak))
}

/// arecord: ADC Off → start process → delayed ADC On → trim click.
fn record_pcm_arecord_live_arm(secs: u32) -> io::Result<(Vec<i16>, i16)> {
    ensure_alpine_dev();
    let path = "/tmp/voice-native-fallback.wav";
    let alp = "/mnt/data/alpine";
    let _ = std::fs::create_dir_all(format!("{alp}/tmp"));
    // record a bit longer so after drop we still have `secs`
    let rec_secs = secs.saturating_add(1).max(2);
    let mut child = std::process::Command::new("chroot")
        .args([
            alp,
            "/usr/bin/arecord",
            "-D",
            "hw:0,1",
            "-f",
            "S16_LE",
            "-c1",
            "-r16000",
            "-d",
            &rec_secs.to_string(),
            path,
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    std::thread::sleep(Duration::from_millis(CAP_ADC_ARM_MS));
    mic_adc(true);
    let status = child.wait()?;
    if !status.success() {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            format!("arecord exit {status}"),
        ));
    }
    let candidates = [
        format!("{alp}{path}"),
        path.to_string(),
        format!("/mnt/data/alpine{path}"),
    ];
    let raw = candidates
        .iter()
        .find_map(|p| std::fs::read(p).ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "arecord wav missing"))?;
    let mut samples = wav_to_pcm(&raw)?;
    // Drop turn-on click.
    let drop_n = (CAP_RATE as usize) * CAP_DROP_MS as usize / 1000;
    if samples.len() > drop_n {
        samples.drain(..drop_n);
    }
    // Keep ~secs
    let want = (CAP_RATE as usize) * secs as usize;
    if samples.len() > want {
        samples.truncate(want);
    }
    let peak = samples.iter().map(|x| x.saturating_abs()).max().unwrap_or(0);
    Ok((samples, peak))
}

fn wav_to_pcm(raw: &[u8]) -> io::Result<Vec<i16>> {
    if raw.len() < 44 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "wav too short"));
    }
    let off = if &raw[0..4] == b"RIFF" { 44 } else { 0 };
    let data = &raw[off..];
    let n = data.len() / 2;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let s = i16::from_le_bytes([data[i * 2], data[i * 2 + 1]]);
        out.push(s);
    }
    Ok(out)
}

/// Build a minimal WAV (16-bit mono PCM) from samples.
pub fn pcm_to_wav(samples: &[i16], rate: u32) -> Vec<u8> {
    let data_bytes = samples.len() * 2;
    let mut w = Vec::with_capacity(44 + data_bytes);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data_bytes as u32).to_le_bytes());
    w.extend_from_slice(b"WAVE");
    w.extend_from_slice(b"fmt ");
    w.extend_from_slice(&16u32.to_le_bytes()); // chunk size
    w.extend_from_slice(&1u16.to_le_bytes()); // PCM
    w.extend_from_slice(&(CAP_CH as u16).to_le_bytes());
    w.extend_from_slice(&rate.to_le_bytes());
    let byte_rate = rate * CAP_CH as u32 * 2;
    w.extend_from_slice(&byte_rate.to_le_bytes());
    w.extend_from_slice(&(CAP_CH as u16 * 2).to_le_bytes()); // block align
    w.extend_from_slice(&16u16.to_le_bytes()); // bits
    w.extend_from_slice(b"data");
    w.extend_from_slice(&(data_bytes as u32).to_le_bytes());
    for s in samples {
        w.extend_from_slice(&s.to_le_bytes());
    }
    w
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SttMode {
    Wake,
    Command,
}

fn keyterms(mode: SttMode) -> &'static [&'static str] {
    match mode {
        // Wake-only: bias STT toward the phrase. Do NOT put these on Command —
        // noisy MEMS + AGC was hallucinating "Hey Hax" for every utterance.
        SttMode::Wake => &[
            "hey hax",
            "ok hax",
            "okay hax",
            "hey tablet",
            "hi hax",
            "hax",
        ],
        // Command path (TALK / post-wake): domain phrases only, no wake words.
        SttMode::Command => &[
            "play a song",
            "play music",
            "stop music",
            "stop song",
            "turn on",
            "turn off",
            "sprinkler",
            "garden",
            "water",
            "lights",
            "camera",
            "network",
            "home assistant",
            "open camera",
            "go home",
            "status",
            "volume",
            "testing testing",
            "what time is it",
            "show network",
            "open music",
        ],
    }
}

fn load_api_key() -> Result<String, String> {
    std::fs::read_to_string(XAI_KEY_PATH)
        .map(|s| s.trim().to_string())
        .map_err(|e| format!("read {XAI_KEY_PATH}: {e}"))
        .and_then(|s| {
            if s.is_empty() {
                Err("empty xai.key".into())
            } else {
                Ok(s)
            }
        })
}

/// Upload WAV bytes to Grok STT. Keyterms must be separate form fields (quoted).
pub fn stt_wav(wav: &[u8], mode: SttMode) -> Result<String, String> {
    let key = load_api_key()?;
    let boundary = format!("----neonwire{}", std::process::id());
    let mut body = Vec::with_capacity(wav.len() + 4096);

    let mut field = |name: &str, value: &str| {
        let _ = write!(
            body,
            "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
        );
    };
    field("language", "en");
    field("format", "true");
    for t in keyterms(mode) {
        field("keyterm", t);
    }
    let _ = write!(
        body,
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"voice.wav\"\r\nContent-Type: audio/wav\r\n\r\n"
    );
    body.extend_from_slice(wav);
    let _ = write!(body, "\r\n--{boundary}--\r\n");

    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(60))
        .build();
    let resp = agent
        .post(STT_URL)
        .set("Authorization", &format!("Bearer {key}"))
        .set(
            "Content-Type",
            &format!("multipart/form-data; boundary={boundary}"),
        )
        .send_bytes(&body)
        .map_err(|e| format!("stt http: {e}"))?;

    let text = resp.into_string().map_err(|e| format!("stt body: {e}"))?;
    let v: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("stt json: {e} ({})", text.chars().take(80).collect::<String>()))?;
    Ok(v
        .get("text")
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .trim()
        .to_string())
}

/// Record then STT. Returns (transcript, peak). Empty transcript if silence/energy gate.
pub fn record_and_stt(secs: u32, mode: SttMode) -> Result<(String, i16), String> {
    // Tiny MEMS mics on this board are weak; wake needs a bit more to avoid
    // STT hallucinations, command is more permissive + AGC.
    // Ambient MEMS after live-arm is ~400–800 peak; speech pushes higher.
    // Below ~60 is true digital silence (ADC never latched).
    let min_peak = match mode {
        SttMode::Wake => 120,
        SttMode::Command => 50,
    };
    eprintln!("voice: record {secs}s mode={mode:?}");
    let (mut pcm, peak_raw) = record_pcm(secs).map_err(|e| format!("capture: {e}"))?;
    eprintln!("voice: raw peak={peak_raw} samples={}", pcm.len());
    if peak_raw < min_peak {
        eprintln!("voice: energy too low (peak={peak_raw}) — skip STT");
        return Ok((String::new(), peak_raw));
    }
    let peak = agc_inplace(&mut pcm, 12_000);
    let wav = pcm_to_wav(&pcm, CAP_RATE);
    let _ = std::fs::write(
        if mode == SttMode::Wake {
            "/tmp/voice-wake.wav"
        } else {
            "/tmp/voice-last-command.wav"
        },
        &wav,
    );
    let text = stt_wav(&wav, mode)?;
    eprintln!("voice: stt \"{text}\" (peak_raw={peak_raw} after_agc={peak})");
    Ok((text, peak_raw))
}

/// True if STT text looks like a wake phrase (for the background LISTEN scanner).
pub fn is_wake(text: &str) -> bool {
    let t = text.to_ascii_lowercase();
    let t = t.trim().trim_matches(|c: char| !c.is_alphanumeric() && !c.is_whitespace());
    if t.is_empty() {
        return false;
    }
    if t.contains("hey hax")
        || t.contains("ok hax")
        || t.contains("okay hax")
        || t.contains("hi hax")
        || t.contains("hey tablet")
        || t.contains("hey tax") // common mis-hear
        || t.contains("hey hacks")
    {
        return true;
    }
    let words = t.split_whitespace().count();
    // Short-only: bare "hax" / "tablet" — not "I'm trying to say hey hax"
    if words <= 3 && (t == "hax" || t.starts_with("hax ") || t.ends_with(" hax") || t == "tablet")
    {
        return true;
    }
    false
}

/// Transcript is *only* a wake phrase (no real command). Used to ignore
/// STT hallucinations on the TALK/command path.
pub fn is_wake_only(text: &str) -> bool {
    if !is_wake(text) {
        return false;
    }
    let t = text.to_ascii_lowercase();
    let words: Vec<&str> = t
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    // "hey hax", "ok hax", "hey tablet", "hax" — not longer sentences
    words.len() <= 4
}

/// Background wake scanner. Sets `hit` text when wake heard; stops when `run` is false
/// or after one hit (caller restarts if still preferred).
pub fn wake_scan_loop(run: Arc<AtomicBool>, hit_tx: std::sync::mpsc::Sender<String>) {
    while run.load(Ordering::Relaxed) {
        match record_and_stt(2, SttMode::Wake) {
            Ok((text, _)) if !text.is_empty() && is_wake(&text) => {
                eprintln!("voice: WAKE \"{text}\"");
                let _ = hit_tx.send(text);
                break;
            }
            Ok(_) => {}
            Err(e) => {
                eprintln!("voice: wake scan err: {e}");
                std::thread::sleep(Duration::from_millis(500));
            }
        }
        if !run.load(Ordering::Relaxed) {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}
