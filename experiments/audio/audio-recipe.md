# Audio recipe — CONFIRMED WORKING (2026-07-19)

**Verdict: the HAL-faithful DAPM recipe produces sound.** First audible output from the
DL7006 under NEONWIRE Linux — confirmed by ear (twice: once gloriously glitchy, once clean).
No kernel patching needed; the DCXO clock-buffer question answered itself — DAPM powers the
whole path (clock buffer → DAC → speaker amp) as a side effect of a live stream.

## The recipe (exact, minimal)

```sh
# inside the Alpine chroot (sh /mnt/data/alpine-enter.sh)
amixer -c0 sset Speaker_Amp_Switch On        # the ONE turn-on control (mirrors stock HAL)
amixer -c0 sset Audio_Speaker_PGA_gain 5Db   # volume: MUTE, 0Db..17Db (14Db = loud)
aplay -D hw:0,5 --period-size=2048 --buffer-size=4096 tone.wav
amixer -c0 sset Speaker_Amp_Switch Off       # be polite when done
```

PCM contract for `hw:0,5` (I2S0DL1_Playback), all verified on device:

| Param        | Value                                   |
|--------------|-----------------------------------------|
| Format       | S16_LE, 2ch, 44100 Hz                   |
| Period       | **2048 frames** (HAL geometry)          |
| Buffer       | **4096 frames — hard driver max.** Requesting more gets clamped, and period==buffer is rejected (`aplay: set_params:1414`). Exactly 2 periods fit. |
| Underrun     | `afe_dl1_interrupt_handler underflow` in dmesg + `EPIPE` on write → re-`PREPARE` and continue. speaker-test's tiny writes underflow constantly (that was the horrible noise); aplay with the geometry above plays clean. |

Volume = `Audio_Speaker_PGA_gain`: enum `MUTE, 0Db, 4Db..17Db` (16 items). This is the
Music app's volume control.

## Evidence

- `logs/b1-verdict-20260719.txt` — mixer state + underflow dmesg lines.
- Kernel logs contain **no clk_buf/CLKSQ prints** on this build (pr_debug compiled out), so
  the dmesg-grep verdict in `audio-up.sh` is inconclusive by design on this kernel — the
  *audible tone* is the verdict.
- Known race (Android hits it too): first stream open after boot can return `EPIPE`
  (Broken pipe) — retry once.

## Why it works (the insight, from reference/android-capture/devicetest-main.log)

The stock AudioALSA HAL barely touches the mixer: for speaker output its entire turn-on is
`Speaker_Amp_Switch On` + gains, then it opens `hw:0,5` and lets DAPM auto-power
the clock buffer, DAC, and the DL1→I2S0 route. Forcing controls manually (the old
11-control `audio-test.sh`) *fights* DAPM. Do less, get sound.

## Next (B2/B3, per plan)

- B2: libasound-free PCM writer — direct `/dev/snd/pcmC0D5p` ioctls (tinyalsa-style) in C
  proof then Rust, using the 3.18 tree's `uapi/sound/asound.h` struct layout (load-bearing:
  the struct grew fields in later kernels). Mixer via `/dev/snd/controlC0` ELEM ioctls.
- B3: strudel-core + strudel-dsp render loop → realtime bench on the A7 → Music app sink.
- Capture smoke test (`arecord -D hw:0,1`) still untested.
