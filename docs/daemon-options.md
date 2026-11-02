# Daemon: language and stack options (path A)

Requirements: Linux, macOS and Windows daemons, Android and iOS cores (inside
native VPN app shells), userspace tunnel, NAT traversal with relay fallback,
and Nostr/tool-db for the control plane. Researched 2026-11-02; check
versions before committing.

## A1: Rust + iroh (recommended spike)

| Piece | Crate |
|---|---|
| Peer connectivity: dial by public key, hole punching, relay fallback | `iroh` (1.0 released mid-2026) |
| TUN device, cross-platform | `tun-rs` |
| Nostr control plane, NIP-44 | `nostr-sdk` (rust-nostr) |

- IP packets travel as QUIC datagrams between iroh endpoints. iroh starts a
  connection via its home relay, then hole punches and drops the relay once a
  direct path works.
- iroh relays are self-hostable (`iroh-relay`). n0 runs public defaults, but we
  can point to our own, so there's no vendor dependency.
- **Tradeoff:** the tunnel is QUIC, not WireGuard. A plain WireGuard client
  can't join, and the encryption is QUIC/TLS with ed25519 keys rather than the
  Noise protocol. The upside is that we write no NAT traversal at all.
- **Least code by far.** Rust works on mobile via UniFFI (Firezone and Mullvad
  ship Rust cores this way).

## A2: Rust + WireGuard + our own ICE (like Firezone)

| Piece | Crate |
|---|---|
| WireGuard protocol | `boringtun` (Cloudflare, BSD-3). Active forks: Mullvad's GotaTun (Dec 2025), DefGuard's |
| NAT traversal | Our own ICE (Firezone wrote theirs, "snownet"), or `str0m`'s ICE |
| TUN + Nostr | same as A1 |

- The real WireGuard protocol, so standard WireGuard peers can join.
- Firezone's open-source Rust code shows the approach, but it's a lot of work.

## A3: Go + wireguard-go + pion/ice (like NetBird)

| Piece | Library |
|---|---|
| WireGuard | `wireguard-go` (userspace) or kernel WireGuard |
| NAT traversal | `pion/ice` (STUN/TURN/ICE) |
| Relay | `coturn` / TURN, or our own |
| Nostr | `go-nostr` |

- NetBird is exactly this architecture with a central Signal server, which
  is the part we'd replace with Nostr. Its client is BSD-3, a good reference.
- Mobile via `gomobile` (the WireGuard Android app uses wireguard-go).
- The most proven ingredients. Go isn't the preferred language, though.

## Node

Not for the data plane: per-packet crypto and TUN access in JS are slow, and
it won't run in iOS/Android VPN extensions. Use it for the **owner/admin app**
(PWA with tool-db, reusing homelab-monitor's patterns).

## Cross-language caveat

tool-db is TypeScript, so a Rust or Go daemon needs a native implementation
of its event format: kind 30078 `d` tags `<topic>:<key>`, namespaces, the
newest-wins rule and group key encryption. All of it is documented in
`tool-db/NOSTR_FIRST.md` and small. Keep a compatibility test that writes
with JS tool-db and reads with the daemon, and the reverse.
