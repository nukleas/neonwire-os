//! CAMERA app — live viewfinder over the reverse-engineered SP2509 pipeline.
//!
//! Spawns a single long-lived `camgrab --stream` process that keeps the ISP open
//! and rewrites `/tmp/preview.rgb` every frame. We poll that file and blit it.
//! Much faster than the old spawn-per-frame path (~1 fps → multi-fps), and AE
//! actually works (no forced 16x gain env override).

use neon_gfx::canvas::{mix, Canvas};
use neon_gfx::geom::Rect;
use neon_gfx::theme::*;

use super::{App, ControlResult, Ctx, HitId, HitMap};

const CAMGRAB: &str = "/mnt/sd/linux-lab/camgrab";
const PREVIEW_RGB: &str = "/tmp/preview.rgb";
const PREVIEW_META: &str = "/tmp/preview.meta";
const HIT_RESCAN: HitId = 1;

pub struct CameraApp {
    grab: Option<std::process::Child>,
    frame: Option<(i32, i32, Vec<u8>)>, // (w, h, RGB888)
    frames: u32,                        // successful preview reloads this session
    fails: u32,
    online: bool,
    detail: String,
    anim: u32,
    paused: bool,
    /// Live AE readout from /tmp/preview.meta
    shut: u32,
    gain: u32, // 1/64 units
    mean: u32,
    last_meta_len: u64,
    last_rgb_mtime_ns: u128,
}

impl CameraApp {
    pub fn new() -> CameraApp {
        CameraApp {
            grab: None,
            frame: None,
            frames: 0,
            fails: 0,
            online: false,
            detail: String::new(),
            anim: 0,
            paused: false,
            shut: 0,
            gain: 0,
            mean: 0,
            last_meta_len: 0,
            last_rgb_mtime_ns: 0,
        }
    }

    fn spawn_stream(&mut self) {
        if self.grab.is_some() || self.paused {
            return;
        }
        // Do NOT force CAMGRAB_GAIN — that overrode AE and locked 16x noise.
        // camgrab --stream owns AE via /tmp/camgrab_exp across frames.
        match std::process::Command::new(CAMGRAB)
            .args(["/tmp/frame.raw", "14", "0", "--stream"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(child) => {
                self.grab = Some(child);
                self.detail.clear();
            }
            Err(_) => {
                self.online = false;
                self.detail = "camgrab not deployed".into();
            }
        }
    }

    fn kill_stream(&mut self) {
        if let Some(mut child) = self.grab.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    fn load_preview_if_new(&mut self) {
        // Reload when mtime changes (stream rewrites the file each frame).
        let meta = match std::fs::metadata(PREVIEW_RGB) {
            Ok(m) => m,
            Err(_) => return,
        };
        let mtime_ns = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        if mtime_ns == 0 || mtime_ns == self.last_rgb_mtime_ns {
            return;
        }
        let data = match std::fs::read(PREVIEW_RGB) {
            Ok(d) if d.len() >= 8 => d,
            _ => return,
        };
        let w = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as i32;
        let h = u32::from_le_bytes([data[4], data[5], data[6], data[7]]) as i32;
        let need = (w as usize) * (h as usize) * 3;
        if w <= 0 || h <= 0 || data.len() < 8 + need {
            return;
        }
        self.frame = Some((w, h, data[8..8 + need].to_vec()));
        self.last_rgb_mtime_ns = mtime_ns;
        self.frames = self.frames.wrapping_add(1);
        self.online = true;
        self.load_meta();
    }

    fn load_meta(&mut self) {
        let s = match std::fs::read_to_string(PREVIEW_META) {
            Ok(s) => s,
            Err(_) => return,
        };
        // shut=2000 gain=256 mean=100 w=480 h=360
        for part in s.split_whitespace() {
            if let Some(v) = part.strip_prefix("shut=") {
                self.shut = v.parse().unwrap_or(self.shut);
            } else if let Some(v) = part.strip_prefix("gain=") {
                self.gain = v.parse().unwrap_or(self.gain);
            } else if let Some(v) = part.strip_prefix("mean=") {
                self.mean = v.parse().unwrap_or(self.mean);
            }
        }
        self.last_meta_len = s.len() as u64;
    }

    /// One-shot still: copy latest preview RGB, or fire camgrab once.
    fn snap_to(&mut self, path: &str) -> ControlResult {
        // Prefer live preview buffer if we have one.
        if let Some((w, h, buf)) = &self.frame {
            let mut out = Vec::with_capacity(8 + buf.len());
            out.extend_from_slice(&(*w as u32).to_le_bytes());
            out.extend_from_slice(&(*h as u32).to_le_bytes());
            out.extend_from_slice(buf);
            return match std::fs::write(path, &out) {
                Ok(()) => ControlResult::Ok(format!("camera snap {path} {}x{} rgb", w, h)),
                Err(e) => ControlResult::Err(format!("write {path}: {e}")),
            };
        }
        if std::path::Path::new(PREVIEW_RGB).exists() {
            return match std::fs::copy(PREVIEW_RGB, path) {
                Ok(_) => ControlResult::Ok(format!("camera snap {path} (preview file)")),
                Err(e) => ControlResult::Err(format!("copy preview: {e}")),
            };
        }
        // Fire a one-shot grab into /tmp then copy.
        let raw = "/tmp/frame.raw";
        match std::process::Command::new(CAMGRAB)
            .args([raw, "14", "0"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
        {
            Ok(st) if st.success() => match std::fs::copy(raw, path) {
                Ok(_) => ControlResult::Ok(format!("camera snap {path} (camgrab)")),
                Err(e) => ControlResult::Err(format!("copy raw: {e}")),
            },
            Ok(st) => ControlResult::Err(format!("camgrab exit {st}")),
            Err(e) => ControlResult::Err(format!("camgrab: {e}")),
        }
    }
}

impl App for CameraApp {
    fn title(&self) -> &'static str {
        "CAMERA"
    }
    fn accent(&self) -> u32 {
        MAGENTA
    }
    fn tick_ms(&self) -> u64 {
        // Poll preview file often; stream process writes on its own pace.
        100
    }

    fn control(&mut self, op: &str, arg: &str, _ctx: &mut Ctx) -> ControlResult {
        let op = op.to_ascii_lowercase();
        match op.as_str() {
            "start" | "stream" | "on" => {
                self.paused = false;
                self.spawn_stream();
                ControlResult::Ok("camera stream starting".into())
            }
            "stop" | "off" | "pause" => {
                self.paused = true;
                self.kill_stream();
                ControlResult::Ok("camera stream stopped".into())
            }
            "snap" | "shot" | "capture" => {
                let path = if arg.trim().is_empty() {
                    "/tmp/cam-snap.rgb"
                } else {
                    arg.trim()
                };
                self.snap_to(path)
            }
            "status" | "" => ControlResult::Ok(format!(
                "camera online={} frames={} fails={} paused={} shut={} gain={}",
                self.online, self.frames, self.fails, self.paused, self.shut, self.gain
            )),
            _ => ControlResult::Unhandled,
        }
    }

    fn on_enter(&mut self) {
        self.paused = false;
        self.spawn_stream();
    }

    fn on_leave(&mut self) {
        self.kill_stream();
    }

    fn tick(&mut self, _ctx: &mut Ctx) {
        self.anim = self.anim.wrapping_add(1);

        // Reap a dead stream process and restart (unless paused).
        if let Some(child) = &mut self.grab {
            if let Ok(Some(status)) = child.try_wait() {
                self.grab = None;
                if !status.success() {
                    self.fails = self.fails.wrapping_add(1);
                    self.online = false;
                    self.detail = "stream exited".into();
                }
                if !self.paused {
                    self.spawn_stream();
                }
            }
        } else if !self.paused {
            self.spawn_stream();
        }

        self.load_preview_if_new();
    }

    fn draw(&mut self, c: &mut Canvas, area: Rect, hits: &mut HitMap, _ctx: &Ctx) {
        let vf = Rect::new(area.x, area.y + 8, area.w, area.h - 16);
        c.fill(vf.x, vf.y, vf.w, vf.h, mix(BG, 0x000000, 90));

        // Live frame — fit-width, letterbox vertically if needed.
        if let Some((fw, fh, buf)) = &self.frame {
            let dw = vf.w;
            let dh = (dw * *fh) / (*fw).max(1);
            let dh = dh.min(vf.h);
            let dy = vf.y + (vf.h - dh) / 2;
            c.blit_rgb(vf.x, dy, dw, dh, buf, *fw, *fh);
        } else {
            c.textg(
                vf.x + vf.w / 2 - 60,
                vf.y + vf.h / 2 - 8,
                "NO SIGNAL",
                TEXT_DIM,
                1,
            );
        }

        c.corners(vf.x, vf.y, vf.w, vf.h, MAGENTA, 26);

        // HUD
        c.text(vf.x + 18, vf.y + 12, "SP2509 // STREAM", TEXT2, 1);
        let gain_x = if self.gain > 0 {
            self.gain as f32 / 64.0
        } else {
            0.0
        };
        let expline = if self.shut > 0 {
            format!(
                "S {}  G {:.1}x  Y {}  FR {}",
                self.shut, gain_x, self.mean, self.frames
            )
        } else {
            format!("AE …  FR {}", self.frames)
        };
        c.text(vf.x + 18, vf.y + 34, &expline, TEXT_DIM, 1);

        let capturing = self.grab.is_some() && !self.paused;
        let dotcol = if capturing && (self.anim % 4 < 2) {
            RED
        } else {
            mix(BG, RED, 120)
        };
        let rec = Rect::new(vf.x + vf.w - 118, vf.y + 12, 100, 30);
        c.neonbox(rec.x, rec.y, rec.w, rec.h, dotcol);
        c.fill(rec.x + 12, rec.y + 11, 8, 8, dotcol);
        c.text(
            rec.x + 30,
            rec.y + 7,
            if self.paused { "HOLD" } else { "LIVE" },
            dotcol,
            1,
        );

        let (label, col) = if self.paused {
            ("PREVIEW PAUSED", AMBER)
        } else if self.online {
            ("SENSOR LIVE", GREEN)
        } else if self.frame.is_some() {
            ("RECONNECTING", AMBER)
        } else {
            ("SENSOR OFFLINE", RED)
        };
        let bw = 300;
        let bx = vf.x + vf.w / 2 - bw / 2;
        let by = vf.y + vf.h - 52;
        c.fill(bx, by, bw, 40, mix(BG, col, 40));
        c.neonbox(bx, by, bw, 40, col);
        c.textg(bx + 20, by + 9, label, col, 1);
        hits.add(Rect::new(bx, by, bw, 40), HIT_RESCAN);
        if !self.detail.is_empty() {
            c.text(vf.x + 18, vf.y + vf.h - 8, &self.detail, RED, 1);
        } else {
            c.text(
                vf.x + 18,
                vf.y + vf.h - 8,
                "tap status to pause/resume",
                TEXT_DIM,
                1,
            );
        }
    }

    fn on_tap(&mut self, id: HitId, _ctx: &mut Ctx) -> bool {
        if id == HIT_RESCAN {
            self.paused = !self.paused;
            if self.paused {
                self.kill_stream();
            } else {
                self.spawn_stream();
            }
            return true;
        }
        false
    }
}
