//! SYSTEM app — M4 scope: the info tab (port of neui.c draw_system).
//! PROCESS/STORAGE/KERNLOG sub-tabs + tools overlay arrive in M5.

use neon_gfx::canvas::Canvas;
use neon_gfx::geom::Rect;
use neon_gfx::theme::*;

use super::{App, Ctx, HitId, HitMap};

pub struct SystemApp;

impl App for SystemApp {
    fn title(&self) -> &'static str {
        "SYSTEM"
    }

    fn accent(&self) -> u32 {
        CYAN
    }

    fn draw(&mut self, c: &mut Canvas, area: Rect, _hits: &mut HitMap, ctx: &Ctx) {
        let s = ctx.snap;
        c.panel(area.x, area.y + 8, area.w, area.h - 16, CYAN, "SYSTEM // CORE");
        let x = area.x + 28;
        let mut y = area.y + 40;
        let mut row = |c: &mut Canvas, k: &str, v: &str, col: u32| {
            c.text(x, y, k, TEXT_DIM, 1);
            c.text(x + 130, y, v, col, 1);
            y += 30;
        };
        row(c, "HOST", &s.host, TEXT);
        row(c, "KERNEL", &s.kernel, TEXT);
        row(c, "ARCH", &s.machine, TEXT);
        row(c, "CPU", &format!("{} cores", s.cpus), TEXT);
        let up = format!("{}h {:02}m {:02}s", s.uptime_s / 3600, (s.uptime_s % 3600) / 60, s.uptime_s % 60);
        row(c, "UPTIME", &up, GREEN);
        row(c, "LOAD", &s.load1, TEXT2);
        let used = s.mem_total_kb - s.mem_avail_kb;
        row(
            c,
            "MEM",
            &format!("{} / {} MB", used / 1024, s.mem_total_kb / 1024),
            TEXT2,
        );
        let pct = if s.mem_total_kb > 0 { (used * 100 / s.mem_total_kb) as i32 } else { 0 };
        c.bar(x + 130, y - 4, area.w - 200, 18, pct, CYAN);
    }

    fn on_tap(&mut self, _id: HitId, _ctx: &mut Ctx) -> bool {
        false
    }
}
