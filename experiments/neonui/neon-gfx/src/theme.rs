//! cyberdesign tokens (0xRRGGBB), from in-repo palette src/tokens/{base,semantic}.ts.
//! Accent families are swappable at runtime (Settings theme switcher); everything
//! else is shared chrome. Default identity: cyan/magenta (NEONWIRE).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    pub name: &'static str,
    pub accent: u32,
    pub accent_hi: u32,
    pub accent2: u32, // secondary/complementary accent
}

pub const THEMES: [Theme; 5] = [
    Theme { name: "CYAN", accent: 0x47f6ff, accent_hi: 0xbdffff, accent2: 0xff2bd6 },
    Theme { name: "ORANGE", accent: 0xff8800, accent_hi: 0xffaa00, accent2: 0xff2bd6 },
    Theme { name: "GREEN", accent: 0x00ff00, accent_hi: 0x52ff9f, accent2: 0x9d7cff },
    Theme { name: "RED", accent: 0xff4444, accent_hi: 0xff7777, accent2: 0x6aa8ff },
    Theme { name: "STEEL", accent: 0x9aafc4, accent_hi: 0xc8d8e8, accent2: 0xffaa00 },
];

// Shared chrome (theme-invariant)
pub const BG: u32 = 0x05060a; // bgBase
pub const BG2: u32 = 0x0b0f18; // bgPanel
pub const PANEL: u32 = 0x0b0f18;
pub const BORDER: u32 = 0x26324c; // borderMid
pub const TEXT: u32 = 0xe8e4e0; // textPrimary (warm off-white)
pub const TEXT2: u32 = 0xa7b7d6; // textSecondary
pub const TEXT_MUTED: u32 = 0x8a8078;
pub const TEXT_DIM: u32 = 0x6a625c;
pub const WHITE: u32 = 0xf2f7ff; // textPrimaryAlt

// Status colors (consistent across themes)
pub const GREEN: u32 = 0x52ff9f; // ok/live
pub const AMBER: u32 = 0xffaa00; // warn
pub const RED: u32 = 0xff456c; // error/critical (redAlt)
pub const BLUE: u32 = 0x6aa8ff;
pub const PURPLE: u32 = 0x9d7cff;
pub const GOLD: u32 = 0xffcc00;

// Legacy fbgfx.h names still used by ported drawing code
pub const CYAN: u32 = 0x47f6ff;
pub const CYANHI: u32 = 0xbdffff;
pub const MAGENTA: u32 = 0xff2bd6;
pub const GRID: u32 = 0x47f6ff; // pixel-grid tint (applied faintly)
