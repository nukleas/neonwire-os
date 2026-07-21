#!/usr/bin/env bash
# l1-isp-diff.sh — capture /proc/driver/isp_reg on NeonOS/L1 while camgrab drives the
# pipeline, and diff the seninf+tg sections against the STOCK working reference.
# Same kernel on both sides => same proc format => direct register diff.
#
# Run from the host (device on NeonOS, reachable over Tailscale/SSH).
set -u
HOST="${HOST:-root@100.x.y.z}"  # set HOST=root@<your-device-tailscale-ip>
REF="$(cd "$(dirname "$0")/.." && pwd)/reference/android-capture/camera-live-20260720/proc-isp_reg-preview.txt"
OUT=/tmp/l1-isp-$(date +%s 2>/dev/null || echo x).txt   # host tmp
SSH="ssh -o ConnectTimeout=8 $HOST"

command -v ssh >/dev/null || { echo "no ssh"; exit 1; }
[ -f "$REF" ] || { echo "missing stock ref: $REF"; exit 1; }

echo "== driving camgrab in background + snapshotting isp_reg mid-run =="
# camgrab configures the pipeline then waits on VF; grab isp_reg during that window.
$SSH '
  /mnt/sd/linux-lab/camgrab /tmp/frame.raw 14 0 >/tmp/camgrab.log 2>&1 &
  CG=$!
  # let it program seninf/csi2/tg (mclk, cal, grab, vf)
  sleep 2
  echo "===== L1 isp_reg (camgrab running) ====="
  cat /proc/driver/isp_reg
  wait $CG 2>/dev/null
' > "$OUT" 2>/dev/null
echo "   saved L1 dump: $OUT ($(wc -l < "$OUT") lines)"

# extract non-zero seninf+tg from the L1 dump in the same shape as the ref
python3 - "$OUT" > "${OUT%.txt}-nz.txt" <<'PY'
import re,sys
lines=open(sys.argv[1]).read().splitlines()
sec=None
print("## L1 seninf+tg non-zero (camgrab):")
for ln in lines:
    if '======' in ln: sec=ln.strip('= ').strip(); continue
    m=re.match(r'\+0x([0-9a-f]+)\s+0x([0-9a-f]+)',ln)
    if not m or sec not in ('seninf','tg'): continue
    if int(m.group(2),16)==0: continue
    if sec=='seninf':
        off=int(m.group(1),16)-0xf1654000+0x8000
        print(f"0x{off:04x} = {m.group(2)}")
    else:
        print(f"tg+0x{int(m.group(1),16)-0xf1650400:03x} = {m.group(2)}")
PY

echo
echo "===== DIFF: STOCK(working) vs L1(broken) — seninf+tg, differing regs ====="
python3 - "$REF" "$OUT" <<'PY'
import re,sys
def parse(fn):
    d={}; sec=None
    for ln in open(fn):
        if '======' in ln: sec=ln.strip('= \n').strip(); continue
        m=re.match(r'\+0x([0-9a-f]+)\s+0x([0-9a-f]+)',ln)
        if not m: continue
        a=int(m.group(1),16); v=m.group(2)
        if sec=='seninf': d[('s',0x8000+(a-0xf1654000))]=v
        elif sec=='tg':   d[('t',a-0xf1650400)]=v
    return d
stock=parse(sys.argv[1]); l1=parse(sys.argv[2])
lab={0x8000:'SENINF_TOP',0x8010:'SENINF1_CTRL',0x8020:'SENINF1_CROP',0x8024:'SENINF IMG SIZE',
0x8028:'SENINF IMG SIZE',0x802c:'SENINF IMG SIZE',0x8030:'SENINF IMG SIZE',0x8108:'CSI2 DT/wordcount',
0x8300:'TG1_PH_CNT'}
print(f"{'reg':<14}{'STOCK':<12}{'L1':<12} note")
for k in sorted(set(stock)|set(l1)):
    typ,off=k; sv=stock.get(k,'--------'); lv=l1.get(k,'--------')
    if sv==lv: continue
    name=f"0x{off:04x}" if typ=='s' else f"tg+0x{off:03x}"
    note=lab.get(off,'') if typ=='s' else {0x18:'GRAB_PXL',0x1c:'GRAB_LIN',0x48:'FRMSIZE'}.get(off,'')
    print(f"{name:<14}{sv:<12}{lv:<12} {note}")
PY
echo
echo ">>> Every line above is a register camgrab sets differently from stock."
echo ">>> Prime suspects for px/line=101: 0x8024-0x8030 (SENINF size), 0x8108 (DT/wordcount)."
echo ">>> Full L1 dump: $OUT"
