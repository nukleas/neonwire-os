//! HOUSE — Home Assistant control panel.
//! Layout: left status/metrics, center entity grid (tap to toggle),
//! right detail. Mirrors INTEL's triple-pane density language.
//!
//! Config: /mnt/sd/linux-lab/hass.{url,token[,entities]}

use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use neon_gfx::canvas::{mix, Canvas};
use neon_gfx::font_data::{FONT_H, FONT_W};
use neon_gfx::geom::Rect;
use neon_gfx::theme::*;

use super::{App, Ctx, HitId, HitMap};
use crate::hass::{self, Entity, Snapshot, FILTERS};

const HIT_REFRESH: HitId = 1;
const HIT_CHIP0: HitId = 0x10;
const HIT_TILE0: HitId = 0x100;
const HIT_SCROLL_UP: HitId = 0x20;
const HIT_SCROLL_DN: HitId = 0x21;
const HIT_ACTION: HitId = 0x30; // toggle selected from detail

const SIDE_W: i32 = 188;
const DETAIL_W: i32 = 210;
const TILE_H: i32 = FONT_H as i32 + 18; // single-line tile
const POLL_SECS: u64 = 20;

enum Msg {
    Ok(Snapshot),
    Err(String),
    ToggleOk(String, String), // entity_id, new_state guess
    ToggleErr(String),
}

pub struct HouseApp {
    filter: usize,
    scroll: usize,
    selected: Option<usize>, // index into filtered view
    snap: Option<Snapshot>,
    status: String,
    loading: bool,
    toggling: bool,
    rx: Option<Receiver<Msg>>,
    auto_at: Instant,
}

impl HouseApp {
    pub fn new() -> HouseApp {
        HouseApp {
            filter: 0,
            scroll: 0,
            selected: None,
            snap: None,
            status: "need hass.url + hass.token on SD".into(),
            loading: false,
            toggling: false,
            rx: None,
            auto_at: Instant::now(),
        }
    }

    fn spawn_fetch(&mut self) {
        if self.loading {
            return;
        }
        self.loading = true;
        self.status = "fetching HA...".into();
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        thread::spawn(move || {
            let msg = match hass::fetch() {
                Ok(s) => Msg::Ok(s),
                Err(e) => Msg::Err(e.to_string()),
            };
            let _ = tx.send(msg);
        });
    }

    fn spawn_toggle(&mut self, entity_id: String, currently_on: bool) {
        if self.toggling {
            return;
        }
        self.toggling = true;
        let (tx, rx) = mpsc::channel();
        // reuse channel if idle, else leave old rx (shouldn't happen)
        self.rx = Some(rx);
        thread::spawn(move || {
            let want_on = !currently_on;
            let msg = match hass::turn(&entity_id, want_on) {
                Ok(()) => {
                    let st = if entity_id.starts_with("script.") {
                        "fired".into()
                    } else if want_on {
                        "on".into()
                    } else {
                        "off".into()
                    };
                    Msg::ToggleOk(entity_id, st)
                }
                Err(e) => Msg::ToggleErr(e.to_string()),
            };
            let _ = tx.send(msg);
        });
    }

    fn poll_rx(&mut self, ctx: &mut Ctx) {
        let Some(rx) = &self.rx else {
            return;
        };
        match rx.try_recv() {
            Ok(Msg::Ok(s)) => {
                let n = s.total;
                let on = s.lights_on;
                self.snap = Some(s);
                self.selected = None;
                self.scroll = 0;
                self.loading = false;
                self.rx = None;
                self.auto_at = Instant::now();
                self.status = format!("{n} ents  {on} lights on");
                ctx.set_toast(format!("HA  {n} entities"));
            }
            Ok(Msg::Err(e)) => {
                self.loading = false;
                self.rx = None;
                self.status = e.chars().take(42).collect();
                ctx.set_toast("HA fetch failed");
            }
            Ok(Msg::ToggleOk(id, st)) => {
                self.toggling = false;
                self.rx = None;
                if let Some(snap) = &mut self.snap {
                    if let Some(e) = snap.entities.iter_mut().find(|e| e.entity_id == id) {
                        e.state = st.clone();
                        e.detail = st.clone();
                    }
                    snap.lights_on = snap
                        .entities
                        .iter()
                        .filter(|e| e.domain == "light" && e.state == "on")
                        .count();
                    snap.switches_on = snap
                        .entities
                        .iter()
                        .filter(|e| e.domain == "switch" && e.state == "on")
                        .count();
                }
                ctx.set_toast(format!("{} -> {}", short_id(&id), st));
                // soft refresh soon
                self.auto_at = Instant::now() - Duration::from_secs(POLL_SECS.saturating_sub(3));
            }
            Ok(Msg::ToggleErr(e)) => {
                self.toggling = false;
                self.rx = None;
                ctx.set_toast(format!("toggle fail: {}", e.chars().take(28).collect::<String>()));
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.loading = false;
                self.toggling = false;
                self.rx = None;
            }
        }
    }

    fn filtered<'a>(&'a self) -> Vec<&'a Entity> {
        let Some(snap) = &self.snap else {
            return Vec::new();
        };
        let key = FILTERS[self.filter].0;
        snap.entities
            .iter()
            .filter(|e| key == "all" || e.domain == key)
            .collect()
    }
}

impl App for HouseApp {
    fn title(&self) -> &'static str {
        "HOUSE"
    }

    fn accent(&self) -> u32 {
        GREEN
    }

    fn tick_ms(&self) -> u64 {
        if self.loading || self.toggling {
            200
        } else {
            1000
        }
    }

    fn on_enter(&mut self) {
        if self.snap.is_none() && !self.loading {
            self.spawn_fetch();
        }
    }

    fn tick(&mut self, ctx: &mut Ctx) {
        self.poll_rx(ctx);
        if !self.loading && !self.toggling && self.auto_at.elapsed() >= Duration::from_secs(POLL_SECS)
        {
            self.spawn_fetch();
        }
    }

    fn draw(&mut self, c: &mut Canvas, area: Rect, hits: &mut HitMap, _ctx: &Ctx) {
        let (fw, fh) = (FONT_W as i32, FONT_H as i32);

        c.fill(area.x, area.y, area.w, area.h, mix(BG, BG2, 40));
        c.neonbox(area.x, area.y, area.w, area.h, mix(BG, GREEN, 80));
        c.corners(area.x, area.y, area.w, area.h, GREEN, 12);

        let inner = Rect::new(area.x + 3, area.y + 3, area.w - 6, area.h - 6);
        let side = Rect::new(inner.x, inner.y, SIDE_W, inner.h);
        let detail = Rect::new(inner.x + inner.w - DETAIL_W, inner.y, DETAIL_W, inner.h);
        let main = Rect::new(
            side.x + side.w + 1,
            inner.y,
            detail.x - side.x - side.w - 2,
            inner.h,
        );

        draw_side(c, side, self, hits, fw, fh);
        draw_grid(c, main, self, hits, fw, fh);
        draw_detail(c, detail, self, hits, fw, fh);
    }

    fn on_tap(&mut self, id: HitId, ctx: &mut Ctx) -> bool {
        match id {
            HIT_REFRESH => {
                self.spawn_fetch();
                true
            }
            HIT_SCROLL_UP => {
                self.scroll = self.scroll.saturating_sub(4);
                true
            }
            HIT_SCROLL_DN => {
                self.scroll = self.scroll.saturating_add(4);
                true
            }
            i if (HIT_CHIP0..HIT_CHIP0 + FILTERS.len() as u32).contains(&i) => {
                let ni = (i - HIT_CHIP0) as usize;
                if ni != self.filter {
                    self.filter = ni;
                    self.selected = None;
                    self.scroll = 0;
                }
                true
            }
            i if (HIT_TILE0..HIT_TILE0 + 64).contains(&i) => {
                let list = self.filtered();
                let idx = self.scroll + (i - HIT_TILE0) as usize;
                if idx >= list.len() {
                    return true;
                }
                // second tap on same selected toggleable -> toggle
                if self.selected == Some(idx) && list[idx].toggleable {
                    let e = list[idx];
                    let on = e.state == "on";
                    self.spawn_toggle(e.entity_id.clone(), on);
                } else {
                    self.selected = Some(idx);
                }
                true
            }
            HIT_ACTION => {
                let list = self.filtered();
                if let Some(sel) = self.selected {
                    if let Some(e) = list.get(sel) {
                        if e.toggleable {
                            let on = e.state == "on";
                            self.spawn_toggle(e.entity_id.clone(), on);
                        } else {
                            ctx.set_toast("not toggleable");
                        }
                    }
                }
                true
            }
            _ => false,
        }
    }
}

// ---- side panel ------------------------------------------------------------

fn draw_side(c: &mut Canvas, r: Rect, app: &HouseApp, hits: &mut HitMap, fw: i32, fh: i32) {
    c.fill(r.x, r.y, r.w, r.h, mix(BG, BG2, 200));
    c.vline(r.x + r.w - 1, r.y, r.h, mix(BG, BORDER, 180));

    let mut y = r.y + 6;
    c.text(r.x + 8, y, "> HOUSE", GREEN, 1);
    y += fh + 2;
    c.hline(r.x + 6, y, r.w - 12, mix(BG, GREEN, 50));
    y += 6;

    // refresh
    let br = Rect::new(r.x + 6, y, r.w - 14, fh + 10);
    let busy = app.loading || app.toggling;
    let bcol = if busy { TEXT_DIM } else { CYAN };
    c.fill(br.x, br.y, br.w, br.h, mix(BG, bcol, 18));
    c.neonbox(br.x, br.y, br.w, br.h, mix(BG, bcol, 120));
    let bl = if app.loading {
        "..."
    } else if app.toggling {
        "WAIT"
    } else {
        "REFRESH"
    };
    c.text(br.x + (br.w - bl.len() as i32 * fw) / 2, br.y + 5, bl, bcol, 1);
    if !busy {
        hits.add(br, HIT_REFRESH);
    }
    y += br.h + 8;

    // summary cards
    if let Some(s) = &app.snap {
        draw_mini(c, r.x + 6, y, r.w - 14, fh * 2 + 6, "LIGHTS ON", &s.lights_on.to_string(), AMBER, fw, fh);
        y += fh * 2 + 12;
        draw_mini(c, r.x + 6, y, r.w - 14, fh * 2 + 6, "SWITCHES ON", &s.switches_on.to_string(), CYAN, fw, fh);
        y += fh * 2 + 12;
        draw_mini(c, r.x + 6, y, r.w - 14, fh * 2 + 6, "SHOWN", &s.total.to_string(), TEXT2, fw, fh);
        y += fh * 2 + 12;

        if let Some(w) = &s.weather {
            c.text(r.x + 8, y, "WEATHER", TEXT_DIM, 1);
            y += fh;
            for chunk in w.as_bytes().chunks(((r.w - 16) / fw).max(6) as usize).take(2) {
                c.text(r.x + 8, y, std::str::from_utf8(chunk).unwrap_or(""), BLUE, 1);
                y += fh;
            }
            y += 4;
        }
        if let Some(cl) = &s.climate {
            c.text(r.x + 8, y, "CLIMATE", TEXT_DIM, 1);
            y += fh;
            for chunk in cl.as_bytes().chunks(((r.w - 16) / fw).max(6) as usize).take(2) {
                c.text(r.x + 8, y, std::str::from_utf8(chunk).unwrap_or(""), MAGENTA, 1);
                y += fh;
            }
        }
    } else {
        c.text(r.x + 8, y, "no data yet", TEXT_DIM, 1);
        y += fh + 6;
        c.text(r.x + 8, y, "put url + token at", TEXT_MUTED, 1);
        y += fh;
        c.text(r.x + 8, y, "hass.url / hass.token", TEXT_MUTED, 1);
    }

    // footer
    let fy = r.y + r.h - fh - 4;
    c.hline(r.x + 6, fy - 4, r.w - 12, mix(BG, BORDER, 140));
    let st: String = app.status.chars().take(((r.w - 12) / fw) as usize).collect();
    c.text(r.x + 6, fy, &st, TEXT_DIM, 1);
}

fn draw_mini(
    c: &mut Canvas,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    label: &str,
    value: &str,
    accent: u32,
    fw: i32,
    fh: i32,
) {
    c.fill(x, y, w, h, PANEL);
    c.neonbox(x, y, w, h, mix(BG, accent, 70));
    c.text_tracked(x + 6, y + 2, label, TEXT_DIM, 1, 1);
    let maxc = ((w - 12) / fw).max(2) as usize;
    let v: String = value.chars().take(maxc).collect();
    c.textg(x + 6, y + 2 + fh - 2, &v, accent, 1);
}

// ---- entity grid -----------------------------------------------------------

fn draw_grid(c: &mut Canvas, r: Rect, app: &mut HouseApp, hits: &mut HitMap, fw: i32, fh: i32) {
    c.fill(r.x, r.y, r.w, r.h, BG);

    let mut y = r.y + 4;
    c.text(r.x + 8, y, "> ENTITIES", GREEN, 1);
    y += fh + 2;
    c.hline(r.x + 4, y, r.w - 8, mix(BG, GREEN, 50));
    y += 4;

    // domain chips
    let chip_h = fh + 6;
    let mut cx = r.x + 6;
    let row0 = y;
    let mut row = 0;
    for (i, (_, label)) in FILTERS.iter().enumerate() {
        let cw = label.len() as i32 * fw + 14;
        if cx + cw > r.x + r.w - 6 {
            row += 1;
            if row > 1 {
                break;
            }
            cx = r.x + 6;
        }
        let cy = row0 + row * (chip_h + 4);
        let chip = Rect::new(cx, cy, cw, chip_h);
        let on = i == app.filter;
        if on {
            c.fill(chip.x, chip.y, chip.w, chip.h, mix(BG, GREEN, 30));
        }
        c.neonbox(chip.x, chip.y, chip.w, chip.h, if on { GREEN } else { BORDER });
        c.text(chip.x + 7, chip.y + 3, label, if on { GREEN } else { TEXT_DIM }, 1);
        hits.add(chip, HIT_CHIP0 + i as u32);
        cx += cw + 4;
    }
    y = row0 + (row + 1) * (chip_h + 4) + 4;
    c.hline(r.x + 4, y, r.w - 8, mix(BG, BORDER, 140));
    y += 4;

    // tiles — 2 columns
    let list_h = r.y + r.h - y - 4;
    let cols = 2i32;
    let gap = 6;
    let tw = (r.w - 10 - gap) / cols;
    let rows_fit = (list_h / (TILE_H + 4)).max(1) as usize;
    let capacity = rows_fit * cols as usize;
    let n_list = app.filtered().len();
    let max_scroll = n_list.saturating_sub(capacity);
    if app.scroll > max_scroll {
        app.scroll = max_scroll;
    }
    // align scroll to even columns
    app.scroll = (app.scroll / cols as usize) * cols as usize;
    let scroll = app.scroll;
    let selected = app.selected;
    let loading = app.loading;

    if n_list == 0 {
        c.text(
            r.x + 8,
            y + 8,
            if loading { "loading..." } else { "no entities" },
            TEXT_DIM,
            1,
        );
        return;
    }

    let page: Vec<Entity> = app
        .filtered()
        .into_iter()
        .skip(scroll)
        .take(capacity)
        .cloned()
        .collect();
    for (i, e) in page.iter().enumerate() {
        let abs = scroll + i;
        let col = (i as i32) % cols;
        let row = (i as i32) / cols;
        let x = r.x + 4 + col * (tw + gap);
        let ty = y + row * (TILE_H + 4);
        let tile = Rect::new(x, ty, tw, TILE_H);
        let on = e.state == "on" || e.state == "heat_cool" || e.state == "heat" || e.state == "cool";
        let sel = selected == Some(abs);
        let dcol = hass::domain_color(&e.domain);
        let bg = if sel {
            mix(BG, dcol, 50)
        } else if on {
            mix(BG, dcol, 28)
        } else {
            PANEL
        };
        c.fill(tile.x, tile.y, tile.w, tile.h, bg);
        c.neonbox(tile.x, tile.y, tile.w, tile.h, if sel { dcol } else { mix(BG, dcol, 90) });
        if on {
            c.fill(tile.x, tile.y + 3, 3, tile.h - 6, dcol);
        }
        // single line: name (clipped) + state right
        let st = if e.state.len() > 8 {
            e.state.chars().take(6).collect::<String>()
        } else {
            e.state.clone()
        };
        let st_w = st.len() as i32 * fw;
        let name_max = ((tile.w - 16 - st_w - fw) / fw).max(4) as usize;
        let name: String = e.name.chars().take(name_max).collect();
        let ly = tile.y + (tile.h - fh) / 2;
        c.text(tile.x + 10, ly, &name, if on { CYANHI } else { TEXT }, 1);
        let scol = if on { dcol } else { TEXT_DIM };
        c.text(tile.x + tile.w - st_w - 8, ly, &st, scol, 1);
        hits.add(tile, HIT_TILE0 + i as u32);
    }

    if n_list > capacity {
        let track = list_h;
        let kh = (track as usize * capacity / n_list).max(10) as i32;
        let ky = y + (track as usize * scroll / n_list) as i32;
        c.fill(r.x + r.w - 5, y, 3, track, mix(BG, GREEN, 40));
        c.fill(r.x + r.w - 5, ky, 3, kh, GREEN);
        let strip = Rect::new(r.x + r.w - 22, y, 20, track);
        let split = y + track * 2 / 5;
        hits.add(Rect::new(strip.x, y, strip.w, split - y), HIT_SCROLL_UP);
        hits.add(Rect::new(strip.x, split, strip.w, y + track - split), HIT_SCROLL_DN);
    }
}

// ---- detail ----------------------------------------------------------------

fn draw_detail(c: &mut Canvas, r: Rect, app: &HouseApp, hits: &mut HitMap, fw: i32, fh: i32) {
    c.fill(r.x, r.y, r.w, r.h, mix(BG, BG2, 200));
    c.vline(r.x, r.y, r.h, mix(BG, BORDER, 180));

    let mut y = r.y + 6;
    c.text(r.x + 8, y, "> DETAIL", MAGENTA, 1);
    y += fh + 2;
    c.hline(r.x + 6, y, r.w - 12, mix(BG, MAGENTA, 50));
    y += 8;

    let list = app.filtered();
    let Some(sel) = app.selected else {
        c.text(r.x + 8, y, "tap a tile", TEXT_DIM, 1);
        y += fh + 4;
        c.text(r.x + 8, y, "again=toggle", TEXT_MUTED, 1);
        y = r.y + r.h - fh * 2 - 8;
        c.hline(r.x + 6, y, r.w - 12, mix(BG, BORDER, 140));
        y += 6;
        c.text(r.x + 8, y, "HOME ASST", GREEN, 1);
        return;
    };
    let Some(e) = list.get(sel) else {
        return;
    };

    let dcol = hass::domain_color(&e.domain);
    c.text(r.x + 8, y, &e.domain.to_uppercase(), dcol, 1);
    y += fh + 2;

    // name wrap
    let maxc = ((r.w - 16) / fw).max(6) as usize;
    for (i, chunk) in e.name.as_bytes().chunks(maxc).enumerate() {
        if i > 2 {
            break;
        }
        c.textg(r.x + 8, y, std::str::from_utf8(chunk).unwrap_or(""), CYANHI, 1);
        y += fh;
    }
    y += 4;

    c.text(r.x + 8, y, "STATE", TEXT_DIM, 1);
    y += fh;
    c.textg(r.x + 8, y, &e.state, dcol, 1);
    y += fh + 2;
    if !e.detail.is_empty() && e.detail != e.state {
        for chunk in e.detail.as_bytes().chunks(maxc).take(2) {
            c.text(r.x + 8, y, std::str::from_utf8(chunk).unwrap_or(""), TEXT2, 1);
            y += fh;
        }
    }
    y += 6;
    c.hline(r.x + 6, y, r.w - 12, mix(BG, BORDER, 140));
    y += 6;

    // entity_id small
    c.text(r.x + 8, y, "ID", TEXT_DIM, 1);
    y += fh;
    for chunk in e.entity_id.as_bytes().chunks(maxc).take(3) {
        c.text(r.x + 8, y, std::str::from_utf8(chunk).unwrap_or(""), TEXT_MUTED, 1);
        y += fh;
    }

    if e.toggleable {
        let bh = fh + 12;
        let by = r.y + r.h - bh - 8;
        let btn = Rect::new(r.x + 8, by, r.w - 16, bh);
        let on = e.state == "on";
        let (label, col) = if e.domain == "script" {
            ("RUN", PURPLE)
        } else if on {
            ("TURN OFF", RED)
        } else {
            ("TURN ON", GREEN)
        };
        c.fill(btn.x, btn.y, btn.w, btn.h, mix(BG, col, 24));
        c.neonbox(btn.x, btn.y, btn.w, btn.h, col);
        c.text(
            btn.x + (btn.w - label.len() as i32 * fw) / 2,
            btn.y + (btn.h - fh) / 2,
            label,
            col,
            1,
        );
        hits.add(btn, HIT_ACTION);
    }
}

fn short_id(entity_id: &str) -> String {
    entity_id.split('.').nth(1).unwrap_or(entity_id).chars().take(18).collect()
}
