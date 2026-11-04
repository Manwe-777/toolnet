# toolnet

A Tailscale-style private network whose **control plane lives on Nostr**,
through [tool-db](../tool-db). There is no coordination server: devices find
each other, exchange keys, addresses, routes and DNS names through signed,
encrypted tool-db data on public or self-hosted Nostr relays. Packets travel
peer to peer over [iroh](https://iroh.computer) (QUIC, hole punching, relay
fallback).

> Status: **working prototype**, Linux (systemd service) and Android (VpnService
> app, installed from the owner app). Owner-approved membership, key rotation
> on removal, subnet routes, exit nodes, MagicDNS, and hole punching through
> carrier NAT on both ends (phone on LTE -> homelab direct, ~80-110 ms). The
> owner app is a PWA with QR / join-code pairing. Not audited.

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
$B id --qr                                     # QR code for the owner app
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

To run as a service, with the tray icon on a desktop (the owner app's "Add
device" screen shows this command with the npub and network filled in):

```bash
curl -fsSL https://github.com/Manwe-777/toolnet/releases/latest/download/install.sh \
  | sh -s -- --owner npub1<owner> --net home

# or from a checkout (installs toolnet-tray too if it's built next to it)
cargo build --release -p toolnet -p toolnet-tray
sudo target/release/toolnet install --owner npub1<owner> --net home
```

It installs the systemd service (and the tray, on a desktop), creates the
device identity in `/var/lib/toolnet` and prints the QR code to approve it.
Run it again to update; `sudo toolnet uninstall` removes it.
Usage details and the exact data format are in [daemon/README.md](daemon/README.md).

## Repo

| Path | What |
|---|---|
| [android/](android) | Android app: Kotlin VpnService shell around the Rust core (`android/rust`); `build.sh`, `publish.sh` |
| [app/](app) | Owner app (PWA): approve devices by QR scan, remove them (key rotation), routes |
| [daemon/](daemon) | The `toolnet` binary (Rust): tool-db client, crypto, control plane, iroh + TUN engine, DNS, control socket |
| [tray/](tray) | `toolnet-tray`: tray icon for Linux desktops (connect / disconnect, exit node, devices) |
| [tests/](tests) | End-to-end tests: local ToolDB relay, JS compatibility, namespaces with real TUN traffic, subnet routes, exit node |
| [spike/](spike) | The first connectivity spike and its field results (LAN, LTE, relays) |
| [tools/](tools) | `natcheck.py` (NAT mapping probe), `punchtest.py` (raw UDP hole-punch test) |
| [relay/](relay) | Self-hostable iroh relay (Docker), for a machine reachable from the internet |
| [packaging/](packaging) | systemd units (daemon, tray), the `curl \| sh` installer, static builds and releases |
| [docs/](docs) | Design: architecture, data model, flows, decisions, roadmap |

## Docs

| Doc | What's in it |
|---|---|
| [docs/decisions.md](docs/decisions.md) | Decision log and open questions. **Read this first when resuming.** |
| [docs/roadmap.md](docs/roadmap.md) | What's done, what's next |
| [docs/installers.md](docs/installers.md) | Plan: one-line installers for Linux, macOS, Windows |
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
tests/netns-tun.sh           # TUN traffic in namespaces, MagicDNS, subnet route on/off, disconnect/connect
tests/netns-exit.sh          # exit node: internet only through the exit, peer traffic bypasses, switched at runtime
tests/app-compat.sh          # owner app logic (JS) drives Rust devices: approve by QR payload, routes, remove
tests/netns-nat.sh           # two devices behind NAT routers + local relays: hole punching goes direct (needs iroh-relay)
tests/netns-roam.sh          # network switch: reconnect to Nostr at once, republish, keep following
```

All unprivileged (user namespaces) and offline. They need a built `../tool-db`
checkout and Node 22.
