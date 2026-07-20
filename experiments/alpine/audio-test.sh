#!/bin/sh
# Enable MT8127 (MT6323 codec) playback and play a tone. The codec DAPM path
# (clock buffer, DAC, DL1->I2S0 route) only powers while a stream is active on
# the DL1 front-end (device 5 = I2S0DL1_Playback), so start that first.
set +e
DEV="${1:-hw:0,5}"
FREQ="${2:-440}"
# clear the diagnostic sine generators from earlier attempts
amixer -c0 sset Audio_i2s0_SideGen_Switch Off >/dev/null 2>&1
amixer -c0 sset SineTable_DAC_HP Off          >/dev/null 2>&1

(timeout 15 speaker-test -D "$DEV" -c2 -t sine -f "$FREQ" >/dev/null 2>&1) &
TONE=$!
sleep 1                       # stream open -> DAPM can power the path

for x in "AUD_CLK_BUF_Switch On" \
         "Audio_I2S0dl1_hd_Switch On" \
         "Audio_i2s0_hd_Switch On" \
         "Speaker_Amp_Switch On" \
         "Ext_Speaker_Amp_Switch On" \
         "Audio_Speaker_class_Switch CALSSD" \
         "Audio_Speaker_PGA_gain 14Db" \
         "Audio_Amp_L_Switch On" \
         "Audio_Amp_R_Switch On" \
         "Headset_PGAL_GAIN 7Db" \
         "Headset_PGAR_GAIN 7Db"; do
  amixer -c0 sset $x >/dev/null 2>&1
done

echo "latched during $DEV playback:"
for c in AUD_CLK_BUF_Switch Audio_I2S0dl1_hd_Switch Audio_i2s0_hd_Switch Speaker_Amp_Switch Audio_Amp_L_Switch; do
  printf "  %-26s %s\n" "$c" "$(amixer sget $c 2>/dev/null | grep -o 'Item0.*')"
done
echo ">>> TONE PLAYING ~14s on $DEV — LISTEN (speaker AND headphones) <<<"
wait $TONE
echo done
