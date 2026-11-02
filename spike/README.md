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
