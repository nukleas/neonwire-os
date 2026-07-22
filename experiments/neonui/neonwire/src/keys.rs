//! Hardware key reader for mtk-kpd (volume / power side buttons).
//!
//! Opens the keypad input node (usually `/dev/input/event1`), drains EV_KEY
//! press events (value==1). Used for global shortcuts:
//!   VOLUME_UP   → open ASSIST
//!   VOLUME_DOWN → go HOME
//!   POWER       → toggle backlight blank/wake

use std::io;
use std::path::Path;

use neon_gfx::input::{InputEvent, EV_KEY, EV_SIZE};

pub const KEY_VOLUME_DOWN: u16 = 114;
pub const KEY_VOLUME_UP: u16 = 115;
pub const KEY_POWER: u16 = 116;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    VolumeUp,
    VolumeDown,
    Power,
    Other(u16),
}

pub struct Keys {
    fd: libc::c_int,
}

impl Keys {
    /// Prefer mtk-kpd by name; fall back to event1.
    pub fn open() -> io::Result<Keys> {
        let path = find_kpd().unwrap_or_else(|| "/dev/input/event1".into());
        let cdev = std::ffi::CString::new(path.as_str()).unwrap();
        let fd = unsafe { libc::open(cdev.as_ptr(), libc::O_RDONLY | libc::O_NONBLOCK) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        eprintln!("keys {path} fd={fd}");
        Ok(Keys { fd })
    }

    pub fn fd(&self) -> libc::c_int {
        self.fd
    }

    /// Drain all queued press events.
    pub fn drain(&mut self) -> Vec<Key> {
        let mut out = Vec::new();
        let mut buf = [0u8; EV_SIZE * 32];
        loop {
            let n =
                unsafe { libc::read(self.fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
            if n <= 0 {
                break;
            }
            for chunk in buf[..n as usize].chunks_exact(EV_SIZE) {
                let ev = unsafe { std::ptr::read_unaligned(chunk.as_ptr() as *const InputEvent) };
                if ev.type_ == EV_KEY && ev.value == 1 {
                    // press edge only
                    out.push(classify(ev.code));
                }
            }
            if (n as usize) < buf.len() {
                break;
            }
        }
        out
    }
}

impl Drop for Keys {
    fn drop(&mut self) {
        unsafe { libc::close(self.fd) };
    }
}

fn classify(code: u16) -> Key {
    match code {
        KEY_VOLUME_UP => Key::VolumeUp,
        KEY_VOLUME_DOWN => Key::VolumeDown,
        KEY_POWER => Key::Power,
        c => Key::Other(c),
    }
}

/// Parse `/proc/bus/input/devices` for `Name="mtk-kpd"` → Handlers eventN.
fn find_kpd() -> Option<String> {
    let text = std::fs::read_to_string("/proc/bus/input/devices").ok()?;
    let mut name = String::new();
    for line in text.lines() {
        if let Some(n) = line.strip_prefix("N: Name=\"") {
            name = n.trim_end_matches('"').to_string();
        } else if line.starts_with("H: Handlers=") && name.contains("mtk-kpd") {
            for tok in line.split_whitespace() {
                if tok.starts_with("event") {
                    let p = format!("/dev/input/{tok}");
                    if Path::new(&p).exists() {
                        return Some(p);
                    }
                }
            }
        }
    }
    // secondary: any "keypad" name
    name.clear();
    for line in text.lines() {
        if let Some(n) = line.strip_prefix("N: Name=\"") {
            name = n.trim_end_matches('"').to_string();
        } else if line.starts_with("H: Handlers=")
            && (name.contains("keypad") || name.contains("kpd") || name.contains("gpio-keys"))
        {
            for tok in line.split_whitespace() {
                if tok.starts_with("event") {
                    let p = format!("/dev/input/{tok}");
                    if Path::new(&p).exists() {
                        return Some(p);
                    }
                }
            }
        }
    }
    None
}
