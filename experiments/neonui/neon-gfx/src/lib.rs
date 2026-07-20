//! neon-gfx — framebuffer engine for NEONWIRE OS (port of experiments/fbui/fbgfx.h).
//!
//! Hardware contract (DL7006 / mtkfb): the ZS070BE3019B3H7II is a command-mode MIPI
//! panel — pixels reach the glass only on FBIOPAN_DISPLAY with a *changed* yoffset.
//! Present by cycling the virtual buffers; never memcpy-and-hope.

pub mod canvas;
pub mod draw;
pub mod fb;
pub mod font;
pub mod font_data;
pub mod theme;
