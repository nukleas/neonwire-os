//! NETWORK app — the full Wi-Fi manager (M6). Port of neui.c draw_net/net_tap/
//! net_tick/net_join: wpa ctrl socket, scan list, join + DHCP state machine,
//! stack bring-up via wifi-up2.sh, PSK entry through the TextPrompt keyboard.

use std::time::Instant;

use neon_gfx::canvas::{mix, Canvas};
use neon_gfx::font_data::{FONT_H, FONT_W};
use neon_gfx::geom::Rect;
use neon_gfx::theme::*;

use super::{App, Ctx, HitId, HitMap};
use crate::widgets::{PromptResult, TextPrompt, KB_BASE};
use crate::wpa::{spawn_sh, wlan_present, Ap, Wpa};

const LAB: &str = "/mnt/sd/linux-lab";

const HIT_BTN: HitId = 1; // BRING UP or RESCAN
const HIT_ROW0: HitId = 0x10; // ..+9

struct Join {
    ssid: String,
    wpa: bool,
    dhcp_started: bool,
    since: Instant,
}

pub struct NetworkApp {
    wpa: Wpa,
    aps: Vec<Ap>,
    scan_sent: Option<Instant>,
    stack_starting: Option<Instant>,
    joining: Option<Join>,
    kb: Option<TextPrompt>,
    kb_target: Option<(String, bool)>, // (ssid, wpa) awaiting psk
    btn_is_bringup: bool,
}

impl NetworkApp {
    pub fn new() -> NetworkApp {
        NetworkApp {
            wpa: Wpa::default(),
            aps: Vec::new(),
            scan_sent: None,
            stack_starting: None,
            joining: None,
            kb: None,
            kb_target: None,
            btn_is_bringup: false,
        }
    }

    fn send_scan(&mut self) {
        if self.wpa.cmd("SCAN").is_some() {
            self.scan_sent = Some(Instant::now());
        }
    }

    /// Port of net_join: reuse a saved network id when the ssid is known.
    fn join(&mut self, ssid: &str, psk: &str, wpa: bool, ctx: &mut Ctx) {
        let known_id = self.wpa.list_networks().iter().find(|(_, s)| s == ssid).map(|(i, _)| *i);
        let id = match known_id {
            Some(id) => id,
            None => {
                let Some(out) = self.wpa.cmd("ADD_NETWORK") else {
                    ctx.set_toast("supplicant not responding");
                    return;
                };
                let id: i32 = out.trim().parse().unwrap_or(-1);
                self.wpa.cmd(&format!("SET_NETWORK {id} ssid \"{ssid}\""));
                if wpa {
                    let ok = self
                        .wpa
                        .cmd(&format!("SET_NETWORK {id} psk \"{psk}\""))
                        .map(|o| o.starts_with("OK"))
                        .unwrap_or(false);
                    if !ok {
                        ctx.set_toast("psk rejected (8..63 chars)");
                        return;
                    }
                } else {
                    self.wpa.cmd(&format!("SET_NETWORK {id} key_mgmt NONE"));
                }
                id
            }
        };
        self.wpa.cmd(&format!("SELECT_NETWORK {id}"));
        self.wpa.cmd("SAVE_CONFIG");
        spawn_sh(&format!("cp /tmp/wpa.conf {LAB}/wpa.conf 2>/dev/null"));
        self.joining = Some(Join {
            ssid: ssid.to_string(),
            wpa,
            dhcp_started: false,
            since: Instant::now(),
        });
        ctx.set_toast(format!("joining {ssid} ..."));
    }

    fn status_fields(&mut self) -> (String, String) {
        match self.wpa.cmd("STATUS") {
            Some(st) => (
                Wpa::field(&st, "wpa_state").unwrap_or_default(),
                Wpa::field(&st, "ssid").unwrap_or_default(),
            ),
            None => (String::new(), String::new()),
        }
    }
}

impl App for NetworkApp {
    fn title(&self) -> &'static str {
        "NETWORK"
    }

    fn accent(&self) -> u32 {
        GREEN
    }

    fn on_enter(&mut self) {
        // fresh scan whenever the app opens (the C UI made you tap RESCAN)
        if wlan_present() && self.wpa.available() && self.scan_sent.is_none() {
            self.send_scan();
        }
    }

    /// Port of net_tick — drives bring-up, scan results, join/DHCP.
    fn tick(&mut self, ctx: &mut Ctx) {
        if let Some(since) = self.stack_starting {
            if wlan_present() && self.wpa.available() {
                self.stack_starting = None;
                self.send_scan();
            } else if since.elapsed().as_secs() > 45 {
                self.stack_starting = None;
            }
            return;
        }
        if !wlan_present() {
            return;
        }
        if self.scan_sent.is_some_and(|t| t.elapsed().as_secs() >= 3) {
            if let Some(aps) = self.wpa.scan_results() {
                if !aps.is_empty() {
                    self.aps = aps;
                    self.scan_sent = None;
                }
            }
        }
        let Some(join) = &mut self.joining else {
            return;
        };
        let timed_out = join.since.elapsed().as_secs() > 40;
        let (state, _) = self.status_fields();
        let join = self.joining.as_mut().unwrap();
        if state == "COMPLETED" {
            if !join.dhcp_started {
                join.dhcp_started = true;
                // devpts must be mounted or telnetd dies after one connection (L1
                // has /dev/ptmx but nothing mounts devpts).
                spawn_sh(&format!(
                    "udhcpc -i wlan0 -n -q -s {LAB}/udhcpc.script >/tmp/udhcpc.log 2>&1; \
                     [ -d /dev/pts ] || mkdir -p /dev/pts; \
                     mount | grep -q ' /dev/pts ' || mount -t devpts devpts /dev/pts 2>/dev/null; \
                     pgrep telnetd >/dev/null || telnetd -l /bin/sh -p 23 2>/dev/null"
                ));
            } else if let Some(ip) = ctx.snap.wlan_ip.clone() {
                ctx.set_toast(format!("ONLINE  {ip}  (telnet ready)"));
                self.joining = None;
                return;
            }
        }
        if timed_out {
            self.joining = None;
            ctx.set_toast("join timed out — check psk");
        }
    }

    fn draw(&mut self, c: &mut Canvas, area: Rect, hits: &mut HitMap, ctx: &Ctx) {
        let (fw, fh) = (FONT_W as i32, FONT_H as i32);
        c.panel(area.x, area.y + 8, area.w, area.h - 16, GREEN, "NETWORK // WIFI");
        let x = area.x + 24;
        let mut y = area.y + 30;
        let lh = fh + 8;

        let up = wlan_present();
        let ctrl = up && self.wpa.available();
        let (state, cssid) =
            if ctrl { self.status_fields() } else { (String::new(), String::new()) };
        let ip = ctx.snap.wlan_ip.clone();

        // status line
        c.text(x, y, "wlan0", TEXT2, 1);
        if !up {
            let (msg, col) = if self.stack_starting.is_some() {
                ("BRINGING UP CONSYS STACK ...", AMBER)
            } else {
                ("STACK DOWN", RED)
            };
            c.text(x + 90, y, msg, col, 1);
        } else if state == "COMPLETED" {
            c.textg(x + 90, y, "CONNECTED", GREEN, 1);
            let info = format!("{}   {}", cssid, ip.as_deref().unwrap_or("(dhcp...)"));
            c.text(x + 90 + 10 * fw + 16, y, &info, WHITE, 1);
        } else {
            let s = if !state.is_empty() {
                state.as_str()
            } else if ctrl {
                "IDLE"
            } else {
                "SUPPLICANT DOWN"
            };
            c.text(x + 90, y, s, if self.joining.is_some() { AMBER } else { MAGENTA }, 1);
            if let Some(j) = &self.joining {
                c.text(x + 90 + s.len() as i32 * fw + 16, y, &j.ssid, TEXT2, 1);
            }
        }

        // the one action button, top right
        let (bw, bh) = (150, 40);
        let br = Rect::new(area.x + area.w - bw - 18, area.y + 20, bw, bh);
        self.btn_is_bringup = false;
        let scanning = self.scan_sent.is_some();
        if !up || !ctrl {
            if self.stack_starting.is_none() {
                draw_button(c, br, "BRING UP", AMBER);
                hits.add(br, HIT_BTN);
                self.btn_is_bringup = true;
            }
        } else {
            draw_button(c, br, if scanning { "SCANNING." } else { "RESCAN" }, CYAN);
            if !scanning {
                hits.add(br, HIT_BTN);
            }
        }
        y += lh + 6;
        c.hline(x, y, area.w - 48, mix(BG, MAGENTA, 60));
        y += 10;

        if !up {
            y += 6;
            if self.stack_starting.is_some() {
                c.text(x, y, &format!("running {LAB}/wifi-up2.sh"), TEXT2, 1);
                y += lh;
                if let Ok(log) = std::fs::read_to_string("/tmp/wifi-up2.log") {
                    let tail: Vec<&str> =
                        log.lines().filter(|l| !l.is_empty()).rev().take(6).collect();
                    for l in tail.iter().rev() {
                        let clipped: String = l.chars().take(96).collect();
                        c.text(x, y, &clipped, TEXT_DIM, 1);
                        y += fh + 4;
                    }
                }
            } else {
                c.text(x, y, "consys / wlan driver not up on this boot.", TEXT2, 1);
                y += lh;
                c.text(x, y, "tap BRING UP to run the full stack bring-up", TEXT2, 1);
                y += lh;
                c.text(x, y, "(firmware stage > wmt_loader > wmtctl2 > wpa_supplicant)", TEXT_DIM, 1);
            }
        } else {
            // scan list
            c.text(x, y, &format!("{:<26} {:>8}   SECURITY", "SSID", "SIGNAL"), TEXT2, 1);
            y += lh - 2;
            c.hline(x, y - 4, area.w - 48, mix(BG, MAGENTA, 40));
            let rowh = lh + 6;
            let maxrows = (((area.y + area.h - 40 - y) / rowh) as usize).min(10);
            if self.aps.is_empty() {
                let msg = if scanning { "scanning ..." } else { "no scan results — tap RESCAN" };
                c.text(x, y + 8, msg, TEXT_DIM, 1);
            }
            for (i, ap) in self.aps.iter().take(maxrows).enumerate() {
                let ry = y + i as i32 * rowh;
                let cur = !cssid.is_empty() && ap.ssid == cssid && state == "COMPLETED";
                if cur {
                    c.fill(x - 6, ry - 3, area.w - 60, rowh - 2, mix(BG, GREEN, 16));
                }
                let bars = match ap.rssi {
                    s if s > -45 => 4,
                    s if s > -55 => 3,
                    s if s > -67 => 2,
                    s if s > -75 => 1,
                    _ => 0,
                };
                for b in 0..4i32 {
                    let col = if b <= bars { if cur { GREEN } else { CYAN } } else { mix(BG, BORDER, 90) };
                    c.fill(x + b * 7, ry + fh - 3 - b * 3, 5, 3 + b * 3, col);
                }
                let line = format!(
                    "{:<26.26} {:>5} dBm  {}",
                    ap.ssid,
                    ap.rssi,
                    if ap.wpa { "WPA2" } else { "open" }
                );
                if cur {
                    c.textg(x + 38, ry, &line, GREEN, 1);
                } else {
                    c.text(x + 38, ry, &line, TEXT, 1);
                }
                hits.add(Rect::new(x - 6, ry - 3, area.w - 60, rowh - 2), HIT_ROW0 + i as u32);
            }
            if !self.aps.is_empty() {
                c.text(x, area.y + area.h - 36, "tap a network to join", TEXT_DIM, 1);
            }
        }

        // PSK keyboard modal — drawn + registered last (topmost)
        if let Some(kb) = &self.kb {
            kb.draw(c, hits);
        }
    }

    fn on_tap(&mut self, id: HitId, ctx: &mut Ctx) -> bool {
        // keyboard owns everything while open
        if let Some(kb) = &mut self.kb {
            if id >= KB_BASE {
                match kb.on_tap(id) {
                    PromptResult::Open => {}
                    PromptResult::Cancelled => self.kb = None,
                    PromptResult::Submitted(psk) => {
                        self.kb = None;
                        if let Some((ssid, wpa)) = self.kb_target.take() {
                            self.join(&ssid, &psk, wpa, ctx);
                        }
                    }
                }
            }
            return true;
        }
        match id {
            HIT_BTN => {
                if self.btn_is_bringup {
                    spawn_sh(&format!("sh {LAB}/wifi-up2.sh >/tmp/wifi-up2.log 2>&1"));
                    self.stack_starting = Some(Instant::now());
                    ctx.set_toast("bringing up wifi stack...");
                } else {
                    self.send_scan();
                    ctx.set_toast("scanning...");
                }
                true
            }
            id if (HIT_ROW0..HIT_ROW0 + 10).contains(&id) => {
                let i = (id - HIT_ROW0) as usize;
                let Some(ap) = self.aps.get(i) else {
                    return false;
                };
                let (ssid, wpa) = (ap.ssid.clone(), ap.wpa);
                let known = self.wpa.list_networks().iter().any(|(_, s)| *s == ssid);
                if known || !wpa {
                    self.join(&ssid, "", wpa, ctx);
                } else {
                    self.kb = Some(TextPrompt::new(format!("[ JOIN {ssid} ]"), 8, "JOIN"));
                    self.kb_target = Some((ssid, wpa));
                }
                true
            }
            _ => false,
        }
    }
}

fn draw_button(c: &mut Canvas, r: Rect, label: &str, acc: u32) {
    c.fill(r.x, r.y, r.w, r.h, mix(BG, acc, 20));
    c.neonbox(r.x, r.y, r.w, r.h, acc);
    c.corners(r.x, r.y, r.w, r.h, acc, 8);
    c.text(
        r.x + r.w / 2 - label.len() as i32 * FONT_W as i32 / 2,
        r.y + r.h / 2 - FONT_H as i32 / 2,
        label,
        acc,
        1,
    );
}
