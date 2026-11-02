# toolnet

A Tailscale-style private network whose **control plane lives on Nostr**,
through [tool-db](../tool-db). There is no coordination server: devices find
each other, exchange keys, addresses, routes and DNS names through signed,
encrypted tool-db data on public or self-hosted Nostr relays. Packets travel
peer to peer over [iroh](https://iroh.computer) (QUIC, hole punching, relay
fallback).

> Status: **working prototype** (Linux). Owner-approved membership, key
> rotation on removal, TUN data plane, subnet routes, exit nodes and MagicDNS
> all work and are covered by tests. Not audited; don't rely on it yet.

```
            control plane: tool-db on Nostr relays (relays only see ciphertext)
   ┌────────────────────────────────────────────────────────────────────┐
   │  :OWNER.net-N      members, IPs, routes, DNS   (signed by owner)   │
   │  :OWNER.gk-N-DEV   group key, wrapped per device                   │
   │  :DEV.node-OWNER-N endpoint id, addresses, relay (signed by device)│
   └───────────▲──────────────────────▲───────────────────────▲─────────┘
               │                      │                       │
         ┌─────┴─────┐  QUIC/iroh ┌───┴───────┐  QUIC/iroh ┌──┴────────┐
         │  laptop   │◄══════════►│  homelab  │◄══════════►│   phone   │
         └───────────┘            └───────────┘            └───────────┘
              direct after hole punching, or through an iroh relay
```

## Try it

```bash
cargo build --release -p toolnet
B=target/release/toolnet

$B owner create --net home                    # once, on the owner's machine
$B init --owner npub1<owner> --net home        # on each device
$B owner approve npub1<device> --name laptop   # owner approves it
sudo setcap cap_net_admin,cap_net_bind_service+ep $B
$B up                                          # on each device
ping server.home.toolnet
```

Routes and exit nodes, both opt in on the device *and* approved by the owner:

```bash
$B up --advertise-routes 192.168.1.0/24        # on the homelab
$B owner routes homelab 192.168.1.0/24         # owner allows it

$B up --advertise-exit-node                    # on the homelab
$B owner routes homelab 0.0.0.0/0              # owner allows it
$B up --exit-node homelab                      # on the laptop
```

To run as a service, see [packaging/toolnet.service](packaging/toolnet.service).
Usage details and the exact data format are in [daemon/README.md](daemon/README.md).

## Repo

| Path | What |
|---|---|
| [daemon/](daemon) | The `toolnet` binary (Rust): tool-db client, crypto, control plane, iroh + TUN engine, DNS |
| [tests/](tests) | End-to-end tests: local ToolDB relay, JS compatibility, namespaces with real TUN traffic, subnet routes, exit node |
| [spike/](spike) | The first connectivity spike and its field results (LAN, LTE, relays) |
| [tools/](tools) | `natcheck.py` (NAT mapping probe), `punchtest.py` (raw UDP hole-punch test) |
| [relay/](relay) | Self-hostable iroh relay (Docker), for a machine reachable from the internet |
| [packaging/](packaging) | systemd unit |
| [docs/](docs) | Design: architecture, data model, flows, decisions, roadmap |

## Docs

| Doc | What's in it |
|---|---|
| [docs/decisions.md](docs/decisions.md) | Decision log and open questions. **Read this first when resuming.** |
| [docs/roadmap.md](docs/roadmap.md) | What's done, what's next |
| [docs/architecture.md](docs/architecture.md) | Components, peer roles, the hard parts |
| [docs/data-model.md](docs/data-model.md) | Original data model design (the implemented format is in daemon/README.md) |
| [docs/flows.md](docs/flows.md) | Join, roaming, removal, exit node, DNS |
| [docs/daemon-options.md](docs/daemon-options.md) | Why Rust + iroh |
| [docs/tailscale-primer.md](docs/tailscale-primer.md) | How Tailscale works, mapped to toolnet |

## Tests

```bash
cargo test -p toolnet        # unit: crypto, membership/rotation, CIDR, DNS
tests/e2e-local.sh           # owner + 2 devices on a local ToolDB relay: join, connect, remove
tests/compat.sh              # JS tool-db reads Rust's data; Rust accepts a JS approval
tests/netns-tun.sh           # TUN traffic in namespaces, MagicDNS, subnet route on/off
tests/netns-exit.sh          # exit node: internet only through the exit, peer traffic bypasses
```

All unprivileged (user namespaces) and offline. They need a built `../tool-db`
checkout and Node 22.
