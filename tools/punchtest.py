#!/usr/bin/env python3
"""Raw UDP reachability test between a home server and a roaming laptop,
no iroh involved. Tells which NAT filtering the server side has.

server (behind a port-preserving NAT, so its public port == local port):
    punchtest.py server PORT REMOTE_IP
  Echoes every packet it receives, tagged with the current phase:
    phase A (0-240 s): only a STUN keepalive, nothing sent toward REMOTE_IP.
        Echoes reaching the laptop => endpoint-independent filtering (full cone).
    phase B (240-480 s): also sends to REMOTE_IP:9 every 0.5 s.
        Echoes starting only now  => address-dependent filtering.
    exits at 600 s.
  No echo in either phase => address+port dependent filtering: only true
  simultaneous hole punching (knowing the laptop's exact port) can work.

laptop:
    punchtest.py laptop SERVER_IP:PORT [SECONDS]
  Sends a probe every 0.5 s and prints every echo (deduplicated by phase).
"""
import os
import socket
import struct
import sys
import time

MAGIC = 0x2112A442
STUN = ("stun.l.google.com", 19302)


def stun_packet():
    return struct.pack("!HHI", 0x0001, 0, MAGIC) + os.urandom(12)


def server(port, remote_ip):
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.bind(("0.0.0.0", port))
    sock.settimeout(0.5)
    stun = (socket.gethostbyname(STUN[0]), STUN[1])
    start = time.time()
    last_keep = 0.0
    print(f"server on :{port}, remote {remote_ip}", flush=True)
    seen = set()
    while (t := time.time() - start) < 600:
        phase = "A" if t < 240 else "B"
        if time.time() - last_keep > 0.5:
            sock.sendto(stun_packet(), stun)  # keeps the NAT mapping alive
            if phase == "B":
                sock.sendto(b"OPEN", (remote_ip, 9))
            last_keep = time.time()
        try:
            data, src = sock.recvfrom(2048)
        except socket.timeout:
            continue
        if not data.startswith(b"PROBE"):
            continue
        sock.sendto(b"ECHO " + phase.encode() + b" " + data, src)
        key = (phase, src)
        if key not in seen:
            seen.add(key)
            print(f"{t:6.1f}s phase {phase}: probe from {src[0]}:{src[1]}", flush=True)
    print("server done", flush=True)


def laptop(target, seconds):
    host, port = target.rsplit(":", 1)
    dst = (host, int(port))
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.bind(("0.0.0.0", 0))
    sock.settimeout(0.5)
    sock.sendto(stun_packet(), (socket.gethostbyname(STUN[0]), STUN[1]))
    start = time.time()
    n = 0
    got = {}
    print(f"laptop local port {sock.getsockname()[1]} -> {dst[0]}:{dst[1]}", flush=True)
    last = 0.0
    while (t := time.time() - start) < seconds:
        if time.time() - last > 0.5:
            n += 1
            sock.sendto(f"PROBE {n}".encode(), dst)
            last = time.time()
        try:
            data, src = sock.recvfrom(2048)
        except socket.timeout:
            continue
        if data.startswith(b"ECHO"):
            phase = data.split()[1].decode()
            got[phase] = got.get(phase, 0) + 1
            if got[phase] in (1, 20, 100):
                print(f"{t:6.1f}s ECHO in phase {phase} (x{got[phase]}) from {src[0]}:{src[1]}", flush=True)
    print(f"laptop done: {n} probes sent, echoes by phase: {got or 'none'}", flush=True)


if __name__ == "__main__":
    if sys.argv[1] == "server":
        server(int(sys.argv[2]), sys.argv[3])
    else:
        laptop(sys.argv[2], int(sys.argv[3]) if len(sys.argv) > 3 else 540)
