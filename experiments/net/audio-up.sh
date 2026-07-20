#!/bin/sh
# audio-up — HAL-faithful speaker bring-up for L1, from the stock DeviceTest capture.
#
# THE INSIGHT (reference/android-capture/devicetest-main.log): the stock AudioALSA HAL
# barely touches the mixer. For speaker_output its ENTIRE turn-on sequence is ONE control
# (`Speaker_Amp_Switch On`) + two gains. DAPM auto-powers the clock buffer, DAC, and the
# DL1->I2S0 route as a side-effect of a live PCM stream on hw:0,5. Our old audio-test.sh
# forced ~11 controls (incl AUD_CLK_BUF_Switch) that FIGHT DAPM. This mirrors the HAL:
# open the stream, set only what the HAL sets, and let DAPM do the rest — then look at
# whether the clock buffer came up ON ITS OWN.
#
#   sh audio-up.sh            # play a test tone on the speaker
#   sh audio-up.sh 660        # 660 Hz
set +e
FREQ="${1:-440}"
DEV=hw:0,5                     # I2S0DL1_PLayback (exactly what the HAL opens)
say(){ echo; echo "==== $* ===="; }

say "0. clear any forced controls from earlier experiments (let DAPM own the path)"
for c in AUD_CLK_BUF_Switch Audio_i2s0_SideGen_Switch SineTable_DAC_HP \
         Audio_I2S0dl1_hd_Switch Audio_i2s0_hd_Switch; do
  amixer -c0 sset "$c" Off >/dev/null 2>&1
done

say "1. HAL turn-on sequence (the ONLY controls stock sets for speaker_output)"
amixer -c0 sset 'Speaker_Amp_Switch' On            # the one turn-on control
amixer -c0 sset 'Audio_Speaker_PGA_gain' '14Db' >/dev/null 2>&1   # gain ~= SpeakerGain 5
echo "  Speaker_Amp_Switch -> $(amixer -c0 sget 'Speaker_Amp_Switch' 2>/dev/null | grep -o 'Item0.*')"

say "2. open a LIVE stream on $DEV (DAPM only powers the path while streaming)"
echo "  (44100/2ch/2048 like the HAL; retry once on Broken-pipe race — Android hits it too)"
( speaker-test -D "$DEV" -c2 -r44100 -t sine -f "$FREQ" -p2048 >/tmp/spk.log 2>&1 || \
  speaker-test -D "$DEV" -c2 -r44100 -t sine -f "$FREQ" -p2048 >/tmp/spk.log 2>&1 ) &
TONE=$!
sleep 2                       # let the stream open + DAPM settle

say "3. did DAPM power the clock buffer ON ITS OWN? (the real question)"
dmesg 2>/dev/null | grep -iE 'clk_buf|CLK_BUF|CLKSQ|Clsq|AudDrv_Clk|Aud_.*Clk_cntr' | tail -12
echo "  --- current codec clock-ish controls ---"
for c in AUD_CLK_BUF_Switch Speaker_Amp_Switch; do
  printf "  %-22s %s\n" "$c" "$(amixer -c0 sget "$c" 2>/dev/null | grep -o 'Item0.*')"
done

say "4. >>> TONE PLAYING ~ a few seconds on $DEV — LISTEN <<<"
wait $TONE
echo "  speaker-test log:"; tail -3 /tmp/spk.log 2>/dev/null | sed 's/^/    /'

say "VERDICT"
if dmesg 2>/dev/null | grep -qiE 'clk_buf.*AUDIO.*true|CLKSQ.*Enable|Aud_ANA_Clk_cntr:[1-9]'; then
  echo "  DAPM powered the audio clock — if silent, chase the analog output block next."
else
  echo "  clock buffer did NOT come up via DAPM → instrument clk_buf_ctrl(CLK_BUF_AUDIO)"
  echo "  (mt_soc_codec_63xx.c:251) — same logd/dmesg technique that cracked Wi-Fi."
fi
