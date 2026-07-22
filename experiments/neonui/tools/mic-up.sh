#!/bin/sh
# mic-up — HAL-ish built-in mic path for DL7006 / MT6323 (L1 bare Linux).
#
# Stock Android audio_device.xml path "builtin_Mic_SingleMic" / "Mic1":
#   MicSource1=ADC1, ADC1+ADC2 On, Preamp1=IN_ADC1, Preamp2=IN_ADC3
# Plus gain / handset PGA so STT hears speech.
#
# DAPM may not auto-power the analog path for bare arecord the way it does for
# speaker playback — without this, hw:0,1 records near-silence (peak ~500).
#
# Usage: sh mic-up.sh          # arm for capture
#        sh mic-up.sh off      # optional teardown

set +e
ALP=/mnt/data/alpine
AMIXER="$ALP/usr/bin/amixer"
if [ ! -x "$AMIXER" ] && [ ! -L "$AMIXER" ]; then
  echo "mic-up: need alpine amixer at $AMIXER" >&2
  exit 1
fi
for d in proc sys dev; do
  mountpoint -q "$ALP/$d" 2>/dev/null || mount --bind "/$d" "$ALP/$d" 2>/dev/null || true
done
am() { chroot "$ALP" /usr/bin/amixer -c0 "$@" >/dev/null 2>&1; }

if [ "$1" = "off" ]; then
  am sset Audio_Preamp1_Switch OPEN
  am sset Audio_Preamp2_Switch OPEN
  am sset Audio_ADC_1_Switch Off
  am sset Audio_ADC_2_Switch Off
  am sset Voice_Amp_Switch Off
  echo "mic-up: off"
  exit 0
fi

# Mode / source / preamps / gain first — do NOT leave ADC On before open.
# MT6323 TurnOnADcPower latches UL rate from the open PCM; pre-arming ADC
# before arecord yields digital zeros at 16 kHz. Callers should:
#   mic-up.sh → start arecord → mic-up.sh rearm  (or rely on neonwire live-arm)
am sset Audio_MIC1_Mode_Select ACCMODE
am sset Audio_MicSource1_Setting ADC1
am sset Audio_ADC_1_Sel Preamp
am sset Audio_ADC_2_Sel Preamp
am sset Audio_Preamp1_Switch IN_ADC1
am sset Audio_Preamp2_Switch IN_ADC3

# Gain: 24dB + software AGC works better on this weak MEMS than 18dB alone.
am sset Audio_PGA1_Setting 24Db
am sset Audio_PGA2_Setting 24Db
am sset Handset_PGA_GAIN 9Db
am sset Voice_Amp_Switch On

if [ "$1" = "rearm" ]; then
  # cold re-toggle after PCM is open
  am sset Audio_ADC_1_Switch Off
  am sset Audio_ADC_2_Switch Off
  am sset Audio_ADC_1_Switch On
  am sset Audio_ADC_2_Switch On
  echo "mic-up: ADC rearmed (post-open)"
  exit 0
fi

# Default: leave ADC On for interactive use; neonwire does Off→open→On itself.
am sset Audio_ADC_1_Switch On
am sset Audio_ADC_2_Switch On

echo "mic-up: SingleMic path armed (ADC1/2 On, Preamp IN_ADC1, PGA 24dB)"
# show state
for c in Audio_ADC_1_Switch Audio_Preamp1_Switch Audio_PGA1_Setting Handset_PGA_GAIN Voice_Amp_Switch; do
  v=$(chroot "$ALP" /usr/bin/amixer -c0 sget "$c" 2>/dev/null | grep Item0 | head -1)
  echo "  $c $v"
done
