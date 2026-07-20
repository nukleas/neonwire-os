#!/usr/bin/env python3
"""Send commands to DL7006 L1 shell over USB ACM (/dev/ttyACM0).

Usage:
  ./tools/serial-cmd.py 'uname -a'
  ./tools/serial-cmd.py -i              # crude interactive
  ./tools/serial-cmd.py -f script.sh    # run lines from file

Close picocom/screen first — only one opener at a time.
"""
from __future__ import annotations

import argparse
import sys
import time

try:
    import serial
except ImportError:
    print("need pyserial: pip install pyserial", file=sys.stderr)
    sys.exit(1)

DEFAULT_PORT = "/dev/ttyACM0"
BAUD = 115200


def open_port(port: str) -> serial.Serial:
    return serial.Serial(port, BAUD, timeout=0.3, write_timeout=2)


def drain(ser: serial.Serial, seconds: float = 0.4) -> bytes:
    end = time.time() + seconds
    data = b""
    while time.time() < end:
        chunk = ser.read(4096)
        if chunk:
            data += chunk
        else:
            time.sleep(0.05)
    return data


def run_cmd(ser: serial.Serial, cmd: str, wait: float = 1.2) -> str:
    ser.write(b"\r\n")
    time.sleep(0.1)
    drain(ser, 0.2)
    ser.write((cmd + "\r\n").encode())
    ser.flush()
    time.sleep(wait)
    # keep reading while data arrives
    data = b""
    idle = 0
    while idle < 5:
        chunk = ser.read(8192)
        if chunk:
            data += chunk
            idle = 0
        else:
            idle += 1
            time.sleep(0.1)
    return data.decode(errors="replace")


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("cmd", nargs="?", help="command to run")
    ap.add_argument("-p", "--port", default=DEFAULT_PORT)
    ap.add_argument("-w", "--wait", type=float, default=1.2)
    ap.add_argument("-i", "--interactive", action="store_true")
    ap.add_argument("-f", "--file", type=str, help="run each line as a command")
    args = ap.parse_args()

    try:
        ser = open_port(args.port)
    except Exception as e:
        print(f"cannot open {args.port}: {e}", file=sys.stderr)
        print("Is picocom/screen still attached? Close it first.", file=sys.stderr)
        sys.exit(1)

    try:
        if args.interactive:
            print(f"connected {args.port} — Ctrl+C to exit", file=sys.stderr)
            drain(ser, 0.5)
            ser.write(b"\r\n")
            while True:
                # print any device output
                out = ser.read(4096)
                if out:
                    sys.stdout.buffer.write(out)
                    sys.stdout.buffer.flush()
                # non-blocking-ish stdin
                import select

                r, _, _ = select.select([sys.stdin], [], [], 0.05)
                if r:
                    line = sys.stdin.readline()
                    if not line:
                        break
                    ser.write(line.encode() if line.endswith("\n") else (line + "\n").encode())
                    if not line.endswith("\n"):
                        ser.write(b"\r\n")
        elif args.file:
            for line in open(args.file):
                line = line.strip()
                if not line or line.startswith("#"):
                    continue
                print(f"$ {line}")
                print(run_cmd(ser, line, args.wait))
        elif args.cmd:
            print(run_cmd(ser, args.cmd, args.wait))
        else:
            ap.print_help()
            sys.exit(1)
    finally:
        ser.close()


if __name__ == "__main__":
    main()
