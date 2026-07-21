//! App abstraction: each app is a page under the shell chrome.

pub mod camera;
pub mod files;
pub mod home;
pub mod house;
pub mod intel;
pub mod music;
pub mod network;
pub mod songs;
pub mod system;

use neon_gfx::canvas::Canvas;
use neon_gfx::geom::Rect;

use crate::collectors::Snapshot;

pub type HitId = u32;

/// Hit regions registered during draw, resolved on tap. Replaces the C globals.
#[derive(Default)]
pub struct HitMap {
    items: Vec<(Rect, HitId)>,
}

impl HitMap {
    pub fn clear(&mut self) {
        self.items.clear();
    }

    pub fn add(&mut self, r: Rect, id: HitId) {
        self.items.push((r, id));
    }

    /// Topmost (last-registered) hit wins.
    pub fn hit(&self, x: i32, y: i32) -> Option<HitId> {
        self.items.iter().rev().find(|(r, _)| r.contains(x, y)).map(|(_, id)| *id)
    }
}

/// Shared state handed to apps each tick/draw/tap.
pub struct Ctx<'a> {
    pub snap: &'a Snapshot,
    pub toast: &'a mut Option<(String, std::time::Instant)>,
}

impl Ctx<'_> {
    pub fn set_toast(&mut self, msg: impl Into<String>) {
        *self.toast = Some((msg.into(), std::time::Instant::now()));
    }
}

pub trait App {
    fn title(&self) -> &'static str;
    fn accent(&self) -> u32;
    /// Redraw cadence while this app is active.
    fn tick_ms(&self) -> u64 {
        1000
    }
    /// Background work each tick (active app only).
    fn tick(&mut self, _ctx: &mut Ctx) {}
    fn draw(&mut self, c: &mut Canvas, area: Rect, hits: &mut HitMap, ctx: &Ctx);
    /// Returns true if state changed (needs redraw).
    fn on_tap(&mut self, _id: HitId, _ctx: &mut Ctx) -> bool {
        false
    }
    /// Called when the app becomes the active screen.
    fn on_enter(&mut self) {}
    /// Called when the app is no longer the active screen.
    fn on_leave(&mut self) {}
}
