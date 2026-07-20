#!/usr/bin/env python3
"""Push a file to the DL7006 L1 shell over USB ACM and verify it.

Streams gzip(file) as chunked base64 into a device-side `base64 -d | gunzip`
pipeline, terminated by Ctrl-D. Lands on the SD card (persistent) and checks
the sha256 against the host copy.

  ./push.py neofb.gz --sha <expected-sha256-of-decompressed>
  ./push.py neofb.gz                 # sha taken from ./neofb (sibling)

Close picocom/serial-cmd first — one opener at a time.
"""
from __future__ import annotations

import argparse
import base64
import hashlib
import sys
import time

import serial

PORT = "/dev/ttyACM0"
BAUD = 115200
LINE = 200  # base64 chars per line (well under tty canonical buffer)


def send(ser, s: str) -> None:
    ser.write(s.encode())
    ser.flush()


def drain(ser, seconds: float = 0.5) -> str:
    end = time.time() + seconds
    buf = b""
    while time.time() < end:
        c = ser.read(4096)
        if c:
            buf += c
        else:
            time.sleep(0.03)
    return buf.decode(errors="replace")


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("gzfile")
    ap.add_argument("--target", default="/mnt/sd/linux-lab/neofb")
    ap.add_argument("--sha", default=None, help="expected sha256 of decompressed file")
    ap.add_argument("--run", default=None, help="command to run after verify")
    args = ap.parse_args()

    data = open(args.gzfile, "rb").read()
    b64 = base64.b64encode(data).decode()

    if args.sha is None:
        raw = args.gzfile[:-3] if args.gzfile.endswith(".gz") else args.gzfile
        try:
            args.sha = hashlib.sha256(open(raw, "rb").read()).hexdigest()
        except OSError:
            pass

    ser = serial.Serial(PORT, BAUD, timeout=0.3, write_timeout=5)
    try:
        # wake prompt
        send(ser, "\r\n")
        drain(ser, 0.4)

        # ensure SD mounted + target dir
        tgt = args.target
        tdir = tgt.rsplit("/", 1)[0]
        send(ser, "mount -t vfat -o rw /dev/mmcblk1p1 /mnt/sd 2>/dev/null; "
                  f"mkdir -p {tdir}\r\n")
        drain(ser, 0.6)

        # start the receiver pipeline; its stdin is the tty from here on
        print(f"streaming {len(data)} gz bytes ({len(b64)} b64 chars) -> {tgt}", file=sys.stderr)
        send(ser, f"base64 -d | gunzip > {tgt}\r\n")
        time.sleep(0.3)
        drain(ser, 0.2)

        t0 = time.time()
        for i in range(0, len(b64), LINE):
            send(ser, b64[i:i + LINE] + "\n")
            if (i // LINE) % 40 == 0:
                ser.read(8192)  # keep host RX from backing up (echo)
                pct = 100 * i // len(b64)
                print(f"\r  {pct:3d}%", end="", file=sys.stderr, flush=True)
        # EOF to base64
        send(ser, "\x04")
        print(f"\r  100%  ({time.time()-t0:.1f}s)", file=sys.stderr)
        drain(ser, 1.5)

        # verify
        send(ser, f"chmod +x {tgt}; sha256sum {tgt}\r\n")
        out = drain(ser, 2.5)
        got = ""
        for tok in out.split():
            if len(tok) == 64 and all(c in "0123456789abcdef" for c in tok):
                got = tok
                break
        print(f"device sha256: {got or '(none captured)'}", file=sys.stderr)
        if args.sha:
            ok = got == args.sha
            print(f"expected     : {args.sha}", file=sys.stderr)
            print("MATCH" if ok else "*** MISMATCH ***", file=sys.stderr)
            if not ok:
                sys.exit(2)

        if args.run:
            send(ser, args.run + "\r\n")
            print(drain(ser, 2.0), file=sys.stderr)
    finally:
        ser.close()


if __name__ == "__main__":
    main()
