//! Agent / shell control plane for neonwire.
//!
//! External processes (ZeroClaw `shell` tool, `neonctl`, scripts) drop a one-line
//! command into `CMD_PATH`. The shell drains it each loop iteration, applies it
//! to the live UI, and writes a one-line reply to `REPLY_PATH`.
//!
//! Status snapshot (no request needed) is rewritten to `STATUS_PATH` on ticks
//! so the agent can `cat` it without round-tripping.
//!
//! Protocol (line-oriented, ASCII, no quotes required for simple tokens):
//! ```text
//! view <name|index>          # home|system|network|house|files|intel|camera|music|songs|assist|ai
//! toast <message...>
//! backlight on|off|wake|<0-255>
//! status                     # force status rewrite + echo summary
//! shot [path]                # framebuffer dump (default /tmp/neon-shot.raw)
//! music play|stop|toggle|bpm <n>|vol <n>|preset <n|name>
//! songs stop|list|play [folder/track|folder track|N]
//! camera start|stop|snap [path]
//! ha refresh|toggle <entity_id>|on <id>|off <id>|list
//! help
//! ```

use std::fs;
use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// Dropbox for inbound commands (atomic-ish: writer truncates/writes full line).
pub const CMD_PATH: &str = "/tmp/neonwire.cmd";
/// Last command result for the caller.
pub const REPLY_PATH: &str = "/tmp/neonwire.reply";
/// Live JSON-ish status (rewritten often; safe to cat).
pub const STATUS_PATH: &str = "/tmp/neonwire.status";
/// Append-only log of executed commands (debug / agent memory).
pub const LOG_PATH: &str = "/tmp/neonwire.ctl.log";

/// App index order MUST match `Shell::apps` / `home::TILES`.
pub const APP_NAMES: &[&str] = &[
    "system", "network", "house", "files", "intel", "camera", "music", "songs", "assist",
];

/// Aliases accepted by `view`.
pub fn resolve_view(name: &str) -> Option<ViewTarget> {
    let n = name.trim().to_ascii_lowercase();
    match n.as_str() {
        "home" | "overview" | "0" | "01" => Some(ViewTarget::Home),
        "system" | "sys" | "1" | "02" => Some(ViewTarget::App(0)),
        "network" | "net" | "wifi" | "2" | "03" => Some(ViewTarget::App(1)),
        "house" | "ha" | "hass" | "homeassistant" | "3" | "04" => Some(ViewTarget::App(2)),
        "files" | "file" | "sd" | "4" | "05" => Some(ViewTarget::App(3)),
        "intel" | "ocint" | "5" | "06" => Some(ViewTarget::App(4)),
        "camera" | "cam" | "6" | "07" => Some(ViewTarget::App(5)),
        "music" | "seq" | "7" | "08" => Some(ViewTarget::App(6)),
        "songs" | "song" | "ost" | "8" | "09" => Some(ViewTarget::App(7)),
        "assist" | "assistant" | "ai" | "hax" | "agent" | "9" | "10" => Some(ViewTarget::App(8)),
        _ => {
            if let Ok(i) = n.parse::<usize>() {
                if i == 0 {
                    return Some(ViewTarget::Home);
                }
                if (1..=APP_NAMES.len()).contains(&i) {
                    return Some(ViewTarget::App(i - 1));
                }
            }
            None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewTarget {
    Home,
    App(usize),
}

#[derive(Debug, Clone)]
pub enum Cmd {
    View(ViewTarget),
    Toast(String),
    Backlight(BlCmd),
    Status,
    Shot(String),
    Music(String),
    Songs(String),
    Camera(String),
    Ha(String),
    Help,
    Unknown(String),
}

#[derive(Debug, Clone)]
pub enum BlCmd {
    On,
    Off,
    Wake,
    Level(u32),
}

/// Parse one command line into a [`Cmd`].
pub fn parse_line(line: &str) -> Option<Cmd> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let mut parts = line.splitn(2, char::is_whitespace);
    let verb = parts.next().unwrap_or("").to_ascii_lowercase();
    let rest = parts.next().unwrap_or("").trim();

    Some(match verb.as_str() {
        "view" | "goto" | "open" | "nav" => match resolve_view(rest) {
            Some(v) => Cmd::View(v),
            None => Cmd::Unknown(format!("unknown view: {rest}")),
        },
        "toast" | "notify" | "say" => {
            if rest.is_empty() {
                Cmd::Unknown("toast needs a message".into())
            } else {
                Cmd::Toast(rest.chars().take(64).collect())
            }
        }
        "backlight" | "bl" | "brightness" => {
            let a = rest.to_ascii_lowercase();
            match a.as_str() {
                "on" | "full" | "wake" => Cmd::Backlight(if a == "wake" {
                    BlCmd::Wake
                } else {
                    BlCmd::On
                }),
                "off" | "blank" | "sleep" => Cmd::Backlight(BlCmd::Off),
                s => match s.parse::<u32>() {
                    Ok(n) => Cmd::Backlight(BlCmd::Level(n.min(255))),
                    Err(_) => Cmd::Unknown(format!("backlight: {rest}")),
                },
            }
        }
        "status" | "stat" | "whoami" => Cmd::Status,
        "shot" | "screenshot" | "capture" => {
            let path = if rest.is_empty() {
                "/tmp/neon-shot.raw".into()
            } else {
                rest.to_string()
            };
            Cmd::Shot(path)
        }
        "music" | "seq" => Cmd::Music(rest.to_string()),
        "songs" | "song" => Cmd::Songs(rest.to_string()),
        "camera" | "cam" => Cmd::Camera(rest.to_string()),
        "ha" | "hass" | "house" => Cmd::Ha(rest.to_string()),
        "help" | "?" => Cmd::Help,
        other => Cmd::Unknown(format!("unknown cmd: {other} (try help)")),
    })
}

/// Non-blocking: take the pending command file if present.
pub fn take_pending() -> Option<String> {
    let path = Path::new(CMD_PATH);
    if !path.exists() {
        return None;
    }
    // Rename away first so a concurrent writer doesn't race the read.
    let staging = format!("{CMD_PATH}.taking");
    if fs::rename(CMD_PATH, &staging).is_err() {
        // Fall back to read+remove
        let raw = fs::read_to_string(path).ok()?;
        let _ = fs::remove_file(path);
        return Some(raw);
    }
    let raw = fs::read_to_string(&staging).ok();
    let _ = fs::remove_file(&staging);
    raw
}

/// Queue one or more command lines for the shell to drain (from apps / chips).
/// Multi-line payloads are executed in order in a single poll_control pass.
pub fn queue_lines(text: &str) {
    // Append if a pending file already exists (chip + concurrent neonctl).
    let mut body = String::new();
    if let Ok(existing) = fs::read_to_string(CMD_PATH) {
        body.push_str(&existing);
        if !body.ends_with('\n') {
            body.push('\n');
        }
    }
    body.push_str(text);
    if !body.ends_with('\n') {
        body.push('\n');
    }
    let _ = fs::write(CMD_PATH, body);
}

pub fn write_reply(msg: &str) {
    let line = if msg.ends_with('\n') {
        msg.to_string()
    } else {
        format!("{msg}\n")
    };
    if let Ok(mut f) = fs::File::create(REPLY_PATH) {
        let _ = f.write_all(line.as_bytes());
    }
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(LOG_PATH) {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = writeln!(f, "{ts} {msg}");
    }
}

pub fn help_text() -> String {
    "cmds: view|toast|backlight|status|shot|music|songs|camera|ha|help\n\
     views: home system network house files intel camera music songs assist\n\
     music: play|stop|toggle|bpm N|vol N|preset N|name\n\
     songs: stop|list|play [folder/track|N]\n\
     camera: start|stop|snap [path]\n\
     ha: refresh|list|toggle ID|on ID|off ID\n\
     backlight: on|off|wake|0-255"
        .into()
}

/// Human label for the current screen.
pub fn view_label(home: bool, app: Option<usize>) -> String {
    if home {
        "home".into()
    } else if let Some(i) = app {
        APP_NAMES.get(i).unwrap_or(&"?").to_string()
    } else {
        "?".into()
    }
}
