//! CAMERA app — cyberpunk viewfinder HUD. Until the SP2509 pipeline lands
//! (experiments/camera/), the sensor state comes from spawning `camprobe`
//! and the viewfinder shows an honest SENSOR OFFLINE state.

use neon_gfx::canvas::{mix, Canvas};
use neon_gfx::geom::Rect;
use neon_gfx::theme::*;

use super::{App, Ctx, HitId, HitMap};

const CAMPROBE: &str = "/mnt/sd/linux-lab/camprobe";
const HIT_RESCAN: HitId = 1;

#[derive(Clone, Copy, PartialEq)]
pub enum CamState {
    Unknown,
    Probing,
    Online,
    Offline,
}

pub struct CameraApp {
    state: CamState,
    detail: String,
    probe: Option<std::process::Child>,
    anim: u32,
}

impl CameraApp {
    pub fn new() -> CameraApp {
        CameraApp { state: CamState::Unknown, detail: String::new(), probe: None, anim: 0 }
    }

    fn spawn_probe(&mut self) {
        if self.probe.is_some() {
            return;
        }
        match std::process::Command::new(CAMPROBE)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(child) => {
                self.state = CamState::Probing;
                self.probe = Some(child);
            }
            Err(_) => {
                self.state = CamState::Offline;
                self.detail = "camprobe not deployed".into();
            }
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
        250 // reticle animation
    }

    fn on_enter(&mut self) {
        self.spawn_probe();
    }

    fn tick(&mut self, _ctx: &mut Ctx) {
        self.anim = self.anim.wrapping_add(1);
        // reap a finished probe without blocking
        if let Some(child) = &mut self.probe {
            if let Ok(Some(status)) = child.try_wait() {
                let mut child = self.probe.take().unwrap();
                let mut out = String::new();
                if let Some(so) = child.stdout.as_mut() {
                    use std::io::Read;
                    let _ = so.read_to_string(&mut out);
                }
                self.detail = out.trim().to_string();
                self.state = if status.success() { CamState::Online } else { CamState::Offline };
            }
        }
    }

    fn draw(&mut self, c: &mut Canvas, area: Rect, hits: &mut HitMap, _ctx: &Ctx) {
        // viewfinder frame
        let vf = Rect::new(area.x, area.y + 8, area.w, area.h - 16);
        c.fill(vf.x, vf.y, vf.w, vf.h, mix(BG, 0x000000, 90));
        c.corners(vf.x, vf.y, vf.w, vf.h, MAGENTA, 26);
        // crosshair reticle + sweep
        let (cx, cy) = (vf.x + vf.w / 2, vf.y + vf.h / 2);
        let pulse = 18 + ((self.anim % 8) as i32 - 4).abs() * 2;
        c.hline(cx - pulse, cy, pulse - 6, MAGENTA);
        c.hline(cx + 6, cy, pulse - 6, MAGENTA);
        c.vline(cx, cy - pulse, pulse - 6, MAGENTA);
        c.vline(cx, cy + 6, pulse - 6, MAGENTA);
        c.corners(cx - 60, cy - 44, 120, 88, mix(BG, MAGENTA, 150), 10);

        // HUD readouts
        c.text(vf.x + 18, vf.y + 12, "SP2509 // 1600x1200 RAW10", TEXT2, 1);
        c.text(vf.x + 18, vf.y + 34, "EXP --  ISO --  MCLK 24M", TEXT_DIM, 1);
        let rec = Rect::new(vf.x + vf.w - 110, vf.y + 12, 92, 30);
        c.neonbox(rec.x, rec.y, rec.w, rec.h, mix(BG, RED, 110));
        c.text(rec.x + 18, rec.y + 7, "REC", mix(BG, RED, 150), 1);

        // state block
        let (label, col) = match self.state {
            CamState::Online => ("SENSOR ONLINE", GREEN),
            CamState::Probing => ("PROBING...", AMBER),
            _ => ("SENSOR OFFLINE", RED),
        };
        let bw = 360;
        let bx = cx - bw / 2;
        let by = vf.y + vf.h - 96;
        c.fill(bx, by, bw, 62, mix(BG, col, 26));
        c.neonbox(bx, by, bw, 62, col);
        c.textg(bx + 24, by + 10, label, col, 2);
        if !self.detail.is_empty() {
            let d = if self.detail.len() > 40 { &self.detail[..40] } else { &self.detail };
            c.text(bx + 24, by + 42, d, TEXT_DIM, 1);
        }
        hits.add(Rect::new(bx, by, bw, 62), HIT_RESCAN);
        c.text(vf.x + 18, vf.y + vf.h - 24, "tap status to re-probe", TEXT_DIM, 1);
    }

    fn on_tap(&mut self, id: HitId, _ctx: &mut Ctx) -> bool {
        if id == HIT_RESCAN && self.state != CamState::Probing {
            self.spawn_probe();
            return true;
        }
        false
    }
}
