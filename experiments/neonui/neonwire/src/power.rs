//! Battery-level power management: a low-battery warning banner and a clean
//! auto-shutdown before the pack hits 0% (this session the tablet hard-died at
//! empty — a graceful sync+poweroff avoids fs corruption and the ungraceful
//! crash). Battery state comes from the collectors (/sys/class/power_supply).
//! The CPU governor is already `interactive`, so no cpufreq action is needed.

const LOW_PCT: i32 = 15; // warning banner below this (when discharging)
const CRIT_PCT: i32 = 3; // auto-shutdown at/below this (when discharging)
const CRIT_CONFIRMS: u32 = 2; // consecutive critical reads before acting (~debounce)

#[derive(Clone, Copy, PartialEq)]
pub enum PowerState {
    Ok,
    Low,      // show warning banner
    Shutdown, // imminent: shell draws a final frame then powers off
}

pub struct PowerMgr {
    crit_count: u32,
}

impl PowerMgr {
    pub fn new() -> PowerMgr {
        PowerMgr { crit_count: 0 }
    }

    /// Feed the latest battery reading each slow tick; returns the state to act on.
    pub fn update(&mut self, pct: Option<i32>, charging: bool) -> PowerState {
        let Some(pct) = pct else {
            self.crit_count = 0;
            return PowerState::Ok; // no battery node -> nothing to manage
        };
        if charging {
            self.crit_count = 0;
            return PowerState::Ok;
        }
        if pct <= CRIT_PCT {
            self.crit_count += 1;
            if self.crit_count >= CRIT_CONFIRMS {
                return PowerState::Shutdown;
            }
            return PowerState::Low;
        }
        self.crit_count = 0;
        if pct <= LOW_PCT {
            PowerState::Low
        } else {
            PowerState::Ok
        }
    }
}

/// Clean shutdown: flush disks then power off. Does not return on success.
pub fn power_off() {
    unsafe {
        libc::sync();
        libc::reboot(libc::LINUX_REBOOT_CMD_POWER_OFF);
    }
    // if the syscall was refused, fall back to the poweroff binary
    let _ = std::process::Command::new("/bin/sh").args(["-c", "poweroff -f"]).status();
}
