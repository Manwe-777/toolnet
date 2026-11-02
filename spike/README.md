# toolnet-spike (M1, stack A1: Rust + iroh)

A TUN device whose IPv4 packets travel between peers as QUIC datagrams over
[iroh](https://iroh.computer) 1.4: direct after hole punching, or through
iroh relays. No control plane yet; peers come from the command line.

```
src/main.rs        the whole spike (~250 lines)
test-netns.sh      two peers in network namespaces, unprivileged and offline
```

## Results (2026-11-02, this laptop, namespaces + veth, direct path)

| | debug | release |
|---|---|---|
| ping RTT | ~0.9 ms | ~0.3 ms |
| 1100-byte ping | ok | ok |
| TCP 50 MB | 63 Mbit/s | **640 Mbit/s** |

QUIC reports `max_datagram_size` 1414 on a direct path. The TUN MTU is 1150
to leave headroom for relay paths. Not measured yet.

## Results: laptop <-> homelab (2026-11-02)

Server on the home LAN, laptop first on home Wi-Fi, then on an LTE phone
hotspot (behind both the phone's NAT and the carrier's NAT; public IP in a
different ISP). Both peers were configured **by endpoint id only**.

| | Result |
|---|---|
| Same LAN | `DIRECT` to 192.168.1.2. 14 Mbit/s, the Wi-Fi limit (raw link 18 Mbit/s) |
| Switch to LTE | **Recovered in ~2 s**: the *same connection* migrated to the n0 relay (`use1-1`), no reconnect |
| On LTE | `RELAY` for the whole ~6 min, 0% loss, ~385 ms RTT. **Hole punching never got a direct path** (carrier NAT) |
| Control | **SSH to the homelab through the tunnel** (`ssh 10.213.0.2`) while `server.local` was unreachable |
| Relay throughput | Inconclusive: 5 MB didn't finish in 50 s, and pings dropped during the transfer. Needs a proper test |
| Tailscale on the server | Untouched, ran side by side (10.213.0.0/24 vs 100.64.0.0/10) |

What we learned:

1. **Relay location matters.** n0's relay is in US East, so LTE to
   relay to home costs ~385 ms. A self-hosted relay at home, reachable with a
   port forward, should cut that to the LTE latency itself. That moves
   "self-host the relay" from nice-to-have to required.
2. **Redials depend on n0's address lookup (pkarr).** During the first
   attempt the hotspot lost internet, and every redial failed with "no
   addressing information" until it came back. In toolnet the peer's
   addresses and relay come from tool-db `node` info, cached locally, so a
   redial never depends on a lookup service.
3. **Docker bridges flood the advertised addresses.** The server advertised
   ~30 `172.x`/`192.168.x` bridge addresses, and iroh warned "too many
   addresses". Filter them with `addr_filter` (only physical interfaces, or
   an allowlist).
4. iroh tries IPv6 lookups on a network without IPv6 ("no route to host").
   This is only noise, but worth filtering in logs.

## NAT measurements (tools/natcheck.py, 2026-11-02)

| Network | Public IP | Mapping |
|---|---|---|
| Home (router, laptop and server) | 203.0.113.10, no carrier NAT | **Consistent**, port preserved |
| LTE hotspot (phone NAT + carrier) | 198.51.100.20 | **Consistent**, port not preserved (one stable port per socket) |

Both sides are hole-punching friendly. So the failure to get `DIRECT` on
LTE is **not** the NATs. Hyperswarm's port probing isn't needed for this pair.
The suspect is the test itself: the laptop's endpoint started on home Wi-Fi
and migrated to LTE. Next test: start both sides fresh with the laptop
already on LTE, using `--no-tun` (no privileges needed) and iroh debug logs.

## Run it across real NATs

On each machine:

```bash
cargo build --release
B=target/release/toolnet-spike
sudo setcap cap_net_admin+ep $B     # TUN without running as root
$B keygen node.key                  # prints this node's endpoint id
```

Then, with A's and B's ids:

```bash
# machine A
$B up --key node.key --ip 10.213.0.1 --peer <B_ID>=10.213.0.2
# machine B
$B up --key node.key --ip 10.213.0.2 --peer <A_ID>=10.213.0.1
# either one
ping 10.213.0.2
```

Peers are found by id only. iroh's n0 preset resolves the id to addresses
(pkarr/DNS) and uses n0's public relays. The log prints the selected path
every 5s, `RELAY` or `DIRECT`. Use `RUST_LOG=toolnet_spike=info,iroh=info`
for more detail.

To actually test NAT traversal, put the machines on different networks, e.g.
the laptop on a phone hotspot and the homelab at home.

## Known limits (spike only)

- IPv4 only. No routes beyond the peers' /32s, so no exit node yet.
- Peer list on the command line. tool-db replaces this in M0.
- Address lookup and relays are n0's public services. Self-hosting
  `iroh-relay` and replacing pkarr lookup with tool-db `node` info is the
  plan, see `../docs/daemon-options.md`.
- The lower endpoint id dials, the other side waits.
- One task reads the TUN, with no batching or GSO. Fine for now.
