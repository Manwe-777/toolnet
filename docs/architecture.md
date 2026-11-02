# Architecture

## Roles and keys

| Role | Key | Lives where |
|---|---|---|
| **Owner** (`O`) | Nostr secp256k1 | The admin's phone/app, NIP-07 or NIP-46 signer. Defines the tailnet. |
| **Device** (`D`) | Nostr secp256k1 | Generated on each device. Signs that device's node info. |
| **WireGuard key** (`W`) | Curve25519 | Generated on each device and published in its node info. Can rotate without the owner. |
| **Group key** (`G`) | Nostr secret key | Created by the owner and wrapped for each member (tool-db `createGroupKey` / `wrapGroupKey`). Encrypts all shared state. |

The tailnet *is* the owner pubkey. One owner can run several tailnets by
using different tailnet ids in the keys (see data model).

## Trust model

- **Relays are untrusted.** They see pubkeys, kinds, `d` tags, timing and
  ciphertext. They can withhold events but can't forge them.
- **The owner is the only authority.** A device is a member if and only if the
  latest owner-signed `net` lists its `D`. A peer's WireGuard key is trusted
  if and only if it's in the node info signed by that listed `D`. No policy
  flags, just the identity chain.
- **Removed devices** lose access when peers drop them from WireGuard, and once
  the group key rotates they also stop seeing endpoints.

## Components (on each device)

```
 ┌───────────────────────────── toolnetd ─────────────────────────────┐
 │ control:  tool-db client: read net + nodes, publish own node,      │
 │           signaling events                                         │
 │ netmap:   build the peer list (members ∩ valid node infos ∩ ACL)   │
 │ engine:   WireGuard + NAT traversal + relay fallback               │
 │ os:       TUN/wg device, routes, policy routing (exit node),       │
 │           forwarding and masquerade (when exit), DNS               │
 └────────────────────────────────────────────────────────────────────┘
```

## Two implementation paths

### Path A: our own client

Go (or Rust) daemon using `wireguard-go` (or `boringtun`), with its own STUN,
hole punching and relay fallback. The control part either talks Nostr
natively (Go Nostr libraries exist, and tool-db's format is documented in
`NOSTR_FIRST.md`) or uses a small Node sidecar running tool-db.

- **Pros:** full control, no dependency on Tailscale's protocol, signaling can be
  pure Nostr.
- **Cons:** NAT traversal and a relay are months of work, and we'd need our
  own mobile apps.

### Path B: Tailscale client + local control shim

Run the official `tailscaled` with `--login-server=http://127.0.0.1:PORT`. The
port belongs to a small **per-device shim** that implements the control API
(as Headscale does), but builds netmaps from tool-db instead of a database:

1. `tailscaled` registers with the shim and sends its node key, disco key,
   endpoints (it does its own STUN) and Hostinfo.
2. The shim publishes those into `:D.node-O` (encrypted with `G`).
3. The shim subscribes to `:O.net` and all `:P.node-O`, and streams
   MapResponses (peers, routes, DNS, DERP map) back to `tailscaled`.
4. `tailscaled` handles WireGuard, NAT traversal, DERP, exit nodes, subnet
   routes and MagicDNS.

- **Pros:** almost everything hard is already done and battle-tested. The
  desktop CLI and maybe the mobile apps work as is.
- **Cons:** Tailscale's control protocol (ts2021, Noise based) is semi-documented
  and changes, and we inherit Headscale's maintenance problem. Mobile apps
  can't run a localhost shim next to them, so a phone would point at a shim
  hosted elsewhere (e.g. its owner's homelab), which brings back a
  per-user server.

**Decision (2026-11-02): path A.** toolnet must not depend on Tailscale.
Stack options are in [daemon-options.md](daemon-options.md).

### Peer roles in path A

There's no server-like node. Every peer runs the same daemon and offers
whatever it can:

| Capability | Who | What it does |
|---|---|---|
| peer | everyone | Joins the mesh, hole punches to the others |
| reachable / relay | peers with a public IP or port forward (homelab, VPS) | Answers STUN, relays ciphertext for peers that can't connect directly |
| exit node / subnet router | peers the owner approves for it | Forwards traffic to the internet or a LAN |
| owner | a key (phone/PWA) | Signs membership, IPs, routes and DNS. Only needs to be online to make changes. |

Relays advertise themselves in their `node` info, and the owner can pin
preferred relays in `net`. This is the Hyperswarm/HyperDHT model.

## The hard parts (independent of tool-db)

1. **NAT traversal:** kernel WireGuard owns its UDP socket, so we can't STUN
   from it. That needs a userspace engine (path A) or `tailscaled` (path B).
2. **Relay fallback:** some paths can't be punched. We need `derper` or our own
   relay.
3. **Exit node bootstrap:** traffic to the Nostr relays (and to DERP) must
   bypass the exit tunnel. Otherwise a dead exit node also cuts the
   control plane we'd use to recover.
4. **Stale reads:** a relay can withhold the newest `net`. Read from several
   relays and keep the newest valid version (tool-db already does).
5. **Mobile:** Android VpnService and iOS NetworkExtension apps (path A), or the
   remote shim problem (path B).
6. **Metadata:** relays learn which pubkeys belong to the same tailnet and
   when they update. Consider a private relay for the tailnet, or per-tailnet
   throwaway device keys.
