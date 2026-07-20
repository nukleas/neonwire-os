//! wpa_supplicant control-socket client (UNIX dgram, /tmp/wpa/wlan0).
//! Port of neui.c wpa_open_ctrl/wpa_cmd/wpa_field + scan/network parsers.
//! On any send/recv error the socket is dropped and reopened on the next
//! command (mirrors the C close-and-reopen behavior); the local endpoint is
//! unlinked on Drop (fixes the C leak on crash).

use std::io;

const WPA_CTRL: &str = "/tmp/wpa/wlan0";

pub struct Ap {
    pub ssid: String,
    pub rssi: i32,
    pub wpa: bool,
}

struct Ctrl {
    fd: libc::c_int,
    local: String,
}

impl Drop for Ctrl {
    fn drop(&mut self) {
        unsafe { libc::close(self.fd) };
        let _ = std::fs::remove_file(&self.local);
    }
}

fn sockaddr_un(path: &str) -> libc::sockaddr_un {
    let mut sa: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    sa.sun_family = libc::AF_UNIX as libc::sa_family_t;
    for (i, b) in path.bytes().take(sa.sun_path.len() - 1).enumerate() {
        sa.sun_path[i] = b as libc::c_char;
    }
    sa
}

impl Ctrl {
    fn open() -> io::Result<Ctrl> {
        let fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_DGRAM, 0) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let local = format!("/tmp/.neonui-wpa-{}", std::process::id());
        let _ = std::fs::remove_file(&local);
        let loc = sockaddr_un(&local);
        let rem = sockaddr_un(WPA_CTRL);
        let sz = std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t;
        let ok = unsafe {
            libc::bind(fd, &loc as *const _ as *const libc::sockaddr, sz) == 0
                && libc::connect(fd, &rem as *const _ as *const libc::sockaddr, sz) == 0
        };
        if !ok {
            let e = io::Error::last_os_error();
            unsafe { libc::close(fd) };
            let _ = std::fs::remove_file(&local);
            return Err(e);
        }
        let tv = libc::timeval { tv_sec: 2, tv_usec: 0 };
        unsafe {
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_RCVTIMEO,
                &tv as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::timeval>() as libc::socklen_t,
            );
        }
        Ok(Ctrl { fd, local })
    }

    fn request(&self, cmd: &str) -> io::Result<String> {
        let n = unsafe {
            libc::send(self.fd, cmd.as_ptr() as *const libc::c_void, cmd.len(), 0)
        };
        if n < 0 {
            return Err(io::Error::last_os_error());
        }
        let mut buf = vec![0u8; 8192];
        let n = unsafe {
            libc::recv(self.fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len() - 1, 0)
        };
        if n < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(String::from_utf8_lossy(&buf[..n as usize]).into_owned())
    }
}

/// Lazy-connecting wrapper: `cmd()` opens on demand and drops the socket on
/// any error so the next call reconnects.
#[derive(Default)]
pub struct Wpa {
    ctrl: Option<Ctrl>,
}

impl Wpa {
    /// True if the ctrl socket is (or can be) opened.
    pub fn available(&mut self) -> bool {
        if self.ctrl.is_none() {
            self.ctrl = Ctrl::open().ok();
        }
        self.ctrl.is_some()
    }

    pub fn cmd(&mut self, cmd: &str) -> Option<String> {
        if !self.available() {
            return None;
        }
        match self.ctrl.as_ref().unwrap().request(cmd) {
            Ok(r) => Some(r),
            Err(_) => {
                self.ctrl = None; // force reopen next time
                None
            }
        }
    }

    /// "key=value" field from a STATUS-style reply.
    pub fn field(reply: &str, key: &str) -> Option<String> {
        reply
            .lines()
            .find_map(|l| l.strip_prefix(key).and_then(|r| r.strip_prefix('=')))
            .map(|v| v.to_string())
    }

    /// SCAN_RESULTS -> APs deduped by ssid (keep strongest), sorted by rssi.
    pub fn scan_results(&mut self) -> Option<Vec<Ap>> {
        let buf = self.cmd("SCAN_RESULTS")?;
        let mut aps: Vec<Ap> = Vec::new();
        for line in buf.lines().skip(1) {
            // bssid \t freq \t signal \t flags \t ssid
            let f: Vec<&str> = line.split('\t').collect();
            if f.len() != 5 || f[4].is_empty() {
                continue;
            }
            // mtk driver leaks its NVRAM complaint as a phantom AP — drop it
            if f[4].starts_with("NVRAM WARNING") {
                continue;
            }
            let rssi: i32 = f[2].parse().unwrap_or(-100);
            let wpa = f[3].contains("WPA");
            if let Some(existing) = aps.iter_mut().find(|a| a.ssid == f[4]) {
                existing.rssi = existing.rssi.max(rssi);
            } else if aps.len() < 10 {
                aps.push(Ap { ssid: f[4].to_string(), rssi, wpa });
            }
        }
        aps.sort_by_key(|a| -a.rssi);
        Some(aps)
    }

    /// LIST_NETWORKS -> (id, ssid) pairs.
    pub fn list_networks(&mut self) -> Vec<(i32, String)> {
        let Some(buf) = self.cmd("LIST_NETWORKS") else {
            return Vec::new();
        };
        buf.lines()
            .skip(1)
            .filter_map(|l| {
                let mut f = l.split('\t');
                let id = f.next()?.parse().ok()?;
                Some((id, f.next()?.to_string()))
            })
            .collect()
    }
}

pub fn wlan_present() -> bool {
    std::path::Path::new("/sys/class/net/wlan0").exists()
}

/// Fire-and-forget shell (port of spawn_sh): setsid so it survives us, a
/// detached waiter thread so it never zombifies.
pub fn spawn_sh(cmdline: &str) {
    use std::os::unix::process::CommandExt;
    let mut cmd = std::process::Command::new("/bin/sh");
    cmd.args(["-c", cmdline])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    if let Ok(mut child) = cmd.spawn() {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}
