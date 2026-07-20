# Audio — codec bring-up recipe (DL7006 / MT6323)

**Date:** 2026-07-19
Captured from stock Android (sound-card topology, live) + the matching codec source
(`kernel_amazon_mt8127-common/sound/soc/mediatek/mt_soc_audio_v1/`).

## Honest note on the "mixer state"
The **live** running mixer values are locked on this stock build: it's a `user` build,
SELinux **Enforcing**, `shell` is not in the `audio` group (1005), `/dev/snd` is denied,
there's no `tinymix`, and every playback we could trigger as shell fails
(`pcm_start ... Broken pipe`). So we can't snapshot the running kcontrol values.

What's *more* useful for L1 anyway — where we DO have root — is the authoritative
register sequence from the codec source. That's below.

## The hardware
- **Codec: MT6323 PMIC audio** (`mt_soc_codec_63xx.c`). Analog regs via `Ana_Set_Reg`
  (PMIC pwrap writes). AFE (digital front-end) is the MT8127 SoC side (`AudDrv_Clk.c`).
- **Sound card `mt8127-soc` — 12 DAIs.** The playback front-end we use is
  **device 5 = `I2S0DL1_PLayback`** (`hw:0,5`), feeding DL1 → I2S0 → codec.

## The clock chain (this is the `AUD_CLK_BUF` story)
The ALSA control our `audio-test.sh` toggles maps like this:
```
"AUD_CLK_BUF_Switch" (SOC_ENUM_EXT, mt_soc_codec_63xx.c:2810)
    → Aud_Clk_Buf_Set()
        → clk_buf_ctrl(CLK_BUF_AUDIO, true)      // mt_soc_codec_63xx.c:251
```
`clk_buf_ctrl()` lives in **`mach/mt_clkbuf_ctl.h`** — the **PMIC DCXO clock-buffer**
subsystem. `CLK_BUF_AUDIO` is one buffered 26 MHz output of the MT6323 DCXO. ★ This is
the *same* `clk_buf` class we once blamed for the Wi-Fi wall — and that turned out to be
a firmware-path bug, not the clkbuf. So the audio clkbuf deserves the same fresh, skeptical
look on L1 rather than being treated as an unfixable "PMIC wall."

Full analog clock enable (from `Voice_Call...`/`TurnOnDACPower`, the working order):
```
clk_buf_ctrl(CLK_BUF_AUDIO, true)                 // DCXO audio buffer on (AUD_CLK_BUF)
Ana_Set_Reg(TOP_CLKSQ,        0x0001, 0x0001)     // CKSQ (clock square) enable
Ana_Set_Reg(TOP_CLKSQ_SET,    0x0003, 0xffff)     // CKSQ enable (set-domain)
Ana_Set_Reg(TOP_CKPDN_CON0_CLR, 0x3000, 0xffff)   // release AUD clock power-down (bits 12/13)
Ana_Set_Reg(TOP_CKSEL_CON_CLR,  0x0001, 0x0001)   // use internal 26M
Ana_Set_Reg(AFE_AUDIO_TOP_CON0, 0x0000, 0xffff)   // power on AFE clock
```
Then the analog output block (`ANALDO_CON3`, `AUDBUF_CFG4`, LDOs, DAC, HP depop) and the
DAPM route DL1→I2S0 + `Speaker_Amp_Switch`/`Audio_Amp_L/R_Switch` (which our script
already sets).

## Why L1 was silent (working hypothesis, now sharper)
Our `audio-test.sh` sets `AUD_CLK_BUF_Switch On` but it "won't latch." Given the mapping
above, the real question is whether **`clk_buf_ctrl(CLK_BUF_AUDIO, true)`** actually
enables the DCXO buffer on L1 — i.e. is the `mt_clkbuf` driver initialized, and is the
PMIC pwrap write reaching MT6323? On L1 we have root + can `printk`-instrument
`clk_buf_ctrl` (the codec already logs `+clk_buf_ctrl(CLK_BUF_AUDIO,true)`), and read the
result via `logcat -b kernel` — exactly the technique that cracked Wi-Fi.

## Next (when we tackle audio, after Wi-Fi)
1. On L1, watch `dmesg | grep clk_buf` while a stream is open on `hw:0,5` + toggling
   `AUD_CLK_BUF_Switch` — does `clk_buf_ctrl(CLK_BUF_AUDIO,true)` run and stick?
2. If the DCXO buffer isn't coming up, that's the same PMIC-clkbuf-init gap to close —
   and, like Wi-Fi, likely a missing init step rather than dead hardware.
3. Verify each register in the chain above against `Ana_Get_Reg` dumps.

Source of truth: `mt_soc_codec_63xx.c` (ClsqEnable ~275, clk_buf_ctrl ~250,
Aud_Clk_Buf_Set ~2766), `AudDrv_Clk.c`, `AudDrv_Ana.c`. Script: `experiments/alpine/audio-test.sh`.

## ★★ DeviceTest-confirmed recipe (2026-07-19, `reference/android-capture/devicetest-*.log`)
Ran the stock **`com.DeviceTest`** factory test (Speaker + Mic) and captured the HAL
(`AudioALSA*`) doing the *working* bring-up. The headline: **the HAL barely touches the
mixer — DAPM auto-powers the clock buffer, DAC, MICBIAS, ADC and the DL1↔I2S0 route as a
side-effect of the PCM stream opening on the connected path.** Our `audio-test.sh` was
*over-driving* it with ~11 manual controls (incl. `AUD_CLK_BUF_Switch`) that fight DAPM.

**Speaker (playback) — exactly what the HAL does:**
- Open PCM **`hw:0,5`** (`I2S0DL1_PLayback`), `channels=2 rate=44100 period=2048 count=2`.
- `output_devices = 0x2` (speaker); `routing=2`.
- Turn-on sequence = **ONE control**: `Speaker_Amp_Switch = On`. Nothing else.
- Gains: `SpeakerGain = 5`, `LinoutRGain = 8`.
- Turn-off = `Speaker_Amp_Switch = Off`.

**Mic (capture):**
- Open PCM **`hw:0,1`** (`MultiMedia1_Capture`); `input_source = 1` (MIC),
  `devices = 0x80000004` (built-in mic), mono. No manual `MicBias`/`ADC` controls.

**Revised L1 plan (supersedes the 11-control audio-test.sh):**
1. Open `hw:0,5` and actually **stream audio** (DAPM only powers the path while a
   stream is live). Set **only** `Speaker_Amp_Switch On` + the two gains.
2. `dmesg | grep -iE 'clk_buf|CLKSQ|Audio'` — did DAPM power the clock buffer on its own?
3. If yes → sound. If the clock buffer still doesn't come up, DAPM isn't reaching it →
   *then* instrument `clk_buf_ctrl(CLK_BUF_AUDIO,true)` (don't pre-emptively force controls).
Script: `experiments/net/audio-up.sh` (HAL-faithful, minimal).
Note: even Android hit an occasional `pcm_start ... Broken pipe` race on `hw:0,5` — retry.
