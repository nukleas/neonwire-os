//! ASSISTANT — touch front-end for the on-device ZeroClaw agent.
//!
//! Type a question on the panel keyboard; a worker thread POSTs it to the
//! ZeroClaw gateway on localhost and the reply lands in the transcript. The
//! LLM itself is remote (the tablet's 1 GB can't host a model) but the *agent
//! runtime* is on-device — see src/zeroclaw.rs for the wire contract.
//!
//! Threading mirrors apps/intel.rs (spawn + mpsc + non-blocking poll in tick);
//! text entry reuses widgets::TextPrompt exactly as apps/network.rs drives it.

use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

use neon_gfx::canvas::{mix, Canvas};
use neon_gfx::font_data::FONT_W;
use neon_gfx::geom::Rect;
use neon_gfx::theme::*;

use super::{App, Ctx, HitId, HitMap};
use crate::widgets::{PromptResult, TextPrompt, KB_BASE};
use crate::zeroclaw;

const HIT_ASK: HitId = 0xB000;
const HIT_CLEAR: HitId = 0xB001;

/// Keep the transcript bounded — this is a 1 GB device.
const MAX_TURNS: usize = 24;

enum Msg {
    Ok(String, Option<String>),
    Err(String),
}

#[derive(PartialEq)]
enum Role {
    You,
    Agent,
    Error,
}

struct Turn {
    role: Role,
    text: String,
}

pub struct AssistantApp {
    turns: Vec<Turn>,
    loading: bool,
    rx: Option<Receiver<Msg>>,
    kb: Option<TextPrompt>,
    model: Option<String>,
    online: Option<bool>,
    anim: u32,
}

impl AssistantApp {
    pub fn new() -> AssistantApp {
        AssistantApp {
            turns: Vec::new(),
            loading: false,
            rx: None,
            kb: None,
            model: None,
            online: None,
            anim: 0,
        }
    }

    fn push(&mut self, role: Role, text: String) {
        self.turns.push(Turn { role, text });
        if self.turns.len() > MAX_TURNS {
            let excess = self.turns.len() - MAX_TURNS;
            self.turns.drain(..excess);
        }
    }

    /// Fire the prompt at the gateway on a worker thread (intel.rs:72-89 shape).
    fn spawn_ask(&mut self, prompt: String) {
        if self.loading {
            return;
        }
        self.loading = true;
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

    /// Non-blocking drain (intel.rs:91-122 shape).
    fn poll_rx(&mut self, ctx: &mut Ctx) {
        let Some(rx) = &self.rx else { return };
        match rx.try_recv() {
            Ok(Msg::Ok(text, model)) => {
                self.loading = false;
                self.rx = None;
                self.online = Some(true);
                if model.is_some() {
                    self.model = model;
                }
                self.push(Role::Agent, text);
            }
            Ok(Msg::Err(e)) => {
                self.loading = false;
                self.rx = None;
                self.online = Some(false);
                ctx.set_toast("assistant: request failed");
                self.push(Role::Error, e);
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.loading = false;
                self.rx = None;
                self.push(Role::Error, "worker died".into());
            }
        }
    }
}

/// Greedy word-wrap to `cols` characters (ASCII panel font).
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

/// The bitmap font is ASCII — drop anything it can't draw.
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
        if self.loading {
            200
        } else {
            1000
        }
    }

    fn on_enter(&mut self) {
        if self.online.is_none() {
            self.online = Some(zeroclaw::health());
        }
    }

    fn tick(&mut self, ctx: &mut Ctx) {
        self.anim = self.anim.wrapping_add(1);
        self.poll_rx(ctx);
    }

    fn draw(&mut self, c: &mut Canvas, area: Rect, hits: &mut HitMap, _ctx: &Ctx) {
        c.panel(area.x, area.y + 8, area.w, area.h - 16, PURPLE, "ZEROCLAW // ON-DEVICE AGENT");

        let x = area.x + 24;
        let mut y = area.y + 38;
        let w = area.w - 48;
        let fw = FONT_W as i32;

        // ── status row ──────────────────────────────────────────────────
        let (dot, lab) = match self.online {
            Some(true) => (GREEN, "GATEWAY UP"),
            Some(false) => (RED, "GATEWAY DOWN"),
            None => (TEXT_DIM, "GATEWAY ?"),
        };
        c.fill(x, y + 6, 8, 8, dot);
        c.text(x + 16, y, lab, dot, 1);
        if let Some(m) = &self.model {
            let m = ascii(m);
            c.text(x + 16 + 13 * fw, y, &m.chars().take(28).collect::<String>(), TEXT_DIM, 1);
        }

        // ASK / CLEAR buttons
        let ask = Rect::new(area.x + area.w - 24 - 120, y - 6, 120, 30);
        let can_ask = !self.loading;
        let acol = if can_ask { PURPLE } else { TEXT_DIM };
        c.fill(ask.x, ask.y, ask.w, ask.h, mix(BG, acol, 40));
        c.neonbox(ask.x, ask.y, ask.w, ask.h, acol);
        c.text(ask.x + 30, ask.y + 4, "ASK", acol, 1);
        if can_ask {
            hits.add(ask, HIT_ASK);
        }
        if !self.turns.is_empty() {
            let clr = Rect::new(ask.x - 100, y - 6, 88, 30);
            c.neonbox(clr.x, clr.y, clr.w, clr.h, mix(BG, BORDER, 160));
            c.text(clr.x + 16, clr.y + 4, "CLEAR", TEXT2, 1);
            hits.add(clr, HIT_CLEAR);
        }
        y += 34;
        c.hline(x, y, w, mix(BG, PURPLE, 60));
        y += 10;

        // ── transcript (newest-anchored: render from the bottom up) ─────
        let bottom = area.y + area.h - 26;
        let cols = ((w - 24) / fw).max(20) as usize;
        let mut block: Vec<(u32, String)> = Vec::new();
        for t in &self.turns {
            let (col, prefix) = match t.role {
                Role::You => (CYAN, "> "),
                Role::Agent => (TEXT, ""),
                Role::Error => (RED, "! "),
            };
            let body = ascii(&t.text);
            for (i, line) in wrap(&body, cols).into_iter().enumerate() {
                let s = if i == 0 { format!("{prefix}{line}") } else { line };
                block.push((col, s));
            }
            block.push((0, String::new())); // spacer
        }
        if self.loading {
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

        if self.turns.is_empty() && !self.loading {
            c.text(x, y, "TAP ASK TO TALK TO THE AGENT", TEXT_DIM, 1);
            y += 20;
            if self.online == Some(false) {
                c.text(x, y, "gateway offline - is zeroclaw running?", TEXT_DIM, 1);
            }
        }

        // keyboard modal draws LAST so it captures every tap (network.rs:305)
        if let Some(kb) = &self.kb {
            kb.draw(c, hits);
        }
    }

    fn on_tap(&mut self, id: HitId, _ctx: &mut Ctx) -> bool {
        // modal owns all input while open (network.rs:311-327)
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
            HIT_ASK => {
                self.kb = Some(TextPrompt::new("[ ASK THE AGENT ]", 1, "SEND"));
                true
            }
            HIT_CLEAR => {
                self.turns.clear();
                true
            }
            _ => false,
        }
    }
}
