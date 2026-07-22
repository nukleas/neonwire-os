//! Panel backlight control for idle-blanking. The DL7006 exposes the LCD
//! backlight as an LED at /sys/class/leds/lcd-backlight/brightness (0..255).
//! First power-management win: blank the panel after inactivity, restore on
//! touch — the backlight is a major battery draw and the tablet has no other
//! screen timeout. The first tap after blanking only wakes the screen; it is
//! not dispatched to the UI.

use std::time::{Duration, Instant};

const NODE: &str = "/sys/class/leds/lcd-backlight/brightness";
const AWAKE: u32 = 255; // full brightness on wake
const BLANK: u32 = 0; // idle level (verify physical effect on device)
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);

pub struct Backlight {
    last_activity: Instant,
    blanked: bool,
    available: bool,
}

impl Backlight {
    pub fn new() -> Backlight {
        let available = std::path::Path::new(NODE).exists();
        let mut b = Backlight { last_activity: Instant::now(), blanked: false, available };
        b.set(AWAKE);
        b
    }

    fn set(&self, level: u32) {
        if self.available {
            let _ = std::fs::write(NODE, level.to_string());
        }
    }

    /// Call on any user input. Returns true if this input only woke the screen
    /// (and should therefore be swallowed, not dispatched to the UI).
    pub fn on_activity(&mut self) -> bool {
        self.last_activity = Instant::now();
        if self.blanked {
            self.blanked = false;
            self.set(AWAKE);
            true // consume the wake tap
        } else {
            false
        }
    }

    /// Call each loop iteration; blanks the panel once idle exceeds the timeout.
    pub fn tick(&mut self) {
        if !self.blanked && self.last_activity.elapsed() >= IDLE_TIMEOUT {
            self.blanked = true;
            self.set(BLANK);
        }
    }

    pub fn is_blanked(&self) -> bool {
        self.blanked
    }

    /// Force the panel on (e.g. to show a critical warning regardless of idle).
    pub fn wake(&mut self) {
        self.last_activity = Instant::now();
        self.blanked = false;
        self.set(AWAKE);
    }

    /// Blank the panel immediately (agent / idle control).
    pub fn blank(&mut self) {
        self.blanked = true;
        self.set(BLANK);
    }

    /// Set absolute brightness 0..=255 and treat as activity if non-zero.
    pub fn set_level(&mut self, level: u32) {
        let level = level.min(255);
        if level == 0 {
            self.blank();
        } else {
            self.last_activity = Instant::now();
            self.blanked = false;
            self.set(level);
        }
    }

    pub fn is_available(&self) -> bool {
        self.available
    }
}
