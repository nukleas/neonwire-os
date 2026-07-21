// SPDX-License-Identifier: AGPL-3.0-or-later
// Derivative of Strudel (https://strudel.cc, AGPL-3.0). See ../LICENSING.md.
//! Strudel song playback for NEONWIRE — evaluate a .strudel file, render
//! synth audio block-by-block for the shell's PCM writer.
//!
//! processor/channel/mapper/scheduler are ports of strudel-audio's modules
//! (upstream hard-depends on cpal, unusable on static musl). offline.rs is
//! reworked into the streaming `SongRenderer`. Keep ports in sync with
//! $HOME/src/strudel-rs when pulling.

mod channel;
mod mapper;
pub mod offline;
pub mod processor;
pub mod scheduler;
pub mod sdbank;

pub use offline::SongRenderer;
pub use processor::VisEvent;
pub use strudel_core::{Fraction, Hap, Pattern, State, TimeSpan, Value};
pub use strudel_dsl::{EvaluatedFile, evaluate_file, parse_strudel_file};

/// A parsed + evaluated song ready to render.
pub struct Song {
    pub pattern: Pattern,
    pub bpm: f64,
}

/// Evaluate .strudel source (full file dialect: comments, setbpm/setcpm,
/// let-bindings, tracks). Returns the stacked pattern + tempo.
pub fn eval_song(source: &str) -> Result<Song, String> {
    let file = parse_strudel_file(source).map_err(|e| format!("parse: {e}"))?;
    if file.tracks.is_empty() {
        return Err("no tracks".into());
    }
    let ev = evaluate_file(&file).map_err(|e| format!("eval: {e}"))?;
    Ok(Song {
        pattern: ev.pattern,
        bpm: ev.tempo.map(|t| t.to_bpm()).unwrap_or(120.0),
    })
}
