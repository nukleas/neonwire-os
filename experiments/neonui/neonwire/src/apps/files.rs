//! FILES — cyberdesign dense browser: places rail + table + detail pane.
//! Inspired by neuromancer left-list / center-stream / right-scope layout.

use neon_gfx::canvas::{mix, Canvas};
use neon_gfx::font_data::{FONT_H, FONT_W};
use neon_gfx::geom::Rect;
use neon_gfx::theme::*;

use super::{App, Ctx, HitId, HitMap};

const HIT_UP: HitId = 1;
const HIT_REFRESH: HitId = 2;
const HIT_OPEN: HitId = 3;
const HIT_PARENT: HitId = 4;
const HIT_ROOT0: HitId = 0x10;
const HIT_CRUMB0: HitId = 0x40; // path segment chips
const HIT_SCROLL_UP: HitId = 0x20;
const HIT_SCROLL_DN: HitId = 0x21;
const HIT_ROW0: HitId = 0x100;
const HIT_OV_BG: HitId = 0x300;
const HIT_OV_X: HitId = 0x301;
const HIT_OV_UP: HitId = 0x302;
const HIT_OV_DN: HitId = 0x303;
const HIT_DET_SCROLL_UP: HitId = 0x310;
const HIT_DET_SCROLL_DN: HitId = 0x311;

const ROOTS: [(&str, &str); 6] = [
    ("SD", "/mnt/sd"),
    ("LAB", "/mnt/sd/linux-lab"),
    ("DATA", "/mnt/data"),
    ("CACHE", "/mnt/cache"),
    ("SYS", "/mnt/system"),
    ("ROOT", "/"),
];

const PREVIEW_CAP: u64 = 96 * 1024;
const PLACES_W: i32 = 104;
const DETAIL_W: i32 = 230;
const ROW_H: i32 = 30; // tight; font is 25 — 5px breathing room

#[derive(Clone)]
struct Entry {
    name: String,
    is_dir: bool,
    is_link: bool,
    size: u64,
}

struct Preview {
    title: String,
    lines: Vec<String>,
    scroll: usize,
}

pub struct FilesApp {
    path: String,
    entries: Vec<Entry>,
    scroll: usize,
    selected: Option<usize>,
    err: Option<String>,
    free_mb: Option<(u64, u64)>,
    preview: Option<Preview>,
    /// Snippet lines for the detail pane (not full modal).
    detail_lines: Vec<String>,
    detail_scroll: usize,
    n_dirs: usize,
    n_files: usize,
}

impl FilesApp {
    pub fn new() -> FilesApp {
        let path = first_existing(&["/mnt/sd/linux-lab", "/mnt/sd", "/mnt/data", "/"]);
        let mut app = FilesApp {
            path,
            entries: Vec::new(),
            scroll: 0,
            selected: None,
            err: None,
            free_mb: None,
            preview: None,
            detail_lines: Vec::new(),
            detail_scroll: 0,
            n_dirs: 0,
            n_files: 0,
        };
        app.reload();
        app
    }

    fn reload(&mut self) {
        self.err = None;
        self.scroll = 0;
        self.selected = None;
        self.preview = None;
        self.detail_lines.clear();
        self.detail_scroll = 0;
        self.free_mb = stat_free_mb(&self.path);
        match list_dir(&self.path) {
            Ok(entries) => {
                self.n_dirs = entries.iter().filter(|e| e.is_dir).count();
                self.n_files = entries.len() - self.n_dirs;
                self.entries = entries;
            }
            Err(e) => {
                self.entries.clear();
                self.n_dirs = 0;
                self.n_files = 0;
                self.err = Some(e);
            }
        }
    }

    fn go(&mut self, path: String, ctx: &mut Ctx) {
        let path = if path != "/" {
            path.trim_end_matches('/').to_string()
        } else {
            path
        };
        if !std::path::Path::new(&path).exists() {
            ctx.set_toast(format!("missing: {path}"));
            return;
        }
        self.path = path;
        self.reload();
    }

    fn go_up(&mut self, ctx: &mut Ctx) {
        if self.path == "/" {
            ctx.set_toast("already at /");
            return;
        }
        let parent = std::path::Path::new(&self.path)
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .filter(|p| !p.is_empty())
            .unwrap_or_else(|| "/".into());
        self.go(parent, ctx);
    }

    fn select(&mut self, idx: usize) {
        if idx >= self.entries.len() {
            return;
        }
        self.selected = Some(idx);
        self.detail_scroll = 0;
        self.detail_lines = detail_for(&self.path, &self.entries[idx]);
    }

    fn open_selected(&mut self, ctx: &mut Ctx) {
        let Some(idx) = self.selected else {
            ctx.set_toast("select an entry");
            return;
        };
        self.open_entry(idx, ctx);
    }

    fn open_entry(&mut self, idx: usize, ctx: &mut Ctx) {
        let Some(e) = self.entries.get(idx).cloned() else {
            return;
        };
        let full = join_path(&self.path, &e.name);
        if e.is_dir {
            self.go(full, ctx);
            return;
        }
        match preview_file(&full, e.size) {
            Ok(pv) => self.preview = Some(pv),
            Err(msg) => ctx.set_toast(msg),
        }
    }
}

impl App for FilesApp {
    fn title(&self) -> &'static str {
        "FILES"
    }

    fn accent(&self) -> u32 {
        BLUE
    }

    fn on_enter(&mut self) {
        self.reload();
    }

    fn draw(&mut self, c: &mut Canvas, area: Rect, hits: &mut HitMap, _ctx: &Ctx) {
        let (fw, fh) = (FONT_W as i32, FONT_H as i32);

        // outer frame
        c.fill(area.x, area.y, area.w, area.h, mix(BG, BG2, 40));
        c.neonbox(area.x, area.y, area.w, area.h, mix(BG, BLUE, 80));
        c.corners(area.x, area.y, area.w, area.h, BLUE, 12);

        let inner = Rect::new(area.x + 4, area.y + 4, area.w - 8, area.h - 8);
        let places = Rect::new(inner.x, inner.y, PLACES_W, inner.h);
        let detail = Rect::new(inner.x + inner.w - DETAIL_W, inner.y, DETAIL_W, inner.h);
        let main = Rect::new(
            places.x + places.w + 1,
            inner.y,
            detail.x - places.x - places.w - 2,
            inner.h,
        );

        draw_places(c, places, &self.path, hits, fw, fh);
        draw_main(c, main, self, hits, fw, fh);
        draw_detail(c, detail, self, hits, fw, fh);

        // full-screen text preview modal (on top)
        if let Some(ov) = &self.preview {
            draw_preview_modal(c, area, ov, hits, fw, fh);
        }
    }

    fn on_tap(&mut self, id: HitId, ctx: &mut Ctx) -> bool {
        if self.preview.is_some() {
            let step = 10;
            match id {
                HIT_OV_X | HIT_OV_BG => self.preview = None,
                HIT_OV_UP => {
                    if let Some(ov) = &mut self.preview {
                        ov.scroll = ov.scroll.saturating_sub(step);
                    }
                }
                HIT_OV_DN => {
                    if let Some(ov) = &mut self.preview {
                        let max = ov.lines.len().saturating_sub(1);
                        ov.scroll = (ov.scroll + step).min(max);
                    }
                }
                _ => self.preview = None,
            }
            return true;
        }

        match id {
            HIT_UP | HIT_PARENT => {
                self.go_up(ctx);
                true
            }
            HIT_REFRESH => {
                self.reload();
                ctx.set_toast("refreshed");
                true
            }
            HIT_OPEN => {
                self.open_selected(ctx);
                true
            }
            HIT_SCROLL_UP => {
                self.scroll = self.scroll.saturating_sub(6);
                true
            }
            HIT_SCROLL_DN => {
                self.scroll = self.scroll.saturating_add(6);
                true
            }
            HIT_DET_SCROLL_UP => {
                self.detail_scroll = self.detail_scroll.saturating_sub(4);
                true
            }
            HIT_DET_SCROLL_DN => {
                self.detail_scroll = self.detail_scroll.saturating_add(4);
                true
            }
            i if (HIT_ROOT0..HIT_ROOT0 + ROOTS.len() as u32).contains(&i) => {
                let (_, path) = ROOTS[(i - HIT_ROOT0) as usize];
                self.go(path.to_string(), ctx);
                true
            }
            i if (HIT_CRUMB0..HIT_CRUMB0 + 32).contains(&i) => {
                let segs = path_segments(&self.path);
                let idx = (i - HIT_CRUMB0) as usize;
                if let Some(p) = segs.get(idx).map(|(_, p)| p.clone()) {
                    self.go(p, ctx);
                }
                true
            }
            i if (HIT_ROW0..HIT_ROW0 + 64).contains(&i) => {
                let idx = self.scroll + (i - HIT_ROW0) as usize;
                if self.selected == Some(idx) {
                    // second tap opens
                    self.open_entry(idx, ctx);
                } else {
                    self.select(idx);
                }
                true
            }
            _ => false,
        }
    }
}

// ---- regions ----------------------------------------------------------------

fn draw_places(c: &mut Canvas, r: Rect, path: &str, hits: &mut HitMap, fw: i32, fh: i32) {
    c.fill(r.x, r.y, r.w, r.h, mix(BG, BG2, 200));
    c.vline(r.x + r.w - 1, r.y, r.h, mix(BG, BORDER, 180));

    let mut y = r.y + 6;
    c.text(r.x + 8, y, "> PLACES", BLUE, 1);
    y += fh + 4;
    c.hline(r.x + 6, y, r.w - 12, mix(BG, BLUE, 50));
    y += 6;

    for (i, (label, p)) in ROOTS.iter().enumerate() {
        let exists = std::path::Path::new(p).exists();
        let on = path == *p || (*p != "/" && path.starts_with(&format!("{p}/")));
        let hr = Rect::new(r.x, y, r.w - 1, 36);
        if on {
            c.fill(hr.x, hr.y, hr.w, hr.h, mix(BG, BLUE, 30));
            c.fill(hr.x, hr.y + 4, 3, hr.h - 8, BLUE);
        }
        let col = if on {
            BLUE
        } else if exists {
            TEXT2
        } else {
            TEXT_DIM
        };
        c.text(hr.x + 10, hr.y + 6, label, col, 1);
        // tiny existence mark
        let orb = if exists { GREEN } else { mix(BG, RED, 140) };
        c.fill(hr.x + hr.w - 14, hr.y + 14, 6, 6, orb);
        hits.add(hr, HIT_ROOT0 + i as u32);
        y += 38;
    }

    y += 4;
    c.text(r.x + 8, y, "> ACT", AMBER, 1);
    y += fh + 4;
    c.hline(r.x + 6, y, r.w - 12, mix(BG, AMBER, 45));
    y += 8;

    for (label, hit, acc) in [
        ("UP ..", HIT_UP, AMBER),
        ("REFRESH", HIT_REFRESH, CYAN),
        ("OPEN", HIT_OPEN, GREEN),
    ] {
        // single-line button: height = font + pad, text vertically centered
        let hr = Rect::new(r.x + 6, y, r.w - 14, fh + 12);
        c.fill(hr.x, hr.y, hr.w, hr.h, mix(BG, acc, 18));
        c.neonbox(hr.x, hr.y, hr.w, hr.h, mix(BG, acc, 120));
        let lx = hr.x + (hr.w - label.len() as i32 * fw) / 2;
        let ly = hr.y + (hr.h - fh) / 2;
        c.text(lx, ly, label, acc, 1);
        hits.add(hr, hit);
        y += hr.h + 6;
    }
    let _ = fh;
}

fn draw_main(c: &mut Canvas, r: Rect, app: &mut FilesApp, hits: &mut HitMap, fw: i32, fh: i32) {
    c.fill(r.x, r.y, r.w, r.h, BG);

    // breadcrumb strip
    let mut y = r.y + 4;
    let mut x = r.x + 6;
    c.text(x, y, "PATH", TEXT_DIM, 1);
    x += 5 * fw + 6;
    let segs = path_segments(&app.path);
    for (i, (name, _)) in segs.iter().enumerate() {
        let label = if name.is_empty() { "/" } else { name.as_str() };
        let w = label.len() as i32 * fw + 16;
        if x + w > r.x + r.w - 8 {
            break;
        }
        let chip = Rect::new(x, y - 2, w, fh + 4);
        let on = i + 1 == segs.len();
        if on {
            c.fill(chip.x, chip.y, chip.w, chip.h, mix(BG, BLUE, 35));
        }
        c.neonbox(chip.x, chip.y, chip.w, chip.h, if on { BLUE } else { BORDER });
        c.text(chip.x + 8, chip.y + 2, label, if on { BLUE } else { TEXT2 }, 1);
        hits.add(chip, HIT_CRUMB0 + i as u32);
        x += w + 4;
        if i + 1 < segs.len() {
            c.text(x - 2, y, "/", TEXT_DIM, 1);
        }
    }
    y += fh + 6;
    c.hline(r.x + 4, y, r.w - 8, mix(BG, BLUE, 55));
    y += 4;

    // table header
    c.fill(r.x, y, r.w, fh + 4, mix(BG, BLUE, 18));
    c.text(r.x + 8, y + 2, "KIND", BLUE, 1);
    c.text(r.x + 8 + 5 * fw, y + 2, "NAME", BLUE, 1);
    c.text(r.x + r.w - 12 * fw, y + 2, "SIZE", BLUE, 1);
    y += fh + 6;

    let foot_h = 22;
    let list_h = r.y + r.h - y - foot_h;
    let rows = (list_h / ROW_H).max(1) as usize;
    let scrollable = app.entries.len() > rows;
    let strip_w = if scrollable { 22 } else { 0 };
    let row_w = r.w - strip_w - 4;

    if app.scroll > app.entries.len().saturating_sub(rows) {
        app.scroll = app.entries.len().saturating_sub(rows);
    }

    if let Some(err) = &app.err {
        c.text(r.x + 8, y + 8, &format!("error: {err}"), RED, 1);
    } else if app.entries.is_empty() {
        c.text(r.x + 8, y + 8, "(empty)", TEXT_DIM, 1);
    } else {
        for (i, e) in app.entries.iter().skip(app.scroll).take(rows).enumerate() {
            let abs = app.scroll + i;
            let ry = y + i as i32 * ROW_H;
            let rr = Rect::new(r.x + 2, ry, row_w, ROW_H - 1);
            let sel = app.selected == Some(abs);
            if sel {
                c.fill(rr.x, rr.y, rr.w, rr.h, mix(BG, BLUE, 40));
                c.fill(rr.x, rr.y + 2, 2, rr.h - 4, BLUE);
            } else if i % 2 == 1 {
                c.fill(rr.x, rr.y, rr.w, rr.h, mix(BG, BG2, 160));
            }
            let kind = if e.is_dir {
                "DIR"
            } else if e.is_link {
                "LNK"
            } else {
                "FIL"
            };
            let kcol = if e.is_dir {
                BLUE
            } else if e.is_link {
                PURPLE
            } else {
                TEXT2
            };
            c.text(rr.x + 6, rr.y + 2, kind, kcol, 1);
            let name_max = ((rr.w - 18 * fw) / fw).max(6) as usize;
            let name = truncate_end(&e.name, name_max);
            c.text(
                rr.x + 6 + 5 * fw,
                rr.y + 2,
                &name,
                if sel { CYANHI } else if e.is_dir { TEXT } else { TEXT2 },
                1,
            );
            if e.is_dir {
                c.text(rr.x + rr.w - 2 * fw - 4, rr.y + 2, ">", mix(BG, BLUE, 180), 1);
            } else {
                let sz = format_size(e.size);
                c.text(rr.x + rr.w - sz.len() as i32 * fw - 4, rr.y + 2, &sz, TEXT_DIM, 1);
            }
            hits.add(rr, HIT_ROW0 + i as u32);
        }
        if scrollable {
            let track = list_h;
            let kh = (track as usize * rows / app.entries.len()).max(10) as i32;
            let ky = y + (track as usize * app.scroll / app.entries.len()) as i32;
            c.fill(r.x + r.w - 6, y, 3, track, mix(BG, BLUE, 40));
            c.fill(r.x + r.w - 6, ky, 3, kh, BLUE);
            let strip = Rect::new(r.x + r.w - strip_w, y, strip_w, track);
            let split = y + track * 2 / 5;
            hits.add(Rect::new(strip.x, y, strip.w, split - y), HIT_SCROLL_UP);
            hits.add(Rect::new(strip.x, split, strip.w, y + track - split), HIT_SCROLL_DN);
        }
    }

    // footer status (cd status-bar dense)
    let fy = r.y + r.h - foot_h + 2;
    c.hline(r.x + 4, fy - 4, r.w - 8, mix(BG, BORDER, 160));
    let free = match app.free_mb {
        Some((a, t)) if t > 0 => format!("{a}/{t}MB free"),
        _ => "fs ?".into(),
    };
    let foot = format!(
        "{} entries  {} dir  {} fil  {}  tap=select  again=open",
        app.entries.len(),
        app.n_dirs,
        app.n_files,
        free
    );
    c.text(r.x + 6, fy, &truncate_end(&foot, ((r.w - 12) / fw) as usize), TEXT_DIM, 1);
}

fn draw_detail(c: &mut Canvas, r: Rect, app: &FilesApp, hits: &mut HitMap, fw: i32, fh: i32) {
    c.fill(r.x, r.y, r.w, r.h, mix(BG, BG2, 200));
    c.vline(r.x, r.y, r.h, mix(BG, BORDER, 180));

    let mut y = r.y + 6;
    c.text(r.x + 8, y, "> DETAIL", MAGENTA, 1);
    y += fh + 4;
    c.hline(r.x + 6, y, r.w - 12, mix(BG, MAGENTA, 50));
    y += 8;

    let Some(idx) = app.selected else {
        c.text(r.x + 8, y, "no selection", TEXT_DIM, 1);
        y += fh + 8;
        c.text(r.x + 8, y, "tap a row", TEXT_MUTED, 1);
        y += fh + 4;
        c.text(r.x + 8, y, "again to open", TEXT_MUTED, 1);
        // still show volume free
        y = r.y + r.h - fh * 3 - 16;
        c.hline(r.x + 6, y, r.w - 12, mix(BG, BORDER, 140));
        y += 6;
        c.text(r.x + 8, y, "VOLUME", TEXT_DIM, 1);
        y += fh;
        if let Some((a, t)) = app.free_mb {
            c.text(r.x + 8, y, &format!("{a} / {t} MB"), CYAN, 1);
            if t > 0 {
                let pct = ((t - a) * 100 / t) as i32;
                c.bar(r.x + 8, y + fh + 2, r.w - 20, 10, pct, BLUE);
            }
        }
        return;
    };

    let e = &app.entries[idx];
    let kind = if e.is_dir {
        "DIRECTORY"
    } else if e.is_link {
        "SYMLINK"
    } else {
        "FILE"
    };
    let kcol = if e.is_dir { BLUE } else if e.is_link { PURPLE } else { GREEN };

    // name block
    let name = truncate_end(&e.name, ((r.w - 16) / fw) as usize);
    c.textg(r.x + 8, y, &name, CYANHI, 1);
    y += fh + 2;
    c.text(r.x + 8, y, kind, kcol, 1);
    y += fh + 2;
    c.text(r.x + 8, y, &format!("size  {}", format_size(e.size)), TEXT2, 1);
    y += fh + 6;
    c.hline(r.x + 6, y, r.w - 12, mix(BG, BORDER, 140));
    y += 6;

    // snippet / meta lines
    let max_lines = ((r.y + r.h - y - 50) / (fh + 2)).max(1) as usize;
    let lines = &app.detail_lines;
    let start = app.detail_scroll.min(lines.len().saturating_sub(1));
    for (i, line) in lines.iter().skip(start).take(max_lines).enumerate() {
        let col = if line.starts_with("##") { AMBER } else { TEXT2 };
        let clipped: String = line.chars().take(((r.w - 16) / fw) as usize).collect();
        c.text(r.x + 8, y + i as i32 * (fh + 2), &clipped, col, 1);
    }
    if lines.len() > max_lines {
        let strip = Rect::new(r.x + r.w - 20, y, 18, (max_lines as i32) * (fh + 2));
        let split = strip.y + strip.h * 2 / 5;
        hits.add(Rect::new(strip.x, strip.y, strip.w, split - strip.y), HIT_DET_SCROLL_UP);
        hits.add(Rect::new(strip.x, split, strip.w, strip.y + strip.h - split), HIT_DET_SCROLL_DN);
    }

    // action buttons at bottom of detail — single-line, text centered
    let bh = fh + 12;
    let by = r.y + r.h - bh - 6;
    c.hline(r.x + 6, by - 6, r.w - 12, mix(BG, BORDER, 140));
    let bw = (r.w - 20) / 2;
    let open = Rect::new(r.x + 6, by, bw, bh);
    let up = Rect::new(r.x + 10 + bw, by, bw, bh);
    let ol = if e.is_dir { "ENTER" } else { "PREVIEW" };
    c.fill(open.x, open.y, open.w, open.h, mix(BG, GREEN, 22));
    c.neonbox(open.x, open.y, open.w, open.h, GREEN);
    c.text(
        open.x + (open.w - ol.len() as i32 * fw) / 2,
        open.y + (open.h - fh) / 2,
        ol,
        GREEN,
        1,
    );
    hits.add(open, HIT_OPEN);
    c.neonbox(up.x, up.y, up.w, up.h, AMBER);
    c.text(up.x + (up.w - 2 * fw) / 2, up.y + (up.h - fh) / 2, "UP", AMBER, 1);
    hits.add(up, HIT_PARENT);
}

fn draw_preview_modal(c: &mut Canvas, area: Rect, ov: &Preview, hits: &mut HitMap, fw: i32, fh: i32) {
    let o = Rect::new(area.x + 6, area.y + 4, area.w - 12, area.h - 8);
    hits.add(Rect::new(0, 0, c.w, c.h), HIT_OV_BG);
    c.fill(o.x, o.y, o.w, o.h, PANEL);
    c.neonbox(o.x, o.y, o.w, o.h, BLUE);
    c.corners(o.x, o.y, o.w, o.h, BLUE, 16);
    let title = truncate_end(&ov.title, ((o.w - 80) / fw) as usize);
    c.textg(o.x + 16, o.y + 8, &title, BLUE, 1);
    c.text(o.x + o.w - 4 * fw - 12, o.y + 8, "[X]", MAGENTA, 1);
    hits.add(Rect::new(o.x + o.w - 6 * fw - 12, o.y, 6 * fw + 12, 40), HIT_OV_X);
    c.hline(o.x + 12, o.y + 34, o.w - 24, mix(BG, BLUE, 70));

    let lh = fh + 2;
    let vis = ((o.h - 40 - 24) / lh) as usize;
    let maxc = ((o.w - 36) / fw) as usize;
    let by = o.y + 40;
    for (i, l) in ov.lines.iter().skip(ov.scroll).take(vis).enumerate() {
        let col = if l.starts_with("##") {
            AMBER
        } else if l.contains("error") || l.contains("Error") {
            RED
        } else {
            TEXT2
        };
        let clipped: String = l.chars().take(maxc).collect();
        c.text(o.x + 14, by + i as i32 * lh, &clipped, col, 1);
    }
    if ov.lines.len() > vis {
        let track = o.h - 40 - 24;
        let kh = (track as usize * vis / ov.lines.len()).max(10) as i32;
        let ky = by + (track as usize * ov.scroll / ov.lines.len()) as i32;
        c.fill(o.x + o.w - 7, by, 3, track, mix(BG, BLUE, 40));
        c.fill(o.x + o.w - 7, ky, 3, kh, BLUE);
        let split = o.y + o.h * 2 / 5;
        hits.add(Rect::new(o.x, o.y + 40, o.w, split - o.y - 40), HIT_OV_UP);
        hits.add(Rect::new(o.x, split, o.w, o.y + o.h - split), HIT_OV_DN);
    }
    c.text(
        o.x + 14,
        o.y + o.h - 18,
        &format!(
            "{}/{}  tap [X] close",
            (ov.scroll + 1).min(ov.lines.len()),
            ov.lines.len()
        ),
        TEXT_DIM,
        1,
    );
    hits.add(Rect::new(o.x + o.w - 6 * fw - 12, o.y, 6 * fw + 12, 40), HIT_OV_X);
}

// ---- helpers ----------------------------------------------------------------

fn path_segments(path: &str) -> Vec<(String, String)> {
    // returns (display_name, absolute_path) including root
    let mut out = vec![("/".into(), "/".into())];
    if path == "/" {
        return out;
    }
    let mut acc = String::new();
    for part in path.trim_start_matches('/').split('/') {
        if part.is_empty() {
            continue;
        }
        acc.push('/');
        acc.push_str(part);
        out.push((part.to_string(), acc.clone()));
    }
    out
}

fn detail_for(base: &str, e: &Entry) -> Vec<String> {
    let full = join_path(base, &e.name);
    let mut lines = vec![
        format!("## path"),
        truncate_end(&full, 40),
        String::new(),
    ];
    if e.is_dir {
        // count children cheaply
        match std::fs::read_dir(&full) {
            Ok(rd) => {
                let mut n = 0usize;
                for _ in rd.flatten() {
                    n += 1;
                    if n > 500 {
                        break;
                    }
                }
                lines.push(format!("children  {n}{}", if n > 500 { "+" } else { "" }));
            }
            Err(err) => lines.push(format!("readdir  {err}")),
        }
        lines.push(String::new());
        lines.push("tap ENTER or".into());
        lines.push("row again".into());
    } else {
        // head of file
        match peek_head(&full, 12) {
            Ok(head) => {
                lines.push("## head".into());
                lines.extend(head);
            }
            Err(e) => lines.push(format!("peek  {e}")),
        }
    }
    lines
}

fn peek_head(path: &str, max_lines: usize) -> Result<Vec<String>, String> {
    use std::io::{BufRead, BufReader};
    let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for (i, line) in BufReader::new(f).lines().enumerate() {
        if i >= max_lines {
            out.push("...".into());
            break;
        }
        let l = line.unwrap_or_default().replace('\t', " ");
        // drop non-ascii for glyph atlas
        let clean: String = l.chars().map(|ch| if ch.is_ascii() { ch } else { '?' }).collect();
        out.push(clean);
    }
    if out.is_empty() {
        out.push("(empty)".into());
    }
    Ok(out)
}

fn first_existing(paths: &[&str]) -> String {
    for p in paths {
        if std::path::Path::new(p).is_dir() {
            return (*p).to_string();
        }
    }
    "/".into()
}

fn join_path(base: &str, name: &str) -> String {
    if base == "/" {
        format!("/{name}")
    } else {
        format!("{base}/{name}")
    }
}

fn list_dir(path: &str) -> Result<Vec<Entry>, String> {
    let rd = std::fs::read_dir(path).map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    for e in rd.filter_map(|e| e.ok()) {
        let name = e.file_name().to_string_lossy().into_owned();
        if name == "." || name == ".." {
            continue;
        }
        let ft = e.file_type().ok();
        let is_link = ft.as_ref().map(|t| t.is_symlink()).unwrap_or(false);
        let is_dir = if is_link {
            e.path().is_dir()
        } else {
            ft.as_ref().map(|t| t.is_dir()).unwrap_or(false)
        };
        let size = e.metadata().map(|m| m.len()).unwrap_or(0);
        entries.push(Entry { name, is_dir, is_link, size });
    }
    entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });
    Ok(entries)
}

fn stat_free_mb(path: &str) -> Option<(u64, u64)> {
    let c = std::ffi::CString::new(path).ok()?;
    let mut vfs: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut vfs) } != 0 || vfs.f_blocks == 0 {
        return None;
    }
    let tot = vfs.f_blocks as u64 * vfs.f_frsize as u64;
    let avail = vfs.f_bavail as u64 * vfs.f_frsize as u64;
    Some((avail >> 20, tot >> 20))
}

fn format_size(n: u64) -> String {
    if n < 1024 {
        format!("{n}B")
    } else if n < 1024 * 1024 {
        format!("{}K", n / 1024)
    } else if n < 1024 * 1024 * 1024 {
        format!("{}M", n / (1024 * 1024))
    } else {
        format!("{}G", n / (1024 * 1024 * 1024))
    }
}

fn truncate_end(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    if max <= 3 {
        return "...".chars().take(max).collect();
    }
    let keep: String = s.chars().take(max - 3).collect();
    format!("{keep}...")
}

fn preview_file(path: &str, size: u64) -> Result<Preview, String> {
    let mut meta_lines = vec![
        format!("## path  {path}"),
        format!("## size  {} ({size} bytes)", format_size(size)),
    ];
    if let Ok(m) = std::fs::metadata(path) {
        if let Ok(modif) = m.modified() {
            if let Ok(d) = modif.duration_since(std::time::UNIX_EPOCH) {
                meta_lines.push(format!("## mtime unix {}", d.as_secs()));
            }
        }
    }
    meta_lines.push(String::new());

    if size == 0 {
        let mut lines = meta_lines;
        lines.push("(empty file)".into());
        return Ok(Preview { title: file_title(path), lines, scroll: 0 });
    }

    let to_read = size.min(PREVIEW_CAP) as usize;
    let mut f = std::fs::File::open(path).map_err(|e| format!("open: {e}"))?;
    use std::io::Read;
    let mut buf = vec![0u8; to_read];
    let n = f.read(&mut buf).map_err(|e| format!("read: {e}"))?;
    buf.truncate(n);

    let sample = &buf[..buf.len().min(512)];
    let binary = sample.contains(&0);
    let meta_n = meta_lines.len();
    let mut lines = meta_lines;

    if binary {
        lines.push("## binary - hex head (first 256 B)".into());
        lines.push(String::new());
        let dump = &buf[..buf.len().min(256)];
        for (i, chunk) in dump.chunks(16).enumerate() {
            let mut hex = String::new();
            let mut asc = String::new();
            for (j, b) in chunk.iter().enumerate() {
                if j == 8 {
                    hex.push(' ');
                }
                hex.push_str(&format!("{b:02x} "));
                let ch = *b as char;
                asc.push(if ch.is_ascii_graphic() || ch == ' ' { ch } else { '.' });
            }
            lines.push(format!("{:04x}  {:<49} {}", i * 16, hex, asc));
        }
        if size > PREVIEW_CAP {
            lines.push(String::new());
            lines.push(format!("## truncated - file is {}", format_size(size)));
        }
    } else {
        let text = String::from_utf8_lossy(&buf);
        if size > PREVIEW_CAP {
            lines.push(format!(
                "## text preview - first {} of {}",
                format_size(PREVIEW_CAP),
                format_size(size)
            ));
            lines.push(String::new());
        }
        for l in text.lines() {
            let clean: String = l
                .chars()
                .map(|ch| if ch.is_ascii() { ch } else { '?' })
                .collect();
            lines.push(clean.replace('\t', "    "));
        }
        if lines.len() == meta_n {
            lines.push("(no lines)".into());
        }
    }

    Ok(Preview { title: file_title(path), lines, scroll: 0 })
}

fn file_title(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}
