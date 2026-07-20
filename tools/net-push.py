#!/usr/bin/env python3
"""Push a file to the DL7006 over the wifi telnet shell (busybox telnetd on :23).

The serial ACM is slow and wedges under load; once wifi is up this is the fast,
reliable transfer path. Streams gzip(file) as base64 into a device-side
`base64 -d | gunzip > target`, then verifies sha256.

  ./tools/net-push.py local.bin /mnt/sd/linux-lab/target [--host 192.168.4.32]

Also usable as a one-shot command runner:
  ./tools/net-push.py --cmd 'uptime'
"""
import argparse, base64, gzip, hashlib, socket, sys, time

def connect(host, port=23, timeout=8):
    s = socket.create_connection((host, port), timeout=timeout)
    s.settimeout(4)
    time.sleep(0.8)
    first = _drain(s)
    # answer telnet IAC negotiation: refuse every option (WONT/DONT)
    r = b""; i = 0
    while i < len(first):
        if first[i] == 255 and i + 2 < len(first):
            cmd, opt = first[i+1], first[i+2]
            if cmd == 253:   r += bytes([255, 252, opt])   # DO   -> WONT
            elif cmd == 251: r += bytes([255, 254, opt])   # WILL -> DONT
            i += 3
        else:
            i += 1
    if r: s.sendall(r)
    time.sleep(0.5); _drain(s)
    return s

def _drain(s, quiet=0.6):
    out = b""; last = time.time()
    while time.time() - last < quiet:
        try:
            b = s.recv(8192)
            if not b: break
            out += b; last = time.time()
        except socket.timeout:
            break
    return out

def run(s, cmd, wait=2.0):
    s.sendall(cmd.encode() + b"\n")
    time.sleep(wait)
    return _drain(s).decode(errors="replace")

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("src", nargs="?")
    ap.add_argument("dst", nargs="?")
    ap.add_argument("--host", default="192.168.4.32")
    ap.add_argument("--cmd")
    ap.add_argument("--run")
    args = ap.parse_args()

    s = connect(args.host)
    try:
        if args.cmd:
            print(run(s, args.cmd, 3)); return
        data = open(args.src, "rb").read()
        sha = hashlib.sha256(data).hexdigest()
        gz = gzip.compress(data, 9)
        b64 = base64.b64encode(gz).decode()

        tdir = args.dst.rsplit("/", 1)[0]
        run(s, f"mkdir -p {tdir}", 0.6)
        s.sendall(f"base64 -d | gunzip > {args.dst}\n".encode())
        time.sleep(0.3); _drain(s, 0.3)
        # stream in chunks; TCP is reliable so no per-line ACK needed
        LINE = 400
        t0 = time.time()
        for i in range(0, len(b64), LINE):
            s.sendall(b64[i:i+LINE].encode() + b"\n")
            if (i // LINE) % 50 == 0:
                _drain(s, 0.05)
                sys.stderr.write(f"\r  {100*i//len(b64):3d}%"); sys.stderr.flush()
        s.sendall(b"\x04")   # EOF to base64
        sys.stderr.write(f"\r  100%  ({time.time()-t0:.1f}s)\n")
        time.sleep(1.0); _drain(s, 1.0)

        out = run(s, f"chmod +x {args.dst}; sha256sum {args.dst}", 2.5)
        got = next((t for t in out.split() if len(t) == 64 and all(c in "0123456789abcdef" for c in t)), "")
        print(f"device sha256: {got or '(none)'}", file=sys.stderr)
        print(f"expected     : {sha}", file=sys.stderr)
        print("MATCH" if got == sha else "*** MISMATCH ***", file=sys.stderr)
        if got != sha: sys.exit(2)
        if args.run:
            print(run(s, args.run, 2.5))
    finally:
        s.close()

if __name__ == "__main__":
    main()
