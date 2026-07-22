//! The shell: event loop, left rail nav, status bar, toast.
//!
//! Loop contract (port of neui.c main): poll() the touch fd with timeout =
//! time-to-next-tick, drain ALL queued events on wake, dispatch taps through
//! the HitMap (topmost wins), tick the active app + collectors, redraw only
//! when dirty, present via the PAN-cycle.

use std::time::{Duration, Instant};

use neon_gfx::fb::Fb;
use neon_gfx::geom::Rect;
use neon_gfx::input::{poll_fds, Touch};
use neon_gfx::theme::*;

use crate::apps::home::{Home, HIT_LAUNCH0, TILES};
use crate::apps::{App, ControlResult, Ctx, HitMap};
use crate::backlight::Backlight;
use crate::collectors::Collectors;
use crate::control::{self, BlCmd, Cmd, ViewTarget, APP_NAMES};
use crate::hass;
use crate::keys::{Key, Keys};
use crate::power::{self, PowerMgr, PowerState};
use crate::rail::{self, HIT_RAIL_APP0, RAIL_W};
use crate::statusbar::{self, BAR_H, HIT_HOME};

/// App index for ASSISTANT — must match Shell::apps order / TILES.
const APP_ASSIST: usize = 8;

enum Screen {
    Home,
    App(usize),
}

pub struct Shell {
    fb: Fb,
    touch: Option<Touch>,
    keys: Option<Keys>,
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
        let keys = Keys::open().map_err(|e| eprintln!("keys: {e}")).ok();
        Shell {
            fb,
            touch,
            keys,
            apps: vec![
                Box::new(crate::apps::system::SystemApp::new()),
                Box::new(crate::apps::network::NetworkApp::new()),
                Box::new(crate::apps::house::HouseApp::new()),
                Box::new(crate::apps::files::FilesApp::new()),
                Box::new(crate::apps::intel::IntelApp::new()),
                Box::new(crate::apps::camera::CameraApp::new()),
                Box::new(crate::apps::music::MusicApp::new()),
                Box::new(crate::apps::songs::SongsApp::new()),
                Box::new(crate::apps::assistant::AssistantApp::new()),
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

    fn go_home(&mut self) {
        if !matches!(self.screen, Screen::Home) {
            self.leave_current_app();
            self.screen = Screen::Home;
            self.dirty = true;
        }
    }

    fn current_view_name(&self) -> String {
        match self.screen {
            Screen::Home => "home".into(),
            Screen::App(i) => APP_NAMES.get(i).unwrap_or(&"?").to_string(),
        }
    }

    /// Drain agent/shell commands from `/tmp/neonwire.cmd`.
    /// All non-empty lines run in order (chips may queue multi-step sequences).
    fn poll_control(&mut self) {
        let Some(raw) = control::take_pending() else {
            return;
        };
        let mut replies: Vec<String> = Vec::new();
        for line in raw.lines() {
            let t = line.trim();
            if t.is_empty() || t.starts_with('#') {
                continue;
            }
            let Some(cmd) = control::parse_line(t) else {
                continue;
            };
            let reply = self.dispatch_cmd(cmd);
            eprintln!("ctl: {t} -> {reply}");
            replies.push(reply);
        }
        if !replies.is_empty() {
            control::write_reply(&replies.join(" | "));
            self.dirty = true;
        }
    }

    fn on_key(&mut self, key: Key) {
        // Any key counts as activity for backlight.
        let woke = self.backlight.on_activity();
        match key {
            Key::VolumeUp => {
                // Vol+ → ASSIST (always, even if it only woke the screen)
                self.open_app(APP_ASSIST);
                self.toast = Some(("ASSIST".into(), Instant::now()));
                self.dirty = true;
                eprintln!("key: VOL+ -> assist");
            }
            Key::VolumeDown => {
                self.go_home();
                self.toast = Some(("HOME".into(), Instant::now()));
                self.dirty = true;
                eprintln!("key: VOL- -> home");
            }
            Key::Power => {
                if woke {
                    // first press after blank only woke the panel
                    eprintln!("key: POWER woke");
                } else if self.backlight.is_blanked() {
                    self.backlight.wake();
                    eprintln!("key: POWER wake");
                } else {
                    self.backlight.blank();
                    eprintln!("key: POWER blank");
                }
                self.dirty = true;
            }
            Key::Other(c) => {
                eprintln!("key: other code={c}");
            }
        }
    }

    fn dispatch_cmd(&mut self, cmd: Cmd) -> String {
        match cmd {
            Cmd::Help => control::help_text(),
            Cmd::Unknown(m) => format!("err: {m}"),
            Cmd::Toast(msg) => {
                self.toast = Some((msg.clone(), Instant::now()));
                self.backlight.wake();
                format!("ok toast: {msg}")
            }
            Cmd::View(ViewTarget::Home) => {
                self.go_home();
                self.backlight.wake();
                "ok view=home".into()
            }
            Cmd::View(ViewTarget::App(i)) => {
                if i >= self.apps.len() {
                    return format!("err: app index {i} out of range");
                }
                self.open_app(i);
                self.backlight.wake();
                format!("ok view={}", APP_NAMES.get(i).unwrap_or(&"?"))
            }
            Cmd::Backlight(BlCmd::On) | Cmd::Backlight(BlCmd::Wake) => {
                self.backlight.wake();
                "ok backlight=on".into()
            }
            Cmd::Backlight(BlCmd::Off) => {
                self.backlight.blank();
                "ok backlight=off".into()
            }
            Cmd::Backlight(BlCmd::Level(n)) => {
                self.backlight.set_level(n);
                format!("ok backlight={n}")
            }
            Cmd::Status => {
                self.write_status_file();
                format!(
                    "ok view={} blanked={} bl={} apps={}",
                    self.current_view_name(),
                    self.backlight.is_blanked(),
                    self.backlight.is_available(),
                    self.apps.len()
                )
            }
            Cmd::Shot(path) => {
                self.backlight.wake();
                self.draw();
                match self.fb.shot(&path) {
                    Ok(()) => format!("ok shot {path}"),
                    Err(e) => format!("err shot: {e}"),
                }
            }
            Cmd::Music(rest) => self.forward_app(6, &rest),
            Cmd::Songs(rest) => self.forward_app(7, &rest),
            Cmd::Camera(rest) => self.forward_app(5, &rest),
            Cmd::Ha(rest) => self.dispatch_ha(&rest),
        }
    }

    fn forward_app(&mut self, idx: usize, rest: &str) -> String {
        if idx >= self.apps.len() {
            return format!("err: no app {idx}");
        }
        let mut parts = rest.splitn(2, char::is_whitespace);
        let op = parts.next().unwrap_or("").trim();
        let arg = parts.next().unwrap_or("").trim();
        let snap = &self.collectors.snap;
        let mut ctx = Ctx {
            snap,
            toast: &mut self.toast,
        };
        match self.apps[idx].control(op, arg, &mut ctx) {
            ControlResult::Ok(s) => {
                self.dirty = true;
                format!("ok {s}")
            }
            ControlResult::Err(s) => format!("err {s}"),
            ControlResult::Unhandled => {
                format!(
                    "err {} unhandled op '{op}' — try status",
                    APP_NAMES.get(idx).unwrap_or(&"app")
                )
            }
        }
    }

    fn dispatch_ha(&mut self, rest: &str) -> String {
        let mut parts = rest.splitn(2, char::is_whitespace);
        let op = parts.next().unwrap_or("").trim().to_ascii_lowercase();
        let arg = parts.next().unwrap_or("").trim();
        match op.as_str() {
            "refresh" | "fetch" | "status" | "" => match hass::fetch() {
                Ok(s) => {
                    let mut line = format!(
                        "ok ha entities={} lights_on={} switches_on={}",
                        s.total, s.lights_on, s.switches_on
                    );
                    if let Some(w) = &s.weather {
                        line.push_str(&format!(" weather={w}"));
                    }
                    line
                }
                Err(e) => format!("err ha: {e}"),
            },
            "list" => match hass::fetch() {
                Ok(s) => {
                    let mut out = format!("ok ha list ({})\n", s.total);
                    for e in s.entities.iter().take(40) {
                        out.push_str(&format!(
                            "{} [{}] {}\n",
                            e.entity_id,
                            e.state,
                            e.name.chars().take(32).collect::<String>()
                        ));
                    }
                    out
                }
                Err(e) => format!("err ha: {e}"),
            },
            "toggle" => {
                if arg.is_empty() {
                    return "err ha toggle needs entity_id".into();
                }
                match hass::toggle(arg) {
                    Ok(()) => format!("ok ha toggled {arg}"),
                    Err(e) => format!("err ha: {e}"),
                }
            }
            "on" => {
                if arg.is_empty() {
                    return "err ha on needs entity_id".into();
                }
                match hass::turn(arg, true) {
                    Ok(()) => format!("ok ha on {arg}"),
                    Err(e) => format!("err ha: {e}"),
                }
            }
            "off" => {
                if arg.is_empty() {
                    return "err ha off needs entity_id".into();
                }
                match hass::turn(arg, false) {
                    Ok(()) => format!("ok ha off {arg}"),
                    Err(e) => format!("err ha: {e}"),
                }
            }
            _ => format!("err ha unknown op '{op}' (refresh|list|toggle|on|off)"),
        }
    }

    fn write_status_file(&self) {
        let view = self.current_view_name();
        let host = if self.collectors.snap.host.is_empty() {
            "dl7006"
        } else {
            self.collectors.snap.host.as_str()
        };
        let batt = self
            .collectors
            .snap
            .batt_pct
            .map(|p| p.to_string())
            .unwrap_or_else(|| "?".into());
        let charging = self.collectors.snap.batt_charging;
        let line = format!(
            "view={view} host={host} batt={batt}% charging={charging} blanked={} apps={} control=v1\n",
            self.backlight.is_blanked(),
            self.apps.len()
        );
        let _ = std::fs::write(control::STATUS_PATH, line);
    }

    fn on_tap(&mut self, sx: i32, sy: i32) {
        let Some(id) = self.hits.hit(sx, sy) else {
            return;
        };

        // global nav (rail)
        if id == HIT_HOME {
            self.go_home();
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

            // Poll touch + hardware keys together.
            let mut fds = Vec::new();
            let mut touch_i = None;
            let mut keys_i = None;
            if let Some(t) = &self.touch {
                touch_i = Some(fds.len());
                fds.push(t.fd());
            }
            if let Some(k) = &self.keys {
                keys_i = Some(fds.len());
                fds.push(k.fd());
            }
            let ready = if fds.is_empty() {
                std::thread::sleep(Duration::from_millis(wait as u64));
                0
            } else {
                poll_fds(&fds, wait)
            };

            if let (Some(ti), Some(t)) = (touch_i, self.touch.as_mut()) {
                if ready & (1 << ti) != 0 {
                    let (w, h) = (self.fb.xres as i32, self.fb.yres as i32);
                    for (sx, sy) in t.drain(w, h) {
                        eprintln!("tap screen({sx},{sy})");
                        if self.backlight.on_activity() {
                            continue;
                        }
                        self.on_tap(sx, sy);
                    }
                }
            }
            let mut key_events = Vec::new();
            if let (Some(ki), Some(k)) = (keys_i, self.keys.as_mut()) {
                if ready & (1 << ki) != 0 {
                    key_events = k.drain();
                }
            }
            for key in key_events {
                self.on_key(key);
            }
            // Agent control plane — after taps/keys so chips can queue cmds.
            self.poll_control();
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
                self.write_status_file();
                self.dirty = true;
            }
            if self.dirty && !self.backlight.is_blanked() {
                self.draw();
                self.dirty = false;
            }
        }
    }

    /// Headless video capture: apply nav taps, warm up (so a started song's
    /// visualizer has real data), then dump one BGRA back-buffer frame per
    /// tick into `dir/fNNNN.raw` at `fps` for `secs`. Assemble host-side.
    /// Reuses the same buffer `shot()` writes — the only capture path proven
    /// to work on this command-mode panel.
    /// Draw the current screen into the back buffer (for record helpers).
    pub fn draw_public(&mut self) {
        self.draw();
    }

    /// Dump the back buffer to a file (for record helpers).
    pub fn fb_shot(&self, path: &str) -> std::io::Result<()> {
        self.fb.shot(path)
    }

    pub fn record(&mut self, taps: &[(i32, i32)], dir: &str, secs: u32, fps: u32) {
        let _ = std::fs::create_dir_all(dir);
        self.draw();
        for &(sx, sy) in taps {
            self.on_tap(sx, sy);
            self.draw(); // refresh hitmap for the new screen before the next tap
            // let async work (song eval + sample load + pcm open) settle
            for _ in 0..20 {
                std::thread::sleep(Duration::from_millis(100));
                self.collectors.refresh();
                if let Screen::App(i) = self.screen {
                    let mut ctx = Ctx { snap: &self.collectors.snap, toast: &mut self.toast };
                    self.apps[i].tick(&mut ctx);
                }
                self.draw();
            }
        }
        // If a media app is present, wait until it actually starts playing and
        // report its position at frame 0 so a host-rendered audio track can be
        // trimmed to match (REC_T0_MS). Non-media captures just report 0.
        let t0_ms = if let Screen::App(i) = self.screen {
            let mut waited = 0;
            while self.apps[i].media_pos_ms().is_none() && waited < 60 {
                std::thread::sleep(Duration::from_millis(100));
                let mut ctx = Ctx { snap: &self.collectors.snap, toast: &mut self.toast };
                self.apps[i].tick(&mut ctx);
                waited += 1;
            }
            self.apps[i].media_pos_ms().unwrap_or(0)
        } else {
            0
        };
        println!("REC_T0_MS {t0_ms}");
        let frame_ms = 1000 / fps.max(1);
        let total = secs * fps;
        println!("REC {total} frames -> {dir} ({fps}fps {secs}s)");
        for f in 0..total {
            let t0 = Instant::now();
            self.collectors.refresh();
            if let Screen::App(i) = self.screen {
                let mut ctx = Ctx { snap: &self.collectors.snap, toast: &mut self.toast };
                self.apps[i].tick(&mut ctx);
            }
            self.draw();
            let path = format!("{dir}/f{f:04}.raw");
            if let Err(e) = self.fb.shot(&path) {
                eprintln!("record: {path}: {e}");
                break;
            }
            let spent = t0.elapsed().as_millis() as u64;
            if (frame_ms as u64) > spent {
                std::thread::sleep(Duration::from_millis(frame_ms as u64 - spent));
            }
        }
        println!("REC done");
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
