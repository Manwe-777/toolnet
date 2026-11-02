#!/usr/bin/env python3
"""NAT mapping check: what public ip:port do different STUN servers see
from ONE local UDP socket?

- Same ip:port for every server  -> endpoint-independent ("consistent") mapping:
  hole punching works with plain simultaneous open.
- Port (or IP) differs per server -> endpoint-dependent ("random") mapping:
  plain hole punching fails; needs port prediction / many-socket probing
  (what HyperDHT does), or a relay.

No dependencies. Usage: python3 natcheck.py [local_port]
"""
import os
import socket
import struct
import sys

SERVERS = [
    ("stun.l.google.com", 19302),
    ("stun1.l.google.com", 19302),
    ("stun2.l.google.com", 19302),
    ("stun.cloudflare.com", 3478),
    ("global.stun.twilio.com", 3478),
]
MAGIC = 0x2112A442


def stun_request(sock, addr):
    txid = os.urandom(12)
    sock.sendto(struct.pack("!HHI", 0x0001, 0, MAGIC) + txid, addr)
    while True:
        data, _ = sock.recvfrom(2048)
        if data[8:20] == txid:
            break
    pos = 20
    while pos + 4 <= len(data):
        atype, alen = struct.unpack("!HH", data[pos:pos + 4])
        val = data[pos + 4:pos + 4 + alen]
        if atype in (0x0020, 0x0001) and val[1] == 0x01:  # (XOR-)MAPPED-ADDRESS, IPv4
            port, ip = struct.unpack("!H4s", val[2:8])
            if atype == 0x0020:
                port ^= MAGIC >> 16
                ip = bytes(b ^ m for b, m in zip(ip, struct.pack("!I", MAGIC)))
            return socket.inet_ntoa(ip), port
        pos += 4 + alen + (-alen % 4)
    return None


def main():
    local_port = int(sys.argv[1]) if len(sys.argv) > 1 else 0
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.bind(("0.0.0.0", local_port))
    sock.settimeout(3)
    print(f"local port: {sock.getsockname()[1]}")
    seen = []
    for host, port in SERVERS:
        try:
            addr = (socket.gethostbyname(host), port)
            mapped = stun_request(sock, addr)
        except Exception as e:  # noqa: BLE001
            print(f"  {host:24} error: {e}")
            continue
        print(f"  {host:24} {addr[0]:15} sees us as {mapped[0]}:{mapped[1]}")
        seen.append(mapped)
    if len(seen) < 2:
        print("verdict: not enough answers")
        return
    ips = {ip for ip, _ in seen}
    ports = [p for _, p in seen]
    if len(set(seen)) == 1:
        preserved = " (port preserved)" if ports[0] == sock.getsockname()[1] else ""
        print(f"verdict: CONSISTENT mapping{preserved}. Hole punching friendly.")
    else:
        kind = "IP and port" if len(ips) > 1 else "port"
        step = sorted(ports)
        deltas = [b - a for a, b in zip(step, step[1:])]
        print(f"verdict: RANDOM mapping, {kind} changes per destination. "
              f"Ports {ports}, deltas {deltas}. Plain hole punching fails.")


if __name__ == "__main__":
    main()
