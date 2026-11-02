# Roadmap

## M0: control plane proof of concept (Linux, no NAT traversal)

Goal: show that tool-db can replace the coordination server.

- Node CLI `toolnet` using tool-db: `init` (owner), `up`, `approve`, `remove`, `status`.
- Kernel WireGuard via `wg`/`ip` commands. Peers must be reachable directly
  (same LAN, or public IP / port forward).
- Data model from [data-model.md](data-model.md): owner-assigned IPs, group
  key, rotation on removal.
- Static `/etc/hosts`-style names (no resolver yet).
- **Done when** 3 machines join, ping each other by tailnet IP, one gets removed
  and loses access, and an endpoint change propagates.

## M1: connectivity spike (A1: Rust + iroh)

- Rust daemon: iroh endpoint per device, TUN via tun-rs, IP packets as QUIC datagrams.
  Control plane via nostr-sdk, compatible with JS tool-db (round trip tests).
- `node` info carries the iroh endpoint id and relay URL instead of raw endpoints.
- **Done when** two machines behind different NATs connect, and an exit node works.
- Status 2026-11-02: `spike/` works across network namespaces on the direct path
  (640 Mbit/s release). Pending: real NAT/relay test on two machines, and exit node.

## M2: daily driver

- Exit node and subnet router approval in the owner flow.
- DNS: MagicDNS names, upstream nameservers, split DNS.
- Relay choice settled (own `derper` or public).
- Owner admin UI (PWA, like homelab-monitor's app).

## M3: mobile

- Depends on open question 3 in [decisions.md](decisions.md).
