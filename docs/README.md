# Field notebook

These notes were written in the order the work happened. Wrong turns are left
in on purpose — several "impossible" walls were misdiagnoses. If a later file
contradicts an earlier one, believe the later one.

Commands that used to hardcode a private checkout now use `$REPO` for the clone
of this repository:

```sh
export REPO=/path/to/neonwire-os
```

`reference/` (dumps, firmware, vendor trees, logcat captures) is **not in git**.
You will not get a working flash from a clone alone. The writeups assume you
have either the same tablet in hand, or you are only here to read.

`tools/mtkclient` is also untracked. Clone [bkerler/mtkclient](https://github.com/bkerler/mtkclient)
there if you are going to talk to a Preloader.

## If you have an hour

1. The public dossier — [naderlabs.io/projects/neonwire](https://www.naderlabs.io/projects/neonwire)
2. The [root README](../README.md) for what the device does now
3. [24-devlog-wifi-ssh.md](24-devlog-wifi-ssh.md) — the best single lesson in the
   repo (four false walls, then a one-nibble fix)
4. [26-devlog-rust-shell.md](26-devlog-rust-shell.md) — the OS face, sound, camera

## If you have the same tablet

Dump first. Never write a partition you cannot restore.

| Step | Read | Do |
|------|------|----|
| Identify the unit | [01-device-identity.md](01-device-identity.md) | Confirm FCC **XMF-MID7006** / MT8127 |
| Talk to Preloader | [03-flashing.md](03-flashing.md), [08-phase-a-own-flash.md](08-phase-a-own-flash.md) | mtkclient + udev; dump; hash |
| Own boot | [09-phase-b-own-boot.md](09-phase-b-own-boot.md), [10-boot-adb-patch.md](10-boot-adb-patch.md) | Unpack `boot.img`, prove a write |
| Boot our Linux | [14-live-linux-lab.md](14-live-linux-lab.md) | Stock 3.18 kernel + busybox initramfs |
| Face | [17-cyberpunk-fbui.md](17-cyberpunk-fbui.md), then `experiments/neonui/` | C HUD first, Rust shell after |
| Radio | [21-wifi-android-recipe.md](21-wifi-android-recipe.md) → [24](24-devlog-wifi-ssh.md) | CONSYS is BTIF `0x23`, not SDIO |
| Camera | [22-camera-sensor-reference.md](22-camera-sensor-reference.md), [27](27-camera-live-capture.md), `experiments/camera/` | |
| Audio | [23-audio-codec-recipe.md](23-audio-codec-recipe.md), `experiments/audio/` | `hw:0,5` + `Speaker_Amp_Switch` |

The living checklist is [checklist.md](checklist.md). The original mission
statement — now a historical document — is [00-charter.md](00-charter.md).

## The story, in the order it was written

The numbers are chronological, not a tutorial outline.

| | File | What it is |
|--|------|------------|
| 00 | [charter](00-charter.md) | Original mission (own flash / boot / map hardware) |
| 01 | [device identity](01-device-identity.md) | DigiLand DL7006-KB, FCC XMF-MID7006, MT8127 |
| 02 | [live probe](02-live-probe.md) | First USB / Preloader observations |
| 03 | [flashing](03-flashing.md) | Host tooling, Preloader window |
| 04–12 | HA kiosk, stock reflash, ADB boot, skip-wizard | Side quests on stock Android, before Linux |
| 13–16 | Linux port research, live lab, CONSYS power, handoff | The L1 Linux bring-up, still blind on Wi-Fi |
| 17 | [framebuffer UI](17-cyberpunk-fbui.md) | First neon face on `mtkfb` |
| 18–21 | Wi-Fi plans, bisect, Alpine chroot, Android recipe | The radio, still mostly wrong |
| 22–23 | Camera sensor + audio codec recipes from stock | HAL traces, before we had either working |
| 24 | [Wi-Fi + SSH devlog](24-devlog-wifi-ssh.md) | The wall breaks |
| 25 | [Tailscale](25-devlog-tailscale.md) | Off-LAN SSH |
| 26 | [Rust shell](26-devlog-rust-shell.md) | The OS grows up — sound, camera, power |
| 27 | [camera live capture](27-camera-live-capture.md) | Stock-vs-L1 ISP register raid |

Handoff docs ([16](16-handoff-linux-consys.md), [19](19-handoff-wifi-bisect.md))
were written for the next agent in the chair. They freeze a moment in time;
several of their "current blockers" were later falsified. That is the point.

## If you are lifting a subsystem

| Want | Start here |
|------|------------|
| Framebuffer / touch / font | `experiments/neonui/neon-gfx/`, [17](17-cyberpunk-fbui.md) |
| CONSYS Wi-Fi on MT8127 | `experiments/net/wmtctl2.c`, [24](24-devlog-wifi-ssh.md) |
| Raw MIPI camera, no V4L2 | `experiments/camera/`, [22](22-camera-sensor-reference.md), [27](27-camera-live-capture.md) |
| Bare ALSA on MT6323 | `experiments/audio/`, [23](23-audio-codec-recipe.md) |
| On-device Strudel | `experiments/neonui/neon-songs/` (AGPL), [LICENSING.md](../experiments/neonui/LICENSING.md) |
| Preloader dump / flash | [08](08-phase-a-own-flash.md), `tools/` |

Sources consulted while identifying the board: [06-sources.md](06-sources.md).
