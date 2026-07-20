//! Status collectors: cheap file reads cached per source with per-source cadence.
//! Everything degrades to placeholders — the status bar never blocks or panics.

use std::time::{Duration, Instant};

pub struct Snapshot {
    pub host: String,
    pub kernel: String,
    pub machine: String,
    pub cpus: i32,
    pub uptime_s: u64,
    pub load1: String,
    pub mem_total_kb: i64,
    pub mem_avail_kb: i64,
    pub batt_pct: Option<i32>,
    pub batt_charging: bool,
    pub wlan_ip: Option<String>,
    pub ts_ip: Option<String>,
    pub rssi_bars: Option<i32>, // 0..=4
}

impl Default for Snapshot {
    fn default() -> Self {
        Snapshot {
            host: String::new(),
            kernel: String::new(),
            machine: String::new(),
            cpus: 0,
            uptime_s: 0,
            load1: "-".into(),
            mem_total_kb: 0,
            mem_avail_kb: 0,
            batt_pct: None,
            batt_charging: false,
            wlan_ip: None,
            ts_ip: None,
            rssi_bars: None,
        }
    }
}

pub struct Collectors {
    pub snap: Snapshot,
    batt_path: Option<String>,
    last_fast: Option<Instant>,
    last_slow: Option<Instant>,
}

impl Collectors {
    pub fn new() -> Collectors {
        // One-time: uname + battery sysfs discovery (names vary per board).
        let mut snap = Snapshot::default();
        let mut un: libc::utsname = unsafe { std::mem::zeroed() };
        if unsafe { libc::uname(&mut un) } == 0 {
            let s = |a: &[libc::c_char]| {
                unsafe { std::ffi::CStr::from_ptr(a.as_ptr()) }.to_string_lossy().into_owned()
            };
            snap.host = s(&un.nodename);
            snap.kernel = s(&un.release);
            snap.machine = s(&un.machine);
        }
        let batt_path = std::fs::read_dir("/sys/class/power_supply")
            .ok()
            .and_then(|rd| {
                rd.filter_map(|e| e.ok()).map(|e| e.path()).find(|p| {
                    std::fs::read_to_string(p.join("type"))
                        .map(|t| t.trim() == "Battery")
                        .unwrap_or(false)
                        && p.join("capacity").exists()
                })
            })
            .map(|p| p.to_string_lossy().into_owned());
        let mut c = Collectors { snap, batt_path, last_fast: None, last_slow: None };
        c.refresh();
        c
    }

    /// Call every tick; internally rate-limits (fast: 1 s, slow: 10 s).
    pub fn refresh(&mut self) {
        let now = Instant::now();
        if self.last_fast.is_none_or(|t| now - t >= Duration::from_millis(950)) {
            self.last_fast = Some(now);
            self.fast();
        }
        if self.last_slow.is_none_or(|t| now - t >= Duration::from_secs(10)) {
            self.last_slow = Some(now);
            self.slow();
        }
    }

    fn fast(&mut self) {
        let s = &mut self.snap;
        if let Ok(up) = std::fs::read_to_string("/proc/uptime") {
            s.uptime_s = up.split('.').next().and_then(|v| v.parse().ok()).unwrap_or(0);
        }
        if let Ok(la) = std::fs::read_to_string("/proc/loadavg") {
            s.load1 = la.split_whitespace().next().unwrap_or("-").to_string();
        }
        if let Ok(mi) = std::fs::read_to_string("/proc/meminfo") {
            for line in mi.lines() {
                if let Some(v) = line.strip_prefix("MemTotal:") {
                    s.mem_total_kb = v.trim().trim_end_matches(" kB").parse().unwrap_or(0);
                } else if let Some(v) = line.strip_prefix("MemAvailable:") {
                    s.mem_avail_kb = v.trim().trim_end_matches(" kB").parse().unwrap_or(0);
                }
            }
        }
        if s.cpus == 0 {
            // "0-3" -> 4; /proc/cpuinfo only lists cores currently online (hotplug)
            s.cpus = std::fs::read_to_string("/sys/devices/system/cpu/present")
                .ok()
                .and_then(|p| p.trim().rsplit('-').next()?.parse::<i32>().ok().map(|n| n + 1))
                .unwrap_or(1);
        }
    }

    fn slow(&mut self) {
        let s = &mut self.snap;
        s.wlan_ip = if_ip("wlan0");
        s.ts_ip = if_ip("tailscale0");
        s.rssi_bars = wireless_bars();
        if let Some(bp) = &self.batt_path {
            s.batt_pct = std::fs::read_to_string(format!("{bp}/capacity"))
                .ok()
                .and_then(|v| v.trim().parse().ok());
            s.batt_charging = std::fs::read_to_string(format!("{bp}/status"))
                .map(|v| v.trim() == "Charging")
                .unwrap_or(false);
        }
    }
}

/// IPv4 of an interface via SIOCGIFADDR (port of neui.c wlan_ip()).
fn if_ip(ifname: &str) -> Option<String> {
    const SIOCGIFADDR: libc::c_int = 0x8915;
    let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0) };
    if fd < 0 {
        return None;
    }
    let mut ifr: libc::ifreq = unsafe { std::mem::zeroed() };
    for (i, b) in ifname.bytes().take(15).enumerate() {
        ifr.ifr_name[i] = b as libc::c_char;
    }
    let r = unsafe { libc::ioctl(fd, SIOCGIFADDR, &mut ifr) };
    unsafe { libc::close(fd) };
    if r != 0 {
        return None;
    }
    let addr = unsafe { &*(&ifr.ifr_ifru as *const _ as *const libc::sockaddr_in) };
    // s_addr is network byte order: memory order IS octet order
    let b = addr.sin_addr.s_addr.to_ne_bytes();
    Some(format!("{}.{}.{}.{}", b[0], b[1], b[2], b[3]))
}

/// Wi-Fi link quality from /proc/net/wireless -> 0..=4 bars (wext drivers).
fn wireless_bars() -> Option<i32> {
    let txt = std::fs::read_to_string("/proc/net/wireless").ok()?;
    let line = txt.lines().find(|l| l.trim_start().starts_with("wlan0:"))?;
    // "wlan0: 0000   54.  -56.  -256 ..." -> second field = link quality 0..~70
    let q: f32 = line.split_whitespace().nth(2)?.trim_end_matches('.').parse().ok()?;
    Some(((q / 70.0 * 4.0).round() as i32).clamp(0, 4))
}
