#!/usr/bin/env python3
"""Render neofb on the tablet, pull the framebuffer screenshot, save a PNG.

  ./pull_shot.py [--bin /mnt/sd/linux-lab/neofb] [--out shot.png]

Runs `<bin> --shot /tmp/shot.raw`, then `gzip|base64` the raw pane back over
serial between sentinels, gunzips it, and reconstructs a PNG using the pixel
layout neofb reports (stride + r/g/b channel offsets).
"""
from __future__ import annotations

import argparse
import base64
import gzip
import re
import sys
import time

import serial
from PIL import Image

PORT, BAUD = "/dev/ttyACM0", 115200
BEGIN, END = "__B64_BEGIN__", "__B64_END__"


def send(ser, s):
    ser.write(s.encode()); ser.flush()


def read_until(ser, needle, timeout):
    end = time.time() + timeout
    buf = b""
    while time.time() < end:
        c = ser.read(8192)
        if c:
            buf += c
            if needle.encode() in buf:
                return buf.decode(errors="replace")
        else:
            time.sleep(0.02)
    return buf.decode(errors="replace")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", default="/mnt/sd/linux-lab/neofb")
    ap.add_argument("--out", default="shot.png")
    ap.add_argument("--args", default="", help="extra args appended to the render command")
    args = ap.parse_args()

    ser = serial.Serial(PORT, BAUD, timeout=0.3)
    try:
        send(ser, "\r\n"); read_until(ser, "#", 1.0)
        send(ser, f"{args.bin} --shot /tmp/shot.raw {args.args}\r\n")
        info = read_until(ser, "SHOT", 6.0)
        m = re.search(r"SHOT w=(\d+) h=(\d+) stride=(\d+) r=(\d+) g=(\d+) b=(\d+)", info)
        if not m:
            print("no SHOT line:\n" + info, file=sys.stderr); sys.exit(1)
        w, h, stride, ro, go, bo = map(int, m.groups())
        read_until(ser, "#", 2.0)
        print(f"pane {w}x{h} stride={stride} rgb@{ro}/{go}/{bo}", file=sys.stderr)

        # gzip once on device, then pull base64 with md5 verification + retry
        send(ser, "gzip -c /tmp/shot.raw > /tmp/shot.gz; md5sum /tmp/shot.gz\r\n")
        mtxt = read_until(ser, "#", 4.0)
        mm = re.search(r"\b([0-9a-f]{32})\b", mtxt)
        want_md5 = mm.group(1) if mm else None
        print(f"device md5(gz): {want_md5}", file=sys.stderr)

        # split the sentinels with "" so the ECHOED command text doesn't
        # contain them verbatim — only the command OUTPUT does.
        db = '__B64_BE""GIN__'
        de = '__B64_EN""D__'
        raw = None
        for attempt in range(1, 5):
            send(ser, f"echo {db}; base64 /tmp/shot.gz; echo {de}\r\n")
            blob = read_until(ser, END, 90.0)
            try:
                body = blob.rsplit(BEGIN, 1)[1].split(END, 1)[0]
            except IndexError:
                print(f"attempt {attempt}: sentinels missing", file=sys.stderr); continue
            b64 = "".join(re.findall(r"[A-Za-z0-9+/=]+", body))
            try:
                gz = base64.b64decode(b64)
            except Exception as e:
                print(f"attempt {attempt}: b64 {e}", file=sys.stderr); continue
            import hashlib
            got = hashlib.md5(gz).hexdigest()
            if want_md5 and got != want_md5:
                print(f"attempt {attempt}: md5 {got} != {want_md5}, retry", file=sys.stderr)
                continue
            raw = gzip.decompress(gz)
            break
        if raw is None:
            print("failed to pull a clean screenshot", file=sys.stderr); sys.exit(1)
        print(f"pulled {len(raw)} raw bytes (verified)", file=sys.stderr)

        img = Image.new("RGB", (w, h))
        px = img.load()
        for y in range(h):
            row = y * stride
            for x in range(w):
                v = int.from_bytes(raw[row + x * 4: row + x * 4 + 4], "little")
                px[x, y] = ((v >> ro) & 0xff, (v >> go) & 0xff, (v >> bo) & 0xff)
        img.save(args.out)
        print(f"saved {args.out}", file=sys.stderr)
    finally:
        ser.close()


if __name__ == "__main__":
    main()
