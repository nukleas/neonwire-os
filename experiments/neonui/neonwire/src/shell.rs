//! The shell: event loop, left rail nav, status bar, toast.
//!
//! Loop contract (port of neui.c main): poll() the touch fd with timeout =
//! time-to-next-tick, drain ALL queued events on wake, dispatch taps through
//! the HitMap (topmost wins), tick the active app + collectors, redraw only
//! when dirty, present via the PAN-cycle.

use std::time::{Duration, Instant};

use neon_gfx::fb::Fb;
use neon_gfx::geom::Rect;
use neon_gfx::input::{poll_fd, Touch};
use neon_gfx::theme::*;

use crate::apps::home::{Home, HIT_LAUNCH0, TILES};
use crate::apps::{App, Ctx, HitMap};
use crate::backlight::Backlight;
use crate::collectors::Collectors;
use crate::power::{self, PowerMgr, PowerState};
use crate::rail::{self, HIT_RAIL_APP0, RAIL_W};
use crate::statusbar::{self, BAR_H, HIT_HOME};

enum Screen {
    Home,
    App(usize),
}

pub struct Shell {
    fb: Fb,
    touch: Option<Touch>,
    apps: Vec<Box<dyn App>>,
    screen: Screen,
    home: Home,
    hits: HitMap,
    collectors: Collectors,
    toast: Option<(String, Instant)>,
    backlight: Backlight,
    power: PowerMgr,
    power_state: PowerState,
    dirty: bool,
}

impl Shell {
    pub fn new(fb: Fb, touch: Option<Touch>) -> Shell {
        Shell {
            fb,
            touch,
            apps: vec![
                Box::new(crate::apps::system::SystemApp::new()),
                Box::new(crate::apps::network::NetworkApp::new()),
                Box::new(crate::apps::house::HouseApp::new()),
                Box::new(crate::apps::files::FilesApp::new()),
                Box::new(crate::apps::intel::IntelApp::new()),
                Box::new(crate::apps::camera::CameraApp::new()),
                Box::new(crate::apps::music::MusicApp::new()),
            ],
            screen: Screen::Home,
            home: Home,
            hits: HitMap::default(),
            collectors: Collectors::new(),
            toast: None,
            backlight: Backlight::new(),
            power: PowerMgr::new(),
            power_state: PowerState::Ok,
            dirty: true,
        }
    }

    /// Content to the right of the nav rail, under the status bar.
    fn content_area(&self) -> Rect {
        let pad = 8;
        Rect::new(
            RAIL_W + pad,
            BAR_H + pad,
            self.fb.xres as i32 - RAIL_W - pad * 2,
            self.fb.yres as i32 - BAR_H - pad * 2,
        )
    }

    fn active_app(&self) -> Option<usize> {
        match self.screen {
            Screen::Home => None,
            Screen::App(i) => Some(i),
        }
    }

    fn draw(&mut self) {
        let area = self.content_area();
        self.hits.clear();
        let active = self.active_app();
        let (title, accent) = match self.screen {
            Screen::Home => ("", CYAN),
            Screen::App(i) => (self.apps[i].title(), self.apps[i].accent()),
        };
        let mut c = self.fb.canvas();
        c.background();

        // status + rail always
        {
            let snap = &self.collectors.snap;
            statusbar::draw(&mut c, snap, title, accent, &mut self.hits);
            rail::draw(&mut c, active, &mut self.hits);
        }

        match self.screen {
            Screen::Home => {
                let ctx = Ctx { snap: &self.collectors.snap, toast: &mut self.toast };
                self.home.draw(&mut c, area, &mut self.hits, &ctx);
            }
            Screen::App(i) => {
                let ctx = Ctx { snap: &self.collectors.snap, toast: &mut self.toast };
                self.apps[i].draw(&mut c, area, &mut self.hits, &ctx);
            }
        }

        // low-battery warning banner
        if self.power_state == PowerState::Low {
            let pct = self.collectors.snap.batt_pct.unwrap_or(0);
            let msg = format!("LOW BATTERY {pct}% - CONNECT CHARGER");
            let bw = msg.len() as i32 * 11 + 40;
            let x = RAIL_W + (c.w - RAIL_W - bw) / 2;
            c.fill(x, BAR_H + 4, bw, 28, neon_gfx::canvas::mix(BG, RED, 60));
            c.neonbox(x, BAR_H + 4, bw, 28, RED);
            c.text(x + 20, BAR_H + 6, &msg, RED, 1);
        }
        // toast
        if let Some((msg, at)) = &self.toast {
            if at.elapsed() < Duration::from_secs(3) {
                let tw = msg.len() as i32 * 11 + 40;
                let x = RAIL_W + (c.w - RAIL_W - tw) / 2;
                let y = c.h - 42;
                c.fill(x, y, tw, 32, neon_gfx::canvas::mix(BG, AMBER, 40));
                c.neonbox(x, y, tw, 32, AMBER);
                c.text(x + 20, y + 4, msg, AMBER, 1);
            }
        }
        c.scanlines(0, 0, c.w, c.h);
        self.fb.present();
    }

    fn draw_shutdown(&mut self) {
        let pct = self.collectors.snap.batt_pct.unwrap_or(0);
        let mut c = self.fb.canvas();
        c.background();
        let (cx, cy) = (c.w / 2, c.h / 2);
        c.textg(cx - 11 * 9, cy - 40, "BATTERY CRITICAL", RED, 2);
        c.text(cx - 15 * 6, cy + 6, &format!("{pct}% - SHUTTING DOWN SAFELY"), TEXT, 1);
        c.text(cx - 13 * 6, cy + 30, "flushing disks + poweroff", TEXT_DIM, 1);
        c.scanlines(0, 0, c.w, c.h);
        self.fb.present();
    }

    fn leave_current_app(&mut self) {
        if let Screen::App(i) = self.screen {
            if i < self.apps.len() {
                self.apps[i].on_leave();
            }
        }
    }

    fn open_app(&mut self, i: usize) {
        if i >= self.apps.len() {
            return;
        }
        if let Screen::App(cur) = self.screen {
            if cur == i {
                return;
            }
        }
        self.leave_current_app();
        self.screen = Screen::App(i);
        self.apps[i].on_enter();
        self.dirty = true;
    }

    fn on_tap(&mut self, sx: i32, sy: i32) {
        let Some(id) = self.hits.hit(sx, sy) else {
            return;
        };

        // global nav (rail)
        if id == HIT_HOME {
            if !matches!(self.screen, Screen::Home) {
                self.leave_current_app();
                self.screen = Screen::Home;
                self.dirty = true;
            }
            return;
        }
        if (HIT_RAIL_APP0..HIT_RAIL_APP0 + TILES.len() as u32).contains(&id) {
            let i = (id - HIT_RAIL_APP0) as usize;
            self.open_app(i);
            return;
        }

        match self.screen {
            Screen::Home => {
                // compact module launcher on the dashboard
                if (HIT_LAUNCH0..HIT_LAUNCH0 + TILES.len() as u32).contains(&id) {
                    let i = (id - HIT_LAUNCH0) as usize;
                    self.open_app(i);
                }
            }
            Screen::App(i) => {
                let snap = &self.collectors.snap;
                let mut ctx = Ctx { snap, toast: &mut self.toast };
                if self.apps[i].on_tap(id, &mut ctx) {
                    self.dirty = true;
                }
            }
        }
    }

    pub fn run(&mut self) -> ! {
        self.fb.print_shot_line();
        self.draw();
        let mut last_tick = Instant::now();
        loop {
            let tick_ms = match self.screen {
                Screen::Home => 1000,
                Screen::App(i) => self.apps[i].tick_ms(),
            };
            let elapsed = last_tick.elapsed().as_millis() as u64;
            let wait = tick_ms.saturating_sub(elapsed).max(10) as i32;

            let mut taps = Vec::new();
            if let Some(t) = &mut self.touch {
                if poll_fd(t.fd(), wait) {
                    let (w, h) = (self.fb.xres as i32, self.fb.yres as i32);
                    taps = t.drain(w, h);
                }
            } else {
                std::thread::sleep(Duration::from_millis(wait as u64));
            }
            for (sx, sy) in taps {
                eprintln!("tap screen({sx},{sy})");
                if self.backlight.on_activity() {
                    continue;
                }
                self.on_tap(sx, sy);
            }
            self.backlight.tick();

            if last_tick.elapsed().as_millis() as u64 >= tick_ms {
                last_tick = Instant::now();
                self.collectors.refresh();
                let snap = &self.collectors.snap;
                self.power_state = self.power.update(snap.batt_pct, snap.batt_charging);
                if self.power_state == PowerState::Shutdown {
                    self.backlight.wake();
                    self.draw_shutdown();
                    power::power_off();
                }
                if let Screen::App(i) = self.screen {
                    let snap = &self.collectors.snap;
                    let mut ctx = Ctx { snap, toast: &mut self.toast };
                    self.apps[i].tick(&mut ctx);
                }
                self.dirty = true;
            }
            if self.dirty && !self.backlight.is_blanked() {
                self.draw();
                self.dirty = false;
            }
        }
    }

    pub fn shot(&mut self, taps: &[(i32, i32)], ticks: u32, path: Option<&str>) {
        self.fb.print_shot_line();
        self.draw();
        for &(sx, sy) in taps {
            println!("TAP ({sx},{sy})");
            self.on_tap(sx, sy);
            for _ in 0..ticks {
                std::thread::sleep(Duration::from_secs(1));
                self.collectors.refresh();
                if let Screen::App(i) = self.screen {
                    let snap = &self.collectors.snap;
                    let mut ctx = Ctx { snap, toast: &mut self.toast };
                    self.apps[i].tick(&mut ctx);
                }
            }
            self.draw();
        }
        if let Some(p) = path {
            if let Err(e) = self.fb.shot(p) {
                eprintln!("shot: {e}");
            }
        }
        println!("RESULT ok");
    }
}
