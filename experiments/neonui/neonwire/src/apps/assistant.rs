//! ASSISTANT — voice-first front-end for the on-device ZeroClaw agent.
//!
//! Voice is **in-process Rust** (`crate::voice`):
//!   mic_arm + PCM capture on pcmC0D1c → multipart STT to Grok → agent.
//!
//!   1. **LISTEN** — background wake-word scan (energy gate + short STT).
//!   2. On wake → 6s command capture → Grok STT → ZeroClaw agent.
//!   3. **TALK** — push-to-talk (same command path).
//!   4. **TYPE** — keyboard.
//!
//! Vol+ jumps here from any screen (shell keys.rs).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use neon_gfx::canvas::{mix, Canvas};
use neon_gfx::font_data::FONT_W;
use neon_gfx::geom::Rect;
use neon_gfx::theme::*;

use super::{App, Ctx, HitId, HitMap};
use crate::voice::{self, SttMode};
use crate::widgets::{PromptResult, TextPrompt, KB_BASE};
use crate::zeroclaw;

const HIT_ASK: HitId = 0xB000;
const HIT_CLEAR: HitId = 0xB001;
const HIT_TALK: HitId = 0xB002;
const HIT_LISTEN: HitId = 0xB003;
const HIT_WATCH: HitId = 0xB004;
const HIT_HIST0: HitId = 0xB180;

const MAX_TURNS: usize = 24;
const MAX_HISTORY: usize = 6;
const HISTORY_PATH: &str = "/mnt/sd/linux-lab/assist-history.json";
const CMD_SECS: u32 = 6;

/// Written by the zeroclaw heartbeat (`watcher` agent, every 30 min) — periodic
/// read-only triage of /tmp/*.log. See cfg/agents/watcher/workspace/HEARTBEAT.md.
/// The daemon has no delivery channel configured, so this file IS the only
/// place findings surface — hence this panel.
const FINDINGS_PATH: &str =
    "/mnt/sd/linux-lab/zeroclaw/cfg/agents/watcher/workspace/FINDINGS.md";
/// Last FINDINGS.md mtime the user actually looked at, so WATCH can badge.
const SEEN_PATH: &str = "/mnt/sd/linux-lab/assist-findings-seen";
/// Cap the dump — the file only grows, and the transcript shows its tail.
const FINDINGS_MAX: usize = 6000;

/// Written by `zeroclaw daemon` (not `gateway start`). If the daemon dies, is
/// replaced by a gateway-only process, or the heartbeat worker wedges, this
/// file simply stops advancing — which is the whole point of watching it.
/// A watchdog that silently stops produces no logs to notice it by.
const HB_STATE_PATH: &str = "/mnt/sd/linux-lab/zeroclaw/cfg/state/daemon_state.json";
/// Must track `heartbeat.interval_minutes` in the zeroclaw config (30 min).
const HB_INTERVAL_SECS: u64 = 30 * 60;
/// One interval plus slack — a tick can take minutes on this CPU.
const HB_OK_SECS: u64 = HB_INTERVAL_SECS + 300;
/// Two missed intervals: something is wrong but may still recover.
const HB_WARN_SECS: u64 = HB_INTERVAL_SECS * 2 + 300;
/// Re-stat the state file at most this often while the app is open.
const HB_POLL: Duration = Duration::from_secs(15);

enum Msg {
    Ok(String, Option<String>),
    Err(String),
    Heard(String),
}

#[derive(PartialEq)]
enum Role {
    You,
    Agent,
    Error,
    Local,
    Watch,
}

struct Turn {
    role: Role,
    text: String,
}

pub struct AssistantApp {
    turns: Vec<Turn>,
    loading: bool,
    listening: bool,
    wake_armed: bool,
    listen_pref: bool,
    wake_run: Option<Arc<AtomicBool>>,
    wake_hit: Option<Receiver<String>>,
    rx: Option<Receiver<Msg>>,
    kb: Option<TextPrompt>,
    model: Option<String>,
    online: Option<bool>,
    anim: u32,
    history: Vec<String>,
    last_wake_check: Instant,
    /// FINDINGS.md is newer than the last one the user opened.
    findings_new: bool,
    /// Liveness of the zeroclaw heartbeat worker; None = state file unreadable.
    hb: Option<Hb>,
    last_hb_check: Instant,
}

/// Snapshot of the heartbeat component from `daemon_state.json`.
struct Hb {
    /// Seconds since the worker last reported ok.
    age: u64,
    /// Component-reported failure, if any.
    err: Option<String>,
}

impl AssistantApp {
    pub fn new() -> AssistantApp {
        AssistantApp {
            turns: Vec::new(),
            loading: false,
            listening: false,
            wake_armed: false,
            listen_pref: false,
            wake_run: None,
            wake_hit: None,
            rx: None,
            kb: None,
            model: None,
            online: None,
            anim: 0,
            history: load_history(),
            last_wake_check: Instant::now(),
            findings_new: false,
            hb: None,
            last_hb_check: Instant::now(),
        }
    }

    /// Load the newest slice of the watchdog's findings into the transcript.
    fn show_findings(&mut self, ctx: &mut Ctx) {
        match read_findings() {
            Ok(text) => {
                self.push(Role::Watch, text);
                save_seen(findings_mtime());
                self.findings_new = false;
                ctx.set_toast("watchdog report");
            }
            Err(e) => {
                self.push(Role::Local, e);
                ctx.set_toast("nothing yet");
            }
        }
    }

    fn push(&mut self, role: Role, text: String) {
        self.turns.push(Turn { role, text });
        if self.turns.len() > MAX_TURNS {
            let excess = self.turns.len() - MAX_TURNS;
            self.turns.drain(..excess);
        }
    }

    fn remember(&mut self, prompt: &str) {
        let p = prompt.trim();
        if p.is_empty() {
            return;
        }
        self.history.retain(|h| h != p);
        self.history.insert(0, p.to_string());
        self.history.truncate(MAX_HISTORY);
        save_history(&self.history);
    }

    fn spawn_ask(&mut self, prompt: String) {
        if self.loading {
            return;
        }
        self.remember(&prompt);
        self.loading = true;
        self.listening = false;
        self.push(Role::You, prompt.clone());
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        thread::spawn(move || {
            let msg = match zeroclaw::ask(&prompt) {
                Ok(r) => Msg::Ok(r.text, r.model),
                Err(e) => Msg::Err(e.to_string()),
            };
            let _ = tx.send(msg);
        });
    }

    fn spawn_voice_command(&mut self, why: &str) {
        if self.loading {
            return;
        }
        // Free the mic — wake scan must not hold CAP_LOCK during TALK.
        self.stop_wake_scanner();
        self.loading = true;
        self.listening = true;
        self.push(
            Role::Local,
            format!("{why} — speak ({CMD_SECS}s), Rust mic → Grok STT…"),
        );
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        thread::spawn(move || {
            match voice::record_and_stt(CMD_SECS, SttMode::Command) {
                Ok((text, peak)) => {
                    if text.is_empty() {
                        let _ = tx.send(Msg::Err(format!(
                            "mic quiet / empty STT (peak={peak}) — speak closer after HEARING…"
                        )));
                        return;
                    }
                    // Command keyterms used to include "hey hax" → STT stuck on it.
                    // Still drop pure wake-only hallucinations on TALK.
                    if voice::is_wake_only(&text) {
                        let _ = tx.send(Msg::Err(format!(
                            "heard only wake-ish \"{text}\" (peak={peak}) — say a command, not hey hax"
                        )));
                        return;
                    }
                    let _ = tx.send(Msg::Heard(text.clone()));
                    match zeroclaw::ask(&text) {
                        Ok(r) => {
                            let _ = tx.send(Msg::Ok(r.text, r.model));
                        }
                        Err(e) => {
                            let _ = tx.send(Msg::Err(e.to_string()));
                        }
                    }
                }
                Err(e) => {
                    let _ = tx.send(Msg::Err(e));
                }
            }
        });
    }

    fn set_wake_armed(&mut self, on: bool, ctx: &mut Ctx) {
        self.listen_pref = on;
        if on {
            self.start_wake_scanner();
            self.push(
                Role::Local,
                "LISTEN on — say \"hey hax\" then your command".into(),
            );
            ctx.set_toast("say hey hax");
        } else {
            self.stop_wake_scanner();
            self.push(Role::Local, "LISTEN off".into());
            ctx.set_toast("listen off");
        }
    }

    fn start_wake_scanner(&mut self) {
        self.stop_wake_scanner();
        let run = Arc::new(AtomicBool::new(true));
        self.wake_run = Some(run.clone());
        self.wake_armed = true;
        let (wtx, wrx) = mpsc::channel();
        self.wake_hit = Some(wrx);
        thread::spawn(move || {
            voice::wake_scan_loop(run, wtx);
        });
    }

    fn stop_wake_scanner(&mut self) {
        if let Some(run) = self.wake_run.take() {
            run.store(false, Ordering::Relaxed);
        }
        self.wake_armed = false;
        self.wake_hit = None;
    }

    fn rearm_wake_if_needed(&mut self) {
        if self.listen_pref && !self.loading {
            self.start_wake_scanner();
        }
    }

    fn poll_wake_hit(&mut self, ctx: &mut Ctx) {
        if !self.listen_pref || self.loading {
            return;
        }
        if self.last_wake_check.elapsed() < Duration::from_millis(200) {
            return;
        }
        self.last_wake_check = Instant::now();
        let Some(rx) = &self.wake_hit else {
            return;
        };
        match rx.try_recv() {
            Ok(phrase) => {
                self.wake_hit = None;
                self.wake_armed = false;
                if let Some(run) = self.wake_run.take() {
                    run.store(false, Ordering::Relaxed);
                }
                self.push(Role::Local, format!("wake: \"{phrase}\""));
                ctx.set_toast("listening…");
                self.spawn_voice_command("wake");
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.wake_hit = None;
                self.wake_armed = false;
                self.wake_run = None;
                if self.listen_pref && !self.loading {
                    self.start_wake_scanner();
                }
            }
        }
    }

    fn poll_rx(&mut self, ctx: &mut Ctx) {
        let Some(rx) = &self.rx else {
            return;
        };
        match rx.try_recv() {
            Ok(Msg::Heard(t)) => {
                self.listening = false;
                if t.is_empty() {
                    self.loading = false;
                    self.rx = None;
                    self.push(
                        Role::Local,
                        "mic quiet / empty STT — speak closer, 1s after HEARING…".into(),
                    );
                    ctx.set_toast("mic quiet");
                    self.rearm_wake_if_needed();
                } else {
                    self.remember(&t);
                    self.push(Role::You, format!("(voice) {t}"));
                    ctx.set_toast("heard");
                }
            }
            Ok(Msg::Ok(text, model)) => {
                self.loading = false;
                self.listening = false;
                self.rx = None;
                self.online = Some(true);
                if model.is_some() {
                    self.model = model;
                }
                self.push(Role::Agent, text);
                self.rearm_wake_if_needed();
            }
            Ok(Msg::Err(e)) => {
                self.loading = false;
                self.listening = false;
                self.rx = None;
                self.online = Some(false);
                ctx.set_toast("voice/agent failed");
                self.push(Role::Error, e);
                self.rearm_wake_if_needed();
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.loading = false;
                self.listening = false;
                self.rx = None;
                self.push(Role::Error, "worker died".into());
            }
        }
    }

    fn draw_chips(
        &self,
        c: &mut Canvas,
        hits: &mut HitMap,
        x0: i32,
        y0: i32,
        max_w: i32,
        labels: &[(HitId, &str, u32)],
    ) -> i32 {
        let fw = FONT_W as i32;
        let chip_h = 28;
        let gap = 6;
        let mut x = x0;
        let mut y = y0;
        for &(id, label, accent) in labels {
            let cw = (label.len() as i32 * fw + 20).min(max_w);
            if x > x0 && x + cw > x0 + max_w {
                x = x0;
                y += chip_h + gap;
            }
            let r = Rect::new(x, y, cw, chip_h);
            c.fill(r.x, r.y, r.w, r.h, mix(BG, accent, 36));
            c.neonbox(r.x, r.y, r.w, r.h, accent);
            c.text(r.x + 10, r.y + 4, label, accent, 1);
            hits.add(r, id);
            x += cw + gap;
        }
        if labels.is_empty() {
            y0
        } else {
            y + chip_h + 8
        }
    }
}

fn load_history() -> Vec<String> {
    let Ok(raw) = std::fs::read_to_string(HISTORY_PATH) else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<String>>(&raw)
        .unwrap_or_default()
        .into_iter()
        .filter(|s| !s.trim().is_empty())
        .take(MAX_HISTORY)
        .collect()
}

fn save_history(h: &[String]) {
    if let Ok(bytes) = serde_json::to_vec(h) {
        let _ = std::fs::write(HISTORY_PATH, bytes);
    }
}

/// Seconds since the unix epoch, now.
fn now_epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// RFC3339 → unix secs. Only the fixed-width prefix is parsed; zeroclaw always
/// writes UTC (`...+00:00`) and the device runs UTC, so the offset is ignored.
/// Avoids pulling `chrono` into the UI binary for one field.
fn rfc3339_epoch(s: &str) -> Option<u64> {
    if s.len() < 19 {
        return None;
    }
    let n = |a: usize, z: usize| s.get(a..z)?.parse::<i64>().ok();
    let (y, mo, d) = (n(0, 4)?, n(5, 7)?, n(8, 10)?);
    let (h, mi, se) = (n(11, 13)?, n(14, 16)?, n(17, 19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return None;
    }
    // days_from_civil (Howard Hinnant's civil calendar algorithm)
    let y = if mo <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = days * 86_400 + h * 3600 + mi * 60 + se;
    u64::try_from(secs).ok()
}

/// Read the heartbeat component out of the daemon state file.
fn read_hb() -> Option<Hb> {
    let raw = std::fs::read_to_string(HB_STATE_PATH).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let hb = v.get("components")?.get("heartbeat")?;
    let last_ok = rfc3339_epoch(hb.get("last_ok")?.as_str()?)?;
    let err = hb
        .get("last_error")
        .and_then(|e| e.as_str())
        .map(|s| s.to_string());
    Some(Hb {
        age: now_epoch().saturating_sub(last_ok),
        err,
    })
}

/// Compact duration for the status line: 45s / 12m / 2h14m / 3d1h.
fn fmt_age(secs: u64) -> String {
    match secs {
        s if s < 90 => format!("{s}s"),
        s if s < 3600 => format!("{}m", s / 60),
        s if s < 86_400 => format!("{}h{:02}m", s / 3600, (s % 3600) / 60),
        s => format!("{}d{}h", s / 86_400, (s % 86_400) / 3600),
    }
}

/// mtime of FINDINGS.md as unix secs, 0 when absent.
fn findings_mtime() -> u64 {
    std::fs::metadata(FINDINGS_PATH)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn load_seen() -> u64 {
    std::fs::read_to_string(SEEN_PATH)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

fn save_seen(t: u64) {
    let _ = std::fs::write(SEEN_PATH, t.to_string());
}

/// Newest tail of FINDINGS.md, trimmed to a whole `## ` section so the panel
/// never opens mid-sentence. Err carries a human line for the transcript.
fn read_findings() -> Result<String, String> {
    let raw = std::fs::read_to_string(FINDINGS_PATH)
        .map_err(|_| "watchdog has not filed anything yet (no FINDINGS.md)".to_string())?;
    if raw.trim().is_empty() {
        return Err("watchdog ran but found nothing worth reporting".into());
    }
    if raw.len() <= FINDINGS_MAX {
        return Ok(raw);
    }
    let mut cut = raw.len() - FINDINGS_MAX;
    while !raw.is_char_boundary(cut) {
        cut += 1;
    }
    let tail = &raw[cut..];
    Ok(match tail.find("\n## ") {
        Some(p) => format!("(older entries trimmed)\n{}", &tail[p + 1..]),
        None => format!("(older entries trimmed)\n{tail}"),
    })
}

fn wrap(text: &str, cols: usize) -> Vec<String> {
    let mut out = Vec::new();
    for para in text.split('\n') {
        if para.trim().is_empty() {
            out.push(String::new());
            continue;
        }
        let mut line = String::new();
        for word in para.split_whitespace() {
            if line.is_empty() {
                line = word.to_string();
            } else if line.len() + 1 + word.len() <= cols {
                line.push(' ');
                line.push_str(word);
            } else {
                out.push(std::mem::take(&mut line));
                line = word.to_string();
            }
        }
        if !line.is_empty() {
            out.push(line);
        }
    }
    out
}

fn ascii(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '\u{2014}' | '\u{2013}' => '-',
            '\u{2018}' | '\u{2019}' => '\'',
            '\u{201c}' | '\u{201d}' => '"',
            c if c == '\n' || (c.is_ascii() && !c.is_control()) => c,
            _ => ' ',
        })
        .collect()
}

impl App for AssistantApp {
    fn title(&self) -> &'static str {
        "ASSISTANT"
    }

    fn accent(&self) -> u32 {
        PURPLE
    }

    fn tick_ms(&self) -> u64 {
        if self.loading || self.listening || self.wake_armed {
            200
        } else {
            1000
        }
    }

    fn on_enter(&mut self) {
        if self.online.is_none() {
            self.online = Some(zeroclaw::health());
        }
        // Cheap stat, not a read — the heartbeat may have filed something
        // while the tablet sat blanked.
        self.findings_new = findings_mtime() > load_seen();
        self.hb = read_hb();
        self.last_hb_check = Instant::now();
    }

    fn on_leave(&mut self) {
        self.listen_pref = false;
        self.stop_wake_scanner();
    }

    fn tick(&mut self, ctx: &mut Ctx) {
        self.anim = self.anim.wrapping_add(1);
        self.poll_wake_hit(ctx);
        self.poll_rx(ctx);
        // A tick landing while the app is open should clear STALE without a
        // re-entry, and also surface findings filed since we opened.
        if self.last_hb_check.elapsed() >= HB_POLL {
            self.last_hb_check = Instant::now();
            self.hb = read_hb();
            if !self.findings_new {
                self.findings_new = findings_mtime() > load_seen();
            }
        }
    }

    fn draw(&mut self, c: &mut Canvas, area: Rect, hits: &mut HitMap, _ctx: &Ctx) {
        c.panel(
            area.x,
            area.y + 8,
            area.w,
            area.h - 16,
            PURPLE,
            "ZEROCLAW // RUST VOICE",
        );

        let x = area.x + 24;
        let mut y = area.y + 38;
        let w = area.w - 48;
        let fw = FONT_W as i32;

        let (dot, lab) = match self.online {
            Some(true) => (GREEN, "GATEWAY UP"),
            Some(false) => (RED, "GATEWAY DOWN"),
            None => (TEXT_DIM, "GATEWAY ?"),
        };
        c.fill(x, y + 6, 8, 8, dot);
        c.text(x + 16, y, lab, dot, 1);
        if let Some(m) = &self.model {
            let m = ascii(m);
            c.text(
                x + 16 + 13 * fw,
                y,
                &m.chars().take(22).collect::<String>(),
                TEXT_DIM,
                1,
            );
        }
        y += 24;

        // Watchdog liveness. The heartbeat is the one component that cannot
        // report its own death, so this is read from daemon_state.json rather
        // than from anything the agent itself writes.
        let (hdot, htext) = match &self.hb {
            Some(hb) if hb.err.is_some() => (
                RED,
                format!(
                    "WATCHDOG ERROR - {}",
                    hb.err.as_deref().unwrap_or("").chars().take(46).collect::<String>()
                ),
            ),
            Some(hb) if hb.age > HB_WARN_SECS => (
                RED,
                format!("WATCHDOG STALE - no tick in {}", fmt_age(hb.age)),
            ),
            Some(hb) if hb.age > HB_OK_SECS => (
                AMBER,
                format!("WATCHDOG LATE - last tick {} ago", fmt_age(hb.age)),
            ),
            Some(hb) => (
                GREEN,
                format!("WATCHDOG OK - last tick {} ago", fmt_age(hb.age)),
            ),
            None => (
                RED,
                "WATCHDOG DOWN - no daemon state (gateway-only?)".to_string(),
            ),
        };
        c.fill(x, y + 6, 8, 8, hdot);
        c.text(x + 16, y, &htext, hdot, 1);

        y += 28;
        c.hline(x, y, w, mix(BG, PURPLE, 60));
        y += 10;

        let can = !self.loading;
        let btn_h = 36;
        let gap = 8;
        let listen_w = 120;
        let talk_w = 140;
        let type_w = 90;
        let mut bx = x;

        let lcol = if self.wake_armed {
            RED
        } else if can {
            AMBER
        } else {
            TEXT_DIM
        };
        let lr = Rect::new(bx, y, listen_w, btn_h);
        c.fill(lr.x, lr.y, lr.w, lr.h, mix(BG, lcol, 48));
        c.neonbox(lr.x, lr.y, lr.w, lr.h, lcol);
        let llab = if self.wake_armed { "LISTENING" } else { "LISTEN" };
        c.textg(
            lr.x + lr.w / 2 - (llab.len() as i32 * fw) / 2,
            lr.y + 8,
            llab,
            lcol,
            1,
        );
        if can || self.wake_armed {
            hits.add(lr, HIT_LISTEN);
        }
        bx += listen_w + gap;

        let tcol = if self.listening {
            RED
        } else if can {
            GREEN
        } else {
            TEXT_DIM
        };
        let tr = Rect::new(bx, y, talk_w, btn_h);
        c.fill(tr.x, tr.y, tr.w, tr.h, mix(BG, tcol, 48));
        c.neonbox(tr.x, tr.y, tr.w, tr.h, tcol);
        let tlab = if self.listening {
            "HEARING…"
        } else {
            "TALK 6s"
        };
        c.textg(
            tr.x + tr.w / 2 - (tlab.len() as i32 * fw) / 2,
            tr.y + 8,
            tlab,
            tcol,
            1,
        );
        if can {
            hits.add(tr, HIT_TALK);
        }
        bx += talk_w + gap;

        let acol = if can { PURPLE } else { TEXT_DIM };
        let ar = Rect::new(bx, y, type_w, btn_h);
        c.fill(ar.x, ar.y, ar.w, ar.h, mix(BG, acol, 40));
        c.neonbox(ar.x, ar.y, ar.w, ar.h, acol);
        c.text(ar.x + 22, ar.y + 8, "TYPE", acol, 1);
        if can {
            hits.add(ar, HIT_ASK);
        }
        bx += type_w + gap;

        // WATCH — the heartbeat watchdog's log triage. Badges amber when it
        // has filed something since the last time this was opened.
        let watch_w = 112;
        let wcol = if self.findings_new { AMBER } else { TEXT2 };
        let wr = Rect::new(bx, y, watch_w, btn_h);
        c.fill(wr.x, wr.y, wr.w, wr.h, mix(BG, wcol, 36));
        c.neonbox(wr.x, wr.y, wr.w, wr.h, wcol);
        let wlab = if self.findings_new { "WATCH *" } else { "WATCH" };
        c.text(
            wr.x + wr.w / 2 - (wlab.len() as i32 * fw) / 2,
            wr.y + 8,
            wlab,
            wcol,
            1,
        );
        hits.add(wr, HIT_WATCH);
        bx += watch_w + gap;

        if !self.turns.is_empty() {
            let cr = Rect::new(bx, y, 86, btn_h);
            c.neonbox(cr.x, cr.y, cr.w, cr.h, mix(BG, BORDER, 160));
            c.text(cr.x + 16, cr.y + 8, "CLEAR", TEXT2, 1);
            hits.add(cr, HIT_CLEAR);
        }
        y += btn_h + 10;

        if self.wake_armed {
            c.text(x, y, "wake: say  HEY HAX  then your command", AMBER, 1);
            y += 22;
        }

        if !self.history.is_empty() {
            c.text(x, y, "RECENT", TEXT_DIM, 1);
            y += 20;
            let hist: Vec<(HitId, String, u32)> = self
                .history
                .iter()
                .enumerate()
                .map(|(i, h)| {
                    let short = if h.len() > 18 {
                        format!("{}…", h.chars().take(16).collect::<String>())
                    } else {
                        h.clone()
                    };
                    (HIT_HIST0 + i as u32, short, CYAN)
                })
                .collect();
            let refs: Vec<(HitId, &str, u32)> = hist
                .iter()
                .map(|(id, s, col)| (*id, s.as_str(), *col))
                .collect();
            y = self.draw_chips(c, hits, x, y, w, &refs);
        }

        c.hline(x, y, w, mix(BG, PURPLE, 40));
        y += 8;

        let bottom = area.y + area.h - 26;
        let cols = ((w - 24) / fw).max(20) as usize;
        let mut block: Vec<(u32, String)> = Vec::new();
        for t in &self.turns {
            let (col, prefix) = match t.role {
                Role::You => (CYAN, "> "),
                Role::Agent => (TEXT, ""),
                Role::Error => (RED, "! "),
                Role::Local => (GREEN, "* "),
                Role::Watch => (AMBER, ""),
            };
            let body = ascii(&t.text);
            for (i, line) in wrap(&body, cols).into_iter().enumerate() {
                let s = if i == 0 {
                    format!("{prefix}{line}")
                } else {
                    line
                };
                block.push((col, s));
            }
            block.push((0, String::new()));
        }
        if self.listening {
            let dots = ".".repeat((self.anim as usize % 4) + 1);
            block.push((RED, format!("recording{dots}")));
        } else if self.wake_armed {
            let dots = ".".repeat((self.anim as usize % 4) + 1);
            block.push((AMBER, format!("scanning for hey hax{dots}")));
        } else if self.loading {
            let dots = ".".repeat((self.anim as usize % 4) + 1);
            block.push((AMBER, format!("thinking{dots}")));
        }

        let avail = ((bottom - y) / 18).max(1) as usize;
        let start = block.len().saturating_sub(avail);
        for (col, line) in &block[start..] {
            if line.is_empty() {
                y += 8;
                continue;
            }
            c.text(x, y, line, *col, 1);
            y += 18;
        }

        if self.turns.is_empty() && !self.loading && !self.wake_armed {
            if self.findings_new {
                c.text(x, y, "watchdog filed a new report - tap WATCH", AMBER, 1);
                y += 20;
            }
            c.text(
                x,
                y,
                "LISTEN = hey hax  ·  TALK = ptt  ·  all Rust voice path",
                TEXT_DIM,
                1,
            );
        }

        if let Some(kb) = &self.kb {
            kb.draw(c, hits);
        }
    }

    fn on_tap(&mut self, id: HitId, ctx: &mut Ctx) -> bool {
        if let Some(kb) = &mut self.kb {
            if id >= KB_BASE {
                match kb.on_tap(id) {
                    PromptResult::Open => {}
                    PromptResult::Cancelled => self.kb = None,
                    PromptResult::Submitted(q) => {
                        self.kb = None;
                        self.spawn_ask(q);
                    }
                }
            }
            return true;
        }
        match id {
            HIT_LISTEN => {
                let next = !self.listen_pref;
                if next && self.loading {
                    ctx.set_toast("busy");
                    return true;
                }
                self.set_wake_armed(next, ctx);
                true
            }
            HIT_TALK => {
                if self.listen_pref {
                    self.set_wake_armed(false, ctx);
                }
                self.spawn_voice_command("talk");
                true
            }
            HIT_ASK => {
                self.kb = Some(TextPrompt::new("[ ASK THE AGENT ]", 1, "SEND"));
                true
            }
            HIT_WATCH => {
                self.show_findings(ctx);
                true
            }
            HIT_CLEAR => {
                self.turns.clear();
                true
            }
            id if (HIT_HIST0..HIT_HIST0 + MAX_HISTORY as u32).contains(&id) => {
                let i = (id - HIT_HIST0) as usize;
                if let Some(p) = self.history.get(i).cloned() {
                    self.spawn_ask(p);
                }
                true
            }
            _ => false,
        }
    }
}
