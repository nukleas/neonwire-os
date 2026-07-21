//! INTEL — OCINT civic feed on the tablet.
//! Layout mirrors ocint's dashboard columns:
//!   left  = METRICS (StatsPanel)
//!   main  = INTEL FEED (IntelFeed + category chips)
//!   right = DETAIL (selected ItemDetailDrawer-style pane)
//!
//! Data: https://ocint.app/api/stats + /api/items (public, 60s poll).

use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use neon_gfx::canvas::{mix, Canvas};
use neon_gfx::font_data::{FONT_H, FONT_W};
use neon_gfx::geom::Rect;
use neon_gfx::theme::*;

use super::{App, Ctx, HitId, HitMap};
use crate::ocint::{self, Item, Snapshot, Stats, FILTERS};

const HIT_REFRESH: HitId = 1;
const HIT_CHIP0: HitId = 0x10;
const HIT_ROW0: HitId = 0x100;
const HIT_SCROLL_UP: HitId = 0x20;
const HIT_SCROLL_DN: HitId = 0x21;
const HIT_DET_UP: HitId = 0x30;
const HIT_DET_DN: HitId = 0x31;

const METRICS_W: i32 = 200;
const DETAIL_W: i32 = 240;
const ROW_H: i32 = 38;
const POLL_SECS: u64 = 60;

enum Msg {
    Ok(Snapshot),
    Err(String),
}

pub struct IntelApp {
    filter: usize, // index into FILTERS
    scroll: usize,
    selected: Option<usize>,
    detail_scroll: usize,
    snap: Option<Snapshot>,
    status: String,
    loading: bool,
    last_ok: Option<Instant>,
    rx: Option<Receiver<Msg>>,
    auto_at: Instant,
}

impl IntelApp {
    pub fn new() -> IntelApp {
        IntelApp {
            filter: 0,
            scroll: 0,
            selected: None,
            detail_scroll: 0,
            snap: None,
            status: "tap REFRESH or wait".into(),
            loading: false,
            last_ok: None,
            rx: None,
            auto_at: Instant::now(),
        }
    }

    fn cat_key(&self) -> &'static str {
        FILTERS[self.filter].0
    }

    fn spawn_fetch(&mut self) {
        if self.loading {
            return;
        }
        self.loading = true;
        self.status = "fetching ocint...".into();
        let cat = self.cat_key().to_string();
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        thread::spawn(move || {
            let cat_opt = if cat == "all" { None } else { Some(cat.as_str()) };
            let msg = match ocint::fetch(cat_opt, 40) {
                Ok(s) => Msg::Ok(s),
                Err(e) => Msg::Err(e.to_string()),
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
                let n = s.items.len();
                self.snap = Some(s);
                self.selected = None;
                self.scroll = 0;
                self.detail_scroll = 0;
                self.loading = false;
                self.rx = None;
                self.last_ok = Some(Instant::now());
                self.auto_at = Instant::now();
                self.status = format!("ok  {n} items");
                ctx.set_toast(format!("OCINT  {n} items"));
            }
            Ok(Msg::Err(e)) => {
                self.loading = false;
                self.rx = None;
                self.status = e.chars().take(48).collect();
                ctx.set_toast("ocint fetch failed");
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.loading = false;
                self.rx = None;
                self.status = "fetch dropped".into();
            }
        }
    }

    fn items(&self) -> &[Item] {
        self.snap.as_ref().map(|s| s.items.as_slice()).unwrap_or(&[])
    }
}

impl App for IntelApp {
    fn title(&self) -> &'static str {
        "INTEL"
    }

    fn accent(&self) -> u32 {
        AMBER // ocint orange family
    }

    fn tick_ms(&self) -> u64 {
        if self.loading {
            250
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
        // 60s auto-refresh like the ocint browser
        if !self.loading && self.auto_at.elapsed() >= Duration::from_secs(POLL_SECS) {
            self.spawn_fetch();
        }
    }

    fn draw(&mut self, c: &mut Canvas, area: Rect, hits: &mut HitMap, _ctx: &Ctx) {
        let (fw, fh) = (FONT_W as i32, FONT_H as i32);

        c.fill(area.x, area.y, area.w, area.h, mix(BG, BG2, 40));
        c.neonbox(area.x, area.y, area.w, area.h, mix(BG, AMBER, 80));
        c.corners(area.x, area.y, area.w, area.h, AMBER, 12);

        let inner = Rect::new(area.x + 3, area.y + 3, area.w - 6, area.h - 6);
        let metrics = Rect::new(inner.x, inner.y, METRICS_W, inner.h);
        let detail = Rect::new(inner.x + inner.w - DETAIL_W, inner.y, DETAIL_W, inner.h);
        let feed = Rect::new(
            metrics.x + metrics.w + 1,
            inner.y,
            detail.x - metrics.x - metrics.w - 2,
            inner.h,
        );

        draw_metrics(c, metrics, self.snap.as_ref().map(|s| &s.stats), &self.status, self.loading, hits, fw, fh);
        draw_feed(c, feed, self, hits, fw, fh);
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
            HIT_DET_UP => {
                self.detail_scroll = self.detail_scroll.saturating_sub(3);
                true
            }
            HIT_DET_DN => {
                self.detail_scroll = self.detail_scroll.saturating_add(3);
                true
            }
            i if (HIT_CHIP0..HIT_CHIP0 + FILTERS.len() as u32).contains(&i) => {
                let ni = (i - HIT_CHIP0) as usize;
                if ni != self.filter {
                    self.filter = ni;
                    self.selected = None;
                    self.scroll = 0;
                    self.spawn_fetch();
                }
                true
            }
            i if (HIT_ROW0..HIT_ROW0 + 64).contains(&i) => {
                let idx = self.scroll + (i - HIT_ROW0) as usize;
                if idx < self.items().len() {
                    self.selected = Some(idx);
                    self.detail_scroll = 0;
                }
                true
            }
            _ => {
                let _ = ctx;
                false
            }
        }
    }
}

// ---- METRICS column (StatsPanel) -------------------------------------------

fn draw_metrics(
    c: &mut Canvas,
    r: Rect,
    stats: Option<&Stats>,
    status: &str,
    loading: bool,
    hits: &mut HitMap,
    fw: i32,
    fh: i32,
) {
    c.fill(r.x, r.y, r.w, r.h, mix(BG, BG2, 200));
    c.vline(r.x + r.w - 1, r.y, r.h, mix(BG, BORDER, 180));

    let mut y = r.y + 6;
    c.text(r.x + 8, y, "> METRICS", AMBER, 1);
    y += fh + 2;
    c.hline(r.x + 6, y, r.w - 12, mix(BG, AMBER, 50));
    y += 6;

    // refresh chip
    let br = Rect::new(r.x + 6, y, r.w - 14, fh + 10);
    let bcol = if loading { TEXT_DIM } else { CYAN };
    c.fill(br.x, br.y, br.w, br.h, mix(BG, bcol, 18));
    c.neonbox(br.x, br.y, br.w, br.h, mix(BG, bcol, 120));
    let bl = if loading { "..." } else { "REFRESH" };
    c.text(br.x + (br.w - bl.len() as i32 * fw) / 2, br.y + 5, bl, bcol, 1);
    if !loading {
        hits.add(br, HIT_REFRESH);
    }
    y += br.h + 8;

    let Some(st) = stats else {
        c.text(r.x + 8, y, "no data yet", TEXT_DIM, 1);
        y += fh + 8;
        let st_line = ocint::ascii(status);
        for (i, chunk) in st_line.as_bytes().chunks(14).enumerate() {
            if y + fh > r.y + r.h - 8 {
                break;
            }
            let s = std::str::from_utf8(chunk).unwrap_or("");
            c.text(r.x + 8, y, s, TEXT_MUTED, 1);
            y += fh;
            if i > 4 {
                break;
            }
        }
        return;
    };

    // key stats 2-col
    let card_w = (r.w - 18) / 2;
    let card_h = fh * 2 + 8;
    draw_mini(c, r.x + 6, y, card_w, card_h, "TODAY", &st.total_today.to_string(), AMBER, fw, fh);
    draw_mini(
        c,
        r.x + 10 + card_w,
        y,
        card_w,
        card_h,
        "7 DAY",
        &st.total_week.to_string(),
        TEXT,
        fw,
        fh,
    );
    y += card_h + 6;

    // alerts row
    if st.active_fires > 0 {
        draw_mini(c, r.x + 6, y, card_w, card_h, "FIRES", &st.active_fires.to_string(), AMBER, fw, fh);
    }
    if st.active_outages > 0 {
        draw_mini(
            c,
            r.x + 10 + card_w,
            y,
            card_w,
            card_h,
            "OUTAGES",
            &st.active_outages.to_string(),
            RED,
            fw,
            fh,
        );
    }
    if st.active_fires > 0 || st.active_outages > 0 {
        y += card_h + 6;
    }

    if let Some(aqi) = st.current_aqi {
        let col = if aqi > 100 { RED } else { GREEN };
        draw_mini(c, r.x + 6, y, r.w - 14, card_h, "AQI", &aqi.to_string(), col, fw, fh);
        y += card_h + 6;
    }

    // weather one-liner
    if let Some(w) = &st.weather_summary {
        c.text(r.x + 8, y, "WEATHER", TEXT_DIM, 1);
        y += fh;
        let line = ocint::ascii(w);
        for chunk in line.as_bytes().chunks(((r.w - 16) / fw).max(8) as usize).take(3) {
            c.text(r.x + 8, y, std::str::from_utf8(chunk).unwrap_or(""), BLUE, 1);
            y += fh;
        }
        y += 4;
    }

    // port
    if st.port_wind.is_some() || st.port_wave.is_some() {
        c.text(r.x + 8, y, "PORT", TEXT_DIM, 1);
        y += fh;
        if let Some(w) = st.port_wind {
            c.text(r.x + 8, y, &format!("wind {:.0} kn", w), CYAN, 1);
            y += fh;
        }
        if let Some(w) = st.port_wave {
            c.text(r.x + 8, y, &format!("wave {:.1} m", w), CYAN, 1);
            y += fh;
        }
        y += 4;
    }

    // by category (top 5 bars)
    c.text(r.x + 8, y, "BY CATEGORY", TEXT_DIM, 1);
    y += fh + 2;
    let mut cats: Vec<(&String, &i64)> = st.by_category.iter().collect();
    cats.sort_by(|a, b| b.1.cmp(a.1));
    let max = cats.first().map(|(_, n)| **n).unwrap_or(1).max(1);
    let bar_w = r.w - 16 - 5 * fw;
    for (cat, n) in cats.iter().take(6) {
        if y + fh + 4 > r.y + r.h - 20 {
            break;
        }
        let label = ocint::cat_label(cat);
        let col = ocint::cat_color(cat);
        c.text(r.x + 8, y, label, col, 1);
        let pct = ((**n * 100) / max) as i32;
        c.bar(r.x + 8 + 4 * fw, y + 6, bar_w, 10, pct, col);
        y += fh + 2;
    }

    // footer status
    let fy = r.y + r.h - fh - 4;
    c.hline(r.x + 6, fy - 4, r.w - 12, mix(BG, BORDER, 140));
    let st_line = ocint::ascii(status);
    c.text(
        r.x + 6,
        fy,
        &st_line.chars().take(((r.w - 12) / fw) as usize).collect::<String>(),
        TEXT_DIM,
        1,
    );
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
    // value on second line, clipped
    let maxc = ((w - 12) / fw).max(2) as usize;
    let v: String = value.chars().take(maxc).collect();
    c.textg(x + 6, y + 2 + fh - 2, &v, accent, 1);
}

// ---- FEED column -----------------------------------------------------------

fn draw_feed(c: &mut Canvas, r: Rect, app: &mut IntelApp, hits: &mut HitMap, fw: i32, fh: i32) {
    c.fill(r.x, r.y, r.w, r.h, BG);

    let mut y = r.y + 4;
    // header
    c.text(r.x + 8, y, "> INTEL FEED", AMBER, 1);
    let count = app.items().len();
    let meta = format!("{count}");
    c.text(r.x + r.w - meta.len() as i32 * fw - 10, y, &meta, TEXT_DIM, 1);
    y += fh + 2;
    c.hline(r.x + 4, y, r.w - 8, mix(BG, AMBER, 50));
    y += 4;

    // category chips (wrap one row, then clip)
    let chip_h = fh + 6;
    let mut cx = r.x + 6;
    let row1_y = y;
    let mut row = 0;
    for (i, (_, label)) in FILTERS.iter().enumerate() {
        let cw = label.len() as i32 * fw + 14;
        if cx + cw > r.x + r.w - 6 {
            row += 1;
            if row > 1 {
                break; // max 2 chip rows
            }
            cx = r.x + 6;
            y += chip_h + 4;
        }
        let cy = row1_y + row * (chip_h + 4);
        let chip = Rect::new(cx, cy, cw, chip_h);
        let on = i == app.filter;
        let col = if on { AMBER } else { BORDER };
        if on {
            c.fill(chip.x, chip.y, chip.w, chip.h, mix(BG, AMBER, 30));
        }
        c.neonbox(chip.x, chip.y, chip.w, chip.h, col);
        c.text(
            chip.x + 7,
            chip.y + 3,
            label,
            if on { AMBER } else { TEXT_DIM },
            1,
        );
        hits.add(chip, HIT_CHIP0 + i as u32);
        cx += cw + 4;
    }
    y = row1_y + (row + 1) * (chip_h + 4) + 4;
    c.hline(r.x + 4, y, r.w - 8, mix(BG, BORDER, 140));
    y += 4;

    // feed rows
    let list_h = r.y + r.h - y - 4;
    let rows = (list_h / ROW_H).max(1) as usize;
    let n_items = app.items().len();
    if app.scroll > n_items.saturating_sub(rows) {
        app.scroll = n_items.saturating_sub(rows);
    }
    let scroll = app.scroll;
    let selected = app.selected;
    let loading = app.loading;
    let scrollable = n_items > rows;
    let strip_w = if scrollable { 20 } else { 0 };
    let row_w = r.w - strip_w - 4;

    if n_items == 0 {
        c.text(
            r.x + 8,
            y + 8,
            if loading { "loading feed..." } else { "no items" },
            TEXT_DIM,
            1,
        );
    } else {
        // clone a page of items so we can release the borrow for drawing
        let page: Vec<Item> = app.items().iter().skip(scroll).take(rows).cloned().collect();
        for (i, item) in page.iter().enumerate() {
            let abs = scroll + i;
            let ry = y + i as i32 * ROW_H;
            let rr = Rect::new(r.x + 2, ry, row_w, ROW_H - 2);
            let sel = selected == Some(abs);
            let col = ocint::cat_color(&item.category);
            if sel {
                c.fill(rr.x, rr.y, rr.w, rr.h, mix(BG, col, 40));
                c.fill(rr.x, rr.y + 2, 3, rr.h - 4, col);
            } else if i % 2 == 1 {
                c.fill(rr.x, rr.y, rr.w, rr.h, mix(BG, BG2, 150));
            } else {
                c.fill(rr.x, rr.y + 2, 2, rr.h - 4, mix(BG, col, 80));
            }
            let tag = ocint::cat_label(&item.category);
            let ty = rr.y + (rr.h - fh) / 2;
            c.text(rr.x + 8, ty, tag, col, 1);
            let age = ocint::age_label(&item.published_at);
            c.text(rr.x + rr.w - age.len() as i32 * fw - 6, ty, &age, TEXT_DIM, 1);
            let title_x = rr.x + 8 + 4 * fw;
            let title_max = ((rr.w - 8 - 4 * fw - age.len() as i32 * fw - 12) / fw).max(6) as usize;
            let title = ocint::ascii(&item.title);
            let shown: String = title.chars().take(title_max).collect();
            c.text(title_x, ty, &shown, if sel { CYANHI } else { TEXT }, 1);
            hits.add(rr, HIT_ROW0 + i as u32);
        }
        if scrollable {
            let track = list_h;
            let kh = (track as usize * rows / n_items).max(10) as i32;
            let ky = y + (track as usize * scroll / n_items) as i32;
            c.fill(r.x + r.w - 5, y, 3, track, mix(BG, AMBER, 40));
            c.fill(r.x + r.w - 5, ky, 3, kh, AMBER);
            let strip = Rect::new(r.x + r.w - strip_w, y, strip_w, track);
            let split = y + track * 2 / 5;
            hits.add(Rect::new(strip.x, y, strip.w, split - y), HIT_SCROLL_UP);
            hits.add(Rect::new(strip.x, split, strip.w, y + track - split), HIT_SCROLL_DN);
        }
    }
}

// ---- DETAIL pane -----------------------------------------------------------

fn draw_detail(c: &mut Canvas, r: Rect, app: &IntelApp, hits: &mut HitMap, fw: i32, fh: i32) {
    c.fill(r.x, r.y, r.w, r.h, mix(BG, BG2, 200));
    c.vline(r.x, r.y, r.h, mix(BG, BORDER, 180));

    let mut y = r.y + 6;
    c.text(r.x + 8, y, "> DETAIL", MAGENTA, 1);
    y += fh + 2;
    c.hline(r.x + 6, y, r.w - 12, mix(BG, MAGENTA, 50));
    y += 8;

    let Some(idx) = app.selected else {
        c.text(r.x + 8, y, "select a row", TEXT_DIM, 1);
        y += fh + 6;
        c.text(r.x + 8, y, "from the feed", TEXT_MUTED, 1);
        // source badge
        y = r.y + r.h - fh * 2 - 10;
        c.hline(r.x + 6, y, r.w - 12, mix(BG, BORDER, 140));
        y += 6;
        c.text(r.x + 8, y, "OCINT.APP", AMBER, 1);
        return;
    };
    let items = app.items();
    let Some(item) = items.get(idx) else {
        return;
    };

    let col = ocint::cat_color(&item.category);
    let tag = ocint::cat_label(&item.category);
    c.text(r.x + 8, y, tag, col, 1);
    c.text(r.x + 8 + 4 * fw + 8, y, &format!("sig {}", item.significance), TEXT_DIM, 1);
    y += fh + 2;

    // title wrapped
    let title = ocint::ascii(&item.title);
    let maxc = ((r.w - 16) / fw).max(8) as usize;
    for (i, chunk) in title.as_bytes().chunks(maxc).enumerate() {
        if i > 3 {
            break;
        }
        c.textg(
            r.x + 8,
            y,
            std::str::from_utf8(chunk).unwrap_or(""),
            CYANHI,
            1,
        );
        y += fh;
    }
    y += 4;

    if let Some(city) = &item.city {
        c.text(r.x + 8, y, &format!("@ {}", ocint::ascii(city)), GREEN, 1);
        y += fh;
    }
    c.text(r.x + 8, y, &ocint::ascii(&item.source), TEXT_DIM, 1);
    y += fh;
    c.text(r.x + 8, y, &ocint::age_label(&item.published_at), TEXT_MUTED, 1);
    y += fh + 4;
    c.hline(r.x + 6, y, r.w - 12, mix(BG, BORDER, 140));
    y += 6;

    // body / summary — hard-wrap to detail width
    let body = item
        .summary
        .as_deref()
        .or(item.body.as_deref())
        .unwrap_or("(no summary)");
    let body = ocint::ascii(body);
    let mut wrapped = Vec::new();
    for para in body.split('\n') {
        let chars: Vec<char> = para.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            let end = (i + maxc).min(chars.len());
            wrapped.push(chars[i..end].iter().collect::<String>());
            i = end;
            if wrapped.len() > 40 {
                break;
            }
        }
    }
    if wrapped.is_empty() {
        wrapped.push("(empty)".into());
    }
    let vis = ((r.y + r.h - y - 8) / fh).max(1) as usize;
    let start = app.detail_scroll.min(wrapped.len().saturating_sub(1));
    for (i, line) in wrapped.iter().skip(start).take(vis).enumerate() {
        c.text(r.x + 8, y + i as i32 * fh, line, TEXT2, 1);
    }
    if wrapped.len() > vis {
        let strip = Rect::new(r.x + r.w - 18, y, 16, vis as i32 * fh);
        let split = strip.y + strip.h * 2 / 5;
        hits.add(Rect::new(strip.x, strip.y, strip.w, split - strip.y), HIT_DET_UP);
        hits.add(Rect::new(strip.x, split, strip.w, strip.y + strip.h - split), HIT_DET_DN);
    }
}
