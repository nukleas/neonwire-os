//! Framebuffer device: open/mmap/present/shot. Port of fbgfx.h fb_open/fb_present.
//!
//! KEY HARDWARE FACT (from the C engine, cost real debugging time): the
//! ZS070BE3019B3H7II is a command-mode MIPI panel. mtkfb only pushes a frame to
//! the glass on FBIOPAN_DISPLAY, and skips the flush when the offset is
//! unchanged — a plain memcpy into the mmap updates memory but NOT the screen.
//! So `present()` cycles through the hardware buffers (3 on mtkfb) every frame
//! to force a real refresh. This is why memory screenshots can look live while
//! the physical panel sits frozen: only an animated test on glass proves it.

use std::io;

// linux/fb.h ioctls (stable ABI). musl's ioctl takes c_int on 32-bit ARM.
const FBIOGET_VSCREENINFO: libc::c_int = 0x4600;
const FBIOGET_FSCREENINFO: libc::c_int = 0x4602;
const FBIOPAN_DISPLAY: libc::c_int = 0x4606;

// Hand-declared 3.18-era linux/fb.h structs for 32-bit ARM (c_ulong = u32).
// Deliberately NOT from a crate: the quirk handling is custom and the layout
// must match this exact kernel.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct FbBitfield {
    pub offset: u32,
    pub length: u32,
    pub msb_right: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct FbVarScreeninfo {
    pub xres: u32,
    pub yres: u32,
    pub xres_virtual: u32,
    pub yres_virtual: u32,
    pub xoffset: u32,
    pub yoffset: u32,
    pub bits_per_pixel: u32,
    pub grayscale: u32,
    pub red: FbBitfield,
    pub green: FbBitfield,
    pub blue: FbBitfield,
    pub transp: FbBitfield,
    pub nonstd: u32,
    pub activate: u32,
    pub height: u32,
    pub width: u32,
    pub accel_flags: u32,
    pub pixclock: u32,
    pub left_margin: u32,
    pub right_margin: u32,
    pub upper_margin: u32,
    pub lower_margin: u32,
    pub hsync_len: u32,
    pub vsync_len: u32,
    pub sync: u32,
    pub vmode: u32,
    pub rotate: u32,
    pub colorspace: u32,
    pub reserved: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct FbFixScreeninfo {
    pub id: [u8; 16],
    pub smem_start: libc::c_ulong,
    pub smem_len: u32,
    pub r#type: u32,
    pub type_aux: u32,
    pub visual: u32,
    pub xpanstep: u16,
    pub ypanstep: u16,
    pub ywrapstep: u16,
    pub line_length: u32,
    pub mmio_start: libc::c_ulong,
    pub mmio_len: u32,
    pub accel: u32,
    pub capabilities: u16,
    pub reserved: [u16; 2],
}

pub struct Fb {
    fd: libc::c_int,
    mem: *mut u8,
    mem_len: usize,
    /// Composed frame; blitted to the next hw buffer by `present()`.
    pub back: Vec<u8>,
    pub vi: FbVarScreeninfo,
    pub xres: u32,
    pub yres: u32,
    pub stride: u32,
    nbuf: u32,
    cur: u32,
}

impl Fb {
    pub fn open() -> io::Result<Fb> {
        let fd = unsafe { libc::open(c"/dev/fb0".as_ptr(), libc::O_RDWR) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let mut vi: FbVarScreeninfo = Default::default();
        let mut fi: FbFixScreeninfo = unsafe { std::mem::zeroed() };
        // Runtime-probe geometry and channel layout; nothing hardcoded.
        if unsafe { libc::ioctl(fd, FBIOGET_VSCREENINFO, &mut vi) } != 0
            || unsafe { libc::ioctl(fd, FBIOGET_FSCREENINFO, &mut fi) } != 0
        {
            let e = io::Error::last_os_error();
            unsafe { libc::close(fd) };
            return Err(e);
        }
        let (xres, yres, stride) = (vi.xres, vi.yres, fi.line_length);
        let nbuf = (vi.yres_virtual / vi.yres).max(1);
        vi.xoffset = 0;
        vi.yoffset = 0;
        unsafe { libc::ioctl(fd, FBIOPAN_DISPLAY, &vi) };
        let mem_len = stride as usize * vi.yres_virtual as usize;
        let mem = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                mem_len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd,
                0,
            )
        };
        if mem == libc::MAP_FAILED {
            let e = io::Error::last_os_error();
            unsafe { libc::close(fd) };
            return Err(e);
        }
        Ok(Fb {
            fd,
            mem: mem as *mut u8,
            mem_len,
            back: vec![0u8; stride as usize * yres as usize],
            vi,
            xres,
            yres,
            stride,
            nbuf,
            cur: 0,
        })
    }

    /// Blit the composed frame to the NEXT hardware buffer and PAN to it.
    pub fn present(&mut self) {
        self.cur = if self.nbuf > 1 { (self.cur + 1) % self.nbuf } else { 0 };
        let yoff = self.cur * self.yres;
        let frame = self.stride as usize * self.yres as usize;
        unsafe {
            std::ptr::copy_nonoverlapping(
                self.back.as_ptr(),
                self.mem.add(yoff as usize * self.stride as usize),
                frame,
            );
        }
        self.vi.xoffset = 0;
        self.vi.yoffset = yoff;
        unsafe { libc::ioctl(self.fd, FBIOPAN_DISPLAY, &self.vi) };
    }

    /// Pack 0xRRGGBB into the native pixel using the runtime channel layout.
    #[inline]
    pub fn pack(&self, rgb: u32) -> u32 {
        let (r, g, b) = ((rgb >> 16) & 0xff, (rgb >> 8) & 0xff, rgb & 0xff);
        let v = &self.vi;
        ((r >> (8 - v.red.length)) << v.red.offset)
            | ((g >> (8 - v.green.length)) << v.green.offset)
            | ((b >> (8 - v.blue.length)) << v.blue.offset)
            | if v.transp.length > 0 {
                (0xffu32 >> (8 - v.transp.length)) << v.transp.offset
            } else {
                0
            }
    }

    /// Dump the composed back buffer (raw native pixels) for pull_shot.py.
    pub fn shot(&self, path: &str) -> io::Result<()> {
        std::fs::write(path, &self.back)
    }

    /// stdout line consumed by pull_shot.py — keep byte-compatible with the C UI.
    pub fn print_shot_line(&self) {
        println!(
            "SHOT w={} h={} stride={} r={} g={} b={}",
            self.xres,
            self.yres,
            self.stride,
            self.vi.red.offset,
            self.vi.green.offset,
            self.vi.blue.offset
        );
    }
}

impl Drop for Fb {
    fn drop(&mut self) {
        unsafe {
            libc::munmap(self.mem as *mut libc::c_void, self.mem_len);
            libc::close(self.fd);
        }
    }
}
