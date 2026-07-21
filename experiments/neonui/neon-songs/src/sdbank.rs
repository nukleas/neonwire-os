//! SD-card sample banks: dirt-samples directory layout, minimal WAV decode.
//!
//! `/mnt/sd/linux-lab/samples/<bank>/*.wav` — bank name = dir name, sample
//! index = alphabetical position (the dirt-samples convention strudel uses).
//! Decoder handles what the dirt subset actually contains: RIFF PCM 8/16/24/32
//! and IEEE float32, mono or stereo, any rate (the DSP voice resamples).

use std::sync::Arc;

use strudel_dsp::sample::Sample;

fn rd_u32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}
fn rd_u16(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}

pub fn decode_wav(b: &[u8]) -> Result<Sample, String> {
    if b.len() < 44 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return Err("not a RIFF/WAVE".into());
    }
    let (mut fmt, mut channels, mut rate, mut bits) = (0u16, 0u16, 0u32, 0u16);
    let mut data: Option<&[u8]> = None;
    let mut o = 12;
    while o + 8 <= b.len() {
        let id = &b[o..o + 4];
        let sz = rd_u32(b, o + 4) as usize;
        let body = o + 8;
        if body + sz > b.len() && id != b"data" {
            break;
        }
        match id {
            b"fmt " if sz >= 16 => {
                fmt = rd_u16(b, body);
                channels = rd_u16(b, body + 2);
                rate = rd_u32(b, body + 4);
                bits = rd_u16(b, body + 14);
                // WAVE_FORMAT_EXTENSIBLE: sub-format GUID starts with the real tag
                if fmt == 0xFFFE && sz >= 26 {
                    fmt = rd_u16(b, body + 24);
                }
            }
            b"data" => data = Some(&b[body..(body + sz).min(b.len())]),
            _ => {}
        }
        o = body + sz + (sz & 1); // chunks are word-aligned
    }
    let data = data.ok_or("no data chunk")?;
    if channels == 0 || rate == 0 {
        return Err("bad fmt chunk".into());
    }
    let ch = channels as usize;
    let bytes_per = (bits as usize / 8).max(1);
    let frames = data.len() / (bytes_per * ch);
    let mut planes: Vec<Vec<f32>> = vec![Vec::with_capacity(frames); ch.min(2)];
    for f in 0..frames {
        for c in 0..ch.min(2) {
            let i = (f * ch + c) * bytes_per;
            let v = match (fmt, bits) {
                (1, 8) => (f32::from(b[i.min(data.len() - 1)]) - 128.0) / 128.0,
                (1, 16) => f32::from(i16::from_le_bytes([data[i], data[i + 1]])) / 32768.0,
                (1, 24) => {
                    let raw =
                        i32::from_le_bytes([0, data[i], data[i + 1], data[i + 2]]) >> 8;
                    raw as f32 / 8_388_608.0
                }
                (1, 32) => {
                    i32::from_le_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]])
                        as f32
                        / 2_147_483_648.0
                }
                (3, 32) => f32::from_le_bytes([
                    data[i],
                    data[i + 1],
                    data[i + 2],
                    data[i + 3],
                ]),
                _ => return Err(format!("unsupported wav fmt={fmt} bits={bits}")),
            };
            planes[c].push(v);
        }
    }
    let left: Arc<[f32]> = planes[0].as_slice().into();
    let right: Option<Arc<[f32]>> = planes.get(1).map(|p| p.as_slice().into());
    Ok(Sample {
        left,
        right,
        sample_rate: f64::from(rate),
        num_frames: frames,
        loop_start: None,
        loop_end: None,
    })
}

/// Load every .wav in `<root>/<bank>/`, alphabetically. Returns decoded
/// samples with their slot index; empty if the bank dir doesn't exist.
pub fn load_bank(root: &str, bank: &str) -> Vec<(u32, Sample)> {
    let dir = format!("{root}/{bank}");
    let mut files: Vec<String> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.filter_map(|e| {
                let p = e.ok()?.path();
                let s = p.to_str()?;
                (s.to_ascii_lowercase().ends_with(".wav")).then(|| s.to_string())
            })
            .collect()
        })
        .unwrap_or_default();
    files.sort();
    files
        .into_iter()
        .enumerate()
        .filter_map(|(i, path)| {
            let bytes = std::fs::read(&path).ok()?;
            match decode_wav(&bytes) {
                Ok(s) => Some((i as u32, s)),
                Err(e) => {
                    eprintln!("sdbank: {path}: {e}");
                    None
                }
            }
        })
        .collect()
}
