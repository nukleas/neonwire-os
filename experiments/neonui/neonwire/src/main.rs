//! neonwire — NEONWIRE OS shell.
//!
//! Default mode runs the shell (status bar + home + apps). Diagnostic modes:
//! --m0 (toolchain probe), --smoke (fb animation), --card (engine test card),
//! --probe/--evdump (touch calibration), --shot/--tap (headless render/tap).

mod apps;
mod audio;
mod backlight;
mod collectors;
mod power;
mod shell;
mod statusbar;
mod widgets;
mod wpa;

use std::time::{Duration, SystemTime, UNIX_EPOCH};

// arm EABI syscall number for getrandom(2) (kernel >= 3.17).
const SYS_GETRANDOM: libc::c_long = 384;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("--m0") => m0_probe(),
        Some("--smoke") => smoke(args.get(2).map(String::as_str)),
        Some("--card") => testcard(args.get(2).map(String::as_str)),
        Some("--probe") => probe_touch(&args, false),
        Some("--evdump") => probe_touch(&args, true),
        Some("--tone") => tone(),
        Some("--wpa-probe") => wpa_probe(),
        Some("--power-test") => power_test(),
        _ => run_shell(&args),
    }
}

/// Dry-run the battery state machine (no actual poweroff) to prove thresholds.
fn power_test() {
    use power::{PowerMgr, PowerState};
    let name = |s: PowerState| match s {
        PowerState::Ok => "OK",
        PowerState::Low => "LOW(warn)",
        PowerState::Shutdown => "SHUTDOWN",
    };
    let mut pm = PowerMgr::new();
    // (pct, charging) sequence exercising the debounce + charging guard
    let seq = [
        (Some(80), false),
        (Some(15), false),
        (Some(12), true), // charging clears the warning
        (Some(3), true),  // critical but charging -> no shutdown
        (Some(3), false), // 1st critical read -> LOW
        (Some(3), false), // 2nd -> SHUTDOWN
        (Some(50), false),
    ];
    for (pct, chg) in seq {
        println!("pct={:?} charging={} -> {}", pct, chg, name(pm.update(pct, chg)));
    }
    println!("POWER-TEST DONE (thresholds: LOW<=15, CRIT<=3, 2 confirms, charging clears)");
}

/// M6a verification: exercise the wpa ctrl client headlessly over SSH.
fn wpa_probe() {
    let mut w = wpa::Wpa::default();
    println!("wlan present: {}", wpa::wlan_present());
    println!("ctrl available: {}", w.available());
    if let Some(st) = w.cmd("STATUS") {
        println!(
            "wpa_state={:?} ssid={:?}",
            wpa::Wpa::field(&st, "wpa_state"),
            wpa::Wpa::field(&st, "ssid")
        );
    }
    println!("known networks: {:?}", w.list_networks());
    if w.cmd("SCAN").is_some() {
        println!("scan sent; waiting 4s...");
        std::thread::sleep(Duration::from_secs(4));
        match w.scan_results() {
            Some(aps) => {
                for ap in aps {
                    println!("  {:<26} {:>5} dBm  {}", ap.ssid, ap.rssi, if ap.wpa { "WPA2" } else { "open" });
                }
            }
            None => println!("scan_results failed"),
        }
    }
    println!("WPA-PROBE DONE");
}

/// B2 verification: 2 s of 440 Hz through the raw-ioctl PCM path, no libasound.
fn tone() {
    audio::speaker_amp(true);
    std::thread::sleep(Duration::from_millis(400)); // let amixer land
    let pcm = match audio::Pcm::open() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("pcm open: {e}");
            audio::speaker_amp(false);
            std::process::exit(1);
        }
    };
    println!("pcm configured: S16_LE 2ch 44100, period 2048 x2");
    let mut buf = vec![0i16; audio::PERIOD * audio::CHANNELS];
    let mut phase = 0.0f32;
    let step = 2.0 * std::f32::consts::PI * 440.0 / audio::RATE as f32;
    for _ in 0..(2 * audio::RATE as usize / audio::PERIOD) {
        for f in 0..audio::PERIOD {
            let s = (phase.sin() * 12000.0) as i16;
            phase += step;
            buf[f * 2] = s;
            buf[f * 2 + 1] = s;
        }
        if let Err(e) = pcm.write(&buf) {
            eprintln!("write: {e}");
            audio::speaker_amp(false);
            std::process::exit(1);
        }
    }
    audio::speaker_amp(false);
    println!("TONE DONE");
}

fn run_shell(args: &[String]) {
    let fb = match neon_gfx::fb::Fb::open() {
        Ok(fb) => fb,
        Err(e) => {
            eprintln!("fb: {e}");
            std::process::exit(1);
        }
    };
    // headless modes: --shot PATH renders one frame; --tap X Y (repeatable) taps first
    let mut shot_path = None;
    let mut taps = Vec::new();
    let mut ticks = 0u32;
    let mut dev = "/dev/input/event4".to_string();
    let mut opts = neon_gfx::input::TouchOpts::default();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--shot" if i + 1 < args.len() => {
                shot_path = Some(args[i + 1].clone());
                i += 1;
            }
            "--ticks" if i + 1 < args.len() => {
                ticks = args[i + 1].parse().unwrap_or(0);
                i += 1;
            }
            "--tap" if i + 2 < args.len() => {
                let x = args[i + 1].parse().unwrap_or(0);
                let y = args[i + 2].parse().unwrap_or(0);
                taps.push((x, y));
                i += 2;
            }
            "--dev" if i + 1 < args.len() => {
                dev = args[i + 1].clone();
                i += 1;
            }
            "--swap" => opts.swap = true,
            "--flipx" => opts.flipx = true,
            "--flipy" => opts.flipy = true,
            _ => {}
        }
        i += 1;
    }
    // NOTE: no SIGCHLD=SIG_IGN here (it would auto-reap and break Child::try_wait
    // for camprobe). M6's fire-and-forget wifi spawns will need per-spawn handling.

    if shot_path.is_some() || !taps.is_empty() {
        let mut sh = shell::Shell::new(fb, None);
        sh.shot(&taps, ticks, shot_path.as_deref());
        return;
    }
    let touch = neon_gfx::input::Touch::open(&dev, opts)
        .map_err(|e| eprintln!("touch {dev}: {e} (running without input)"))
        .ok();
    shell::Shell::new(fb, touch).run();
}

/// M3: touch calibration. --probe prints mapped taps; --evdump prints raw events.
/// Optional trailing arg = seconds to run (default 15, for headless SSH runs).
fn probe_touch(args: &[String], raw: bool) {
    use neon_gfx::input::{poll_fd, Touch, TouchOpts};
    let dev = args
        .iter()
        .position(|a| a == "--dev")
        .and_then(|i| args.get(i + 1).cloned())
        .unwrap_or_else(|| "/dev/input/event4".into());
    let secs: u64 = args.last().and_then(|s| s.parse().ok()).unwrap_or(15);
    let opts = TouchOpts {
        swap: args.iter().any(|a| a == "--swap"),
        flipx: args.iter().any(|a| a == "--flipx"),
        flipy: args.iter().any(|a| a == "--flipy"),
    };
    let mut t = match Touch::open(&dev, opts) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{dev}: {e}");
            std::process::exit(1);
        }
    };
    println!("{}: tap the screen now ({secs}s)...", if raw { "evdump" } else { "probe" });
    let deadline = std::time::Instant::now() + Duration::from_secs(secs);
    while std::time::Instant::now() < deadline {
        if !poll_fd(t.fd(), 500) {
            continue;
        }
        if raw {
            while let Some(ev) = t.read_raw() {
                println!("ev type={} code={} val={}", ev.type_, ev.code, ev.value);
            }
        } else {
            for (sx, sy) in t.drain(1024, 600) {
                println!("tap screen({sx},{sy})");
            }
        }
    }
    println!("PROBE DONE");
}

/// M2: static test card exercising every engine primitive, for --shot comparison
/// against the C renderer.
fn testcard(shot_path: Option<&str>) {
    use neon_gfx::theme::*;
    let mut fb = match neon_gfx::fb::Fb::open() {
        Ok(fb) => fb,
        Err(e) => {
            eprintln!("fb: {e}");
            std::process::exit(1);
        }
    };
    fb.print_shot_line();
    {
        let mut c = fb.canvas();
        c.background();

        // header: glow wordmark + tracked label
        c.textg(24, 18, "NEONWIRE OS", CYAN, 2);
        c.text_tracked(24, 74, "RUST ENGINE TEST CARD // M2", TEXT2, 1, 4);
        c.hline(24, 100, c.w - 48, BORDER);

        // panels with titles + accents
        c.panel(24, 130, 300, 180, CYAN, "PANEL/CYAN");
        c.panel(348, 130, 300, 180, MAGENTA, "PANEL/MAGENTA");
        c.panel(672, 130, 328, 180, AMBER, "PANEL/AMBER");

        // meters at several fills
        for (i, pct) in [12, 35, 60, 88, 100].iter().enumerate() {
            let y = 150 + i as i32 * 30;
            c.bar(48, y, 250, 18, *pct, CYAN);
        }
        // glyph set
        c.text(372, 150, "ABCDEFGHIJKLM", TEXT, 1);
        c.text(372, 176, "NOPQRSTUVWXYZ", TEXT, 1);
        c.text(372, 202, "abcdefghijklm", TEXT2, 1);
        c.text(372, 228, "0123456789!?#$", GREEN, 1);
        c.textg(372, 258, "GLOW 2X", MAGENTA, 2);
        // palette swatches
        let sw = [BG2, BORDER, CYAN, CYANHI, MAGENTA, GREEN, AMBER, GOLD, PURPLE, RED, TEXT, WHITE];
        for (i, col) in sw.iter().enumerate() {
            let x = 690 + (i as i32 % 6) * 52;
            let y = 160 + (i as i32 / 6) * 60;
            c.fill(x, y, 44, 44, *col);
            c.neonbox(x, y, 44, 44, BORDER);
        }

        // status colors + neonboxes + corners standalone
        c.panel(24, 340, 976, 200, GREEN, "PRIMITIVES");
        c.neonbox(48, 370, 120, 60, CYAN);
        c.neonbox(188, 370, 120, 60, MAGENTA);
        c.corners(328, 370, 120, 60, AMBER, 16);
        for (i, (label, col)) in
            [("OK", GREEN), ("WARN", AMBER), ("ERR", RED), ("INFO", BLUE)].iter().enumerate()
        {
            let x = 480 + i as i32 * 120;
            c.fill(x, 380, 100, 34, mix_bg(*col));
            c.neonbox(x, 380, 100, 34, *col);
            c.text(x + 14, 385, label, *col, 1);
        }
        c.text_tracked(48, 460, "WIDE TRACKED LABEL 0.3EM", TEXT_MUTED, 1, 5);
        c.bar(48, 495, 400, 22, 72, GREEN);
        c.scanlines(480, 450, 480, 80);

        // footer
        c.hline(24, 560, c.w - 48, BORDER);
        c.text(24, 570, "> rust engine online", GREEN, 1);
    }
    fb.present();
    if let Some(p) = shot_path {
        if let Err(e) = fb.shot(p) {
            eprintln!("shot: {e}");
        }
    }
    println!("CARD DONE");
}

fn mix_bg(c: u32) -> u32 {
    neon_gfx::canvas::mix(neon_gfx::theme::BG, c, 30)
}

/// M1: animated gradient + bouncing box at ~5 fps. Watching the physical panel
/// is the point — it proves FBIOPAN_DISPLAY is flushing (memory screenshots lie).
fn smoke(shot_path: Option<&str>) {
    let mut fb = match neon_gfx::fb::Fb::open() {
        Ok(fb) => fb,
        Err(e) => {
            eprintln!("fb: {e}");
            std::process::exit(1);
        }
    };
    fb.print_shot_line();
    let (w, h, stride) = (fb.xres as usize, fb.yres as usize, fb.stride as usize);

    for frame in 0u32..50 {
        let t = frame as f32 / 50.0;
        // vertical gradient bg #05060a -> cyan-tinted, phase-shifted per frame
        for y in 0..h {
            let f = y as f32 / h as f32;
            let wave = ((f * 6.28 + t * 6.28).sin() * 0.5 + 0.5) * 0.25;
            let r = (0x05 as f32 + 0x20 as f32 * wave) as u32;
            let g = (0x06 as f32 + 0x60 as f32 * wave * f) as u32;
            let b = (0x0a as f32 + 0x70 as f32 * (wave + f * 0.3)) as u32;
            let px = fb.pack((r << 16) | (g << 8) | b).to_ne_bytes();
            let row = &mut fb.back[y * stride..y * stride + w * 4];
            for chunk in row.chunks_exact_mut(4) {
                chunk.copy_from_slice(&px);
            }
        }
        // bouncing cyan box with magenta border
        let bw = 120usize;
        let bx = ((w - bw) as f32 * (0.5 + 0.5 * (t * 12.56).sin())) as usize;
        let by = ((h - bw) as f32 * (0.5 + 0.5 * (t * 8.4).cos())) as usize;
        let cyan = fb.pack(0x47f6ff).to_ne_bytes();
        let magenta = fb.pack(0xff2bd6).to_ne_bytes();
        for y in by..by + bw {
            let row = &mut fb.back[y * stride + bx * 4..y * stride + (bx + bw) * 4];
            let border = y < by + 4 || y >= by + bw - 4;
            for (i, chunk) in row.chunks_exact_mut(4).enumerate() {
                let edge = border || i < 4 || i >= bw - 4;
                chunk.copy_from_slice(if edge { &magenta } else { &cyan });
            }
        }
        fb.present();
        std::thread::sleep(Duration::from_millis(200));
    }
    if let Some(p) = shot_path {
        if let Err(e) = fb.shot(p) {
            eprintln!("shot: {e}");
        }
    }
    println!("SMOKE DONE 50 frames");
}

fn m0_probe() {
    println!("NEONWIRE M0 PROBE — rust on dl7006");

    // 1. uname — proves basic libc struct ABI sanity.
    let mut un: libc::utsname = unsafe { std::mem::zeroed() };
    if unsafe { libc::uname(&mut un) } == 0 {
        let cstr = |a: &[libc::c_char]| unsafe { std::ffi::CStr::from_ptr(a.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        println!("uname: {} {} {}", cstr(&un.sysname), cstr(&un.release), cstr(&un.machine));
    } else {
        println!("uname: FAILED errno={}", std::io::Error::last_os_error());
    }

    // 2. time64 fallback — SystemTime uses clock_gettime64 on musl 1.2; the 3.18
    //    kernel lacks it, so musl must fall back via ENOSYS. A sane epoch (>2020)
    //    and a working 1s sleep prove the fallback path end-to-end.
    let t0 = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or(Duration::ZERO);
    std::thread::sleep(Duration::from_millis(1000));
    let t1 = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or(Duration::ZERO);
    let delta = t1.as_millis() as i64 - t0.as_millis() as i64;
    let epoch_ok = t0.as_secs() > 1_600_000_000; // > Sep 2020 = clock actually read
    println!(
        "time: epoch={} delta_ms={} [{}]",
        t0.as_secs(),
        delta,
        if epoch_ok && (900..=1500).contains(&delta) { "OK" } else { "FAIL" }
    );

    // 3. getrandom(2) — what rand/getrandom crates use; syscall 384 on arm.
    let mut buf = [0u8; 16];
    let n = unsafe { libc::syscall(SYS_GETRANDOM, buf.as_mut_ptr(), buf.len(), 0) };
    println!(
        "getrandom: ret={} bytes={:02x}{:02x}{:02x}{:02x}.. [{}]",
        n,
        buf[0],
        buf[1],
        buf[2],
        buf[3],
        if n == 16 { "OK" } else { "FAIL" }
    );

    // 4. std fs.
    match std::fs::read_to_string("/proc/version") {
        Ok(v) => println!("fs: /proc/version = {}", v.trim()),
        Err(e) => println!("fs: FAIL {e}"),
    }

    // 5. process spawn (the shell relies on this for tools/wifi bring-up).
    match std::process::Command::new("/bin/sh").args(["-c", "echo spawn-ok"]).output() {
        Ok(o) => println!(
            "spawn: {} [{}]",
            String::from_utf8_lossy(&o.stdout).trim(),
            if o.status.success() { "OK" } else { "FAIL" }
        ),
        Err(e) => println!("spawn: FAIL {e}"),
    }

    // 6. strudel spike — proves the edition-2024 path deps cross-compile and run.
    strudel_spike();

    println!("M0 DONE");
}

fn strudel_spike() {
    use strudel_core::{fastcat, pure};
    let pat = fastcat(vec![
        pure("bd".into()),
        pure("sd".into()),
        pure("hh".into()),
        pure("cp".into()),
    ]);
    let haps = pat.query_cycle(0);
    print!("strudel: cycle0 {} haps:", haps.len());
    for h in &haps {
        print!(" {:?}@{}", h.value, h.part.begin);
    }
    println!(" [{}]", if haps.len() == 4 { "OK" } else { "FAIL" });
}
