//! evdev touch input. Port of neui.c touch_open/map_touch + the tap state machine.
//!
//! LOAD-BEARING (mtk-tpd, type-A multitouch, cost the C author real debugging):
//! the panel emits BTN_TOUCH + ABS_MT_POSITION_X/Y + ABS_MT_TRACKING_ID — no
//! ABS_X/Y. Taps must ARM on the BTN_TOUCH=1 press edge and FIRE on the next
//! SYN_REPORT so the MT coordinates are fresh. Latching on release loses coords;
//! firing on the press edge uses stale ones. Fallback: no-BTN devices arm on the
//! first MT point of a frame.
//!
//! `input_event` is declared by hand: on 32-bit ARM the kernel writes the old
//! 32-bit timeval layout (16-byte struct). libc's definition follows musl 1.2's
//! 64-bit time_t and does NOT match — do not use it.

use std::io;

pub const EV_SYN: u16 = 0;
pub const EV_KEY: u16 = 1;
pub const EV_ABS: u16 = 3;
pub const SYN_REPORT: u16 = 0;
pub const BTN_TOUCH: u16 = 330;
pub const ABS_X: u16 = 0;
pub const ABS_Y: u16 = 1;
pub const ABS_MT_POSITION_X: u16 = 53;
pub const ABS_MT_POSITION_Y: u16 = 54;

/// Kernel ABI input_event for 32-bit ARM (__kernel_old_timeval).
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct InputEvent {
    pub tv_sec: u32,
    pub tv_usec: u32,
    pub type_: u16,
    pub code: u16,
    pub value: i32,
}
const EV_SIZE: usize = std::mem::size_of::<InputEvent>();

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct AbsInfo {
    value: i32,
    minimum: i32,
    maximum: i32,
    fuzz: i32,
    flat: i32,
    resolution: i32,
}

// EVIOCGABS(abs) = _IOR('E', 0x40 + abs, struct input_absinfo[24 bytes])
fn eviocgabs(abs: u16) -> libc::c_int {
    (2u32 << 30 | 24 << 16 | (b'E' as u32) << 8 | (0x40 + abs as u32)) as libc::c_int
}

#[derive(Clone, Copy, Default)]
pub struct TouchOpts {
    pub swap: bool,
    pub flipx: bool,
    pub flipy: bool,
}

pub struct Touch {
    fd: libc::c_int,
    ax_code: u16,
    ay_code: u16,
    min_x: i32,
    max_x: i32,
    min_y: i32,
    max_y: i32,
    opts: TouchOpts,
    // tap state machine
    rx: i32,
    ry: i32,
    pending: bool,
    frame_pt: bool,
    has_btn: bool,
}

impl Touch {
    pub fn open(dev: &str, opts: TouchOpts) -> io::Result<Touch> {
        let cdev = std::ffi::CString::new(dev).unwrap();
        let fd = unsafe { libc::open(cdev.as_ptr(), libc::O_RDONLY | libc::O_NONBLOCK) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // Prefer ABS_X/Y when they carry a range; fall back to type-A MT axes.
        let probe = |abs: u16| -> Option<(i32, i32)> {
            let mut ai = AbsInfo::default();
            if unsafe { libc::ioctl(fd, eviocgabs(abs), &mut ai) } == 0 && ai.maximum > ai.minimum {
                Some((ai.minimum, ai.maximum))
            } else {
                None
            }
        };
        let (ax_code, (min_x, max_x)) = match probe(ABS_X) {
            Some(r) => (ABS_X, r),
            None => (ABS_MT_POSITION_X, probe(ABS_MT_POSITION_X).unwrap_or((0, 1))),
        };
        let (ay_code, (min_y, max_y)) = match probe(ABS_Y) {
            Some(r) => (ABS_Y, r),
            None => (ABS_MT_POSITION_Y, probe(ABS_MT_POSITION_Y).unwrap_or((0, 1))),
        };
        eprintln!(
            "touch {dev}  X[{min_x}..{max_x}] code {ax_code}   Y[{min_y}..{max_y}] code {ay_code}"
        );
        Ok(Touch {
            fd,
            ax_code,
            ay_code,
            min_x,
            max_x: max_x.max(min_x + 1),
            min_y,
            max_y: max_y.max(min_y + 1),
            opts,
            rx: 0,
            ry: 0,
            pending: false,
            frame_pt: false,
            has_btn: false,
        })
    }

    pub fn fd(&self) -> libc::c_int {
        self.fd
    }

    fn map(&self, rx: i32, ry: i32, w: i32, h: i32) -> (i32, i32) {
        let (rx, ry) = if self.opts.swap { (ry, rx) } else { (rx, ry) };
        let mut mx = (rx - self.min_x) * (w - 1) / (self.max_x - self.min_x);
        let mut my = (ry - self.min_y) * (h - 1) / (self.max_y - self.min_y);
        if self.opts.flipx {
            mx = w - 1 - mx;
        }
        if self.opts.flipy {
            my = h - 1 - my;
        }
        (mx.clamp(0, w - 1), my.clamp(0, h - 1))
    }

    /// Feed one event through the tap machine; returns a raw tap on fire.
    fn feed(&mut self, ev: &InputEvent) -> Option<(i32, i32)> {
        match (ev.type_, ev.code) {
            (EV_ABS, c) if c == self.ax_code || c == ABS_MT_POSITION_X => {
                self.rx = ev.value;
                self.frame_pt = true;
            }
            (EV_ABS, c) if c == self.ay_code || c == ABS_MT_POSITION_Y => {
                self.ry = ev.value;
                self.frame_pt = true;
            }
            (EV_KEY, BTN_TOUCH) => {
                self.has_btn = true;
                if ev.value == 1 {
                    self.pending = true; // press edge armed
                }
            }
            (EV_SYN, SYN_REPORT) => {
                if !self.has_btn && self.frame_pt {
                    self.pending = true; // fallback for no-BTN devices
                }
                let fire = self.pending && self.frame_pt;
                self.frame_pt = false;
                if fire {
                    self.pending = false; // fire once, with fresh coords
                    return Some((self.rx, self.ry));
                }
            }
            _ => {}
        }
        None
    }

    /// Drain ALL queued events (fd is non-blocking); return fired taps in
    /// screen space. Improvement over the C loop, which read one event per
    /// poll() wake and could lag multi-event bursts.
    pub fn drain(&mut self, w: i32, h: i32) -> Vec<(i32, i32)> {
        let mut taps = Vec::new();
        let mut buf = [0u8; EV_SIZE * 64];
        loop {
            let n = unsafe { libc::read(self.fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
            if n <= 0 {
                break;
            }
            for chunk in buf[..n as usize].chunks_exact(EV_SIZE) {
                let ev = unsafe { std::ptr::read_unaligned(chunk.as_ptr() as *const InputEvent) };
                if let Some((rx, ry)) = self.feed(&ev) {
                    let (sx, sy) = self.map(rx, ry, w, h);
                    taps.push((sx, sy));
                }
            }
            if (n as usize) < buf.len() {
                break;
            }
        }
        taps
    }

    /// Blocking-ish raw event read for --evdump (caller polls).
    pub fn read_raw(&mut self) -> Option<InputEvent> {
        let mut buf = [0u8; EV_SIZE];
        let n = unsafe { libc::read(self.fd, buf.as_mut_ptr() as *mut libc::c_void, EV_SIZE) };
        if n as usize == EV_SIZE {
            Some(unsafe { std::ptr::read_unaligned(buf.as_ptr() as *const InputEvent) })
        } else {
            None
        }
    }
}

impl Drop for Touch {
    fn drop(&mut self) {
        unsafe { libc::close(self.fd) };
    }
}

/// poll() one fd for readability with timeout_ms; returns true if readable.
pub fn poll_fd(fd: libc::c_int, timeout_ms: i32) -> bool {
    let mut pfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
    let r = unsafe { libc::poll(&mut pfd, 1, timeout_ms) };
    r > 0 && (pfd.revents & libc::POLLIN) != 0
}
