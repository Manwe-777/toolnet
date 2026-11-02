# How Tailscale works (what we need to know)

Source: https://tailscale.com/blog/how-tailscale-works, plus the open-source
client (`tailscale/tailscale`) and Headscale, the open-source control server.

## Two planes

**Control plane (coordination server):**
- Authenticates nodes (SSO in Tailscale, keys for us).
- Each node registers a *node key* (WireGuard Curve25519 pubkey), a *disco
  key* (used for path discovery messages), its endpoints and the routes it
  advertises.
- The server pushes each node a **netmap**: every peer it may talk to, with
  peer keys, tailnet IPs, endpoints, allowed routes, DNS config and the DERP
  map. Updates stream live.
- It never sees traffic, and private keys never leave the nodes.

**Data plane:**
- WireGuard tunnels, peer to peer (a mesh, not hub and spoke).
- Tailscale runs **userspace** `wireguard-go` behind its own UDP layer
  ("magicsock"). That layer can multiplex STUN, disco pings and WireGuard
  over the same socket, which NAT traversal needs.

## NAT traversal

1. STUN against DERP servers to learn the public `ip:port` mapping.
2. Exchange candidate endpoints (LAN, public, IPv6) via the control plane.
3. "Disco" pings from both sides at the same time to open NAT mappings
   (hole punching). The *call-me-maybe* message asks a peer to start pinging
   now, and it is sent through DERP.
4. Pick the best path that works. Keep trying to upgrade from relayed to direct.

## DERP

Encrypted packet relays over HTTPS, keyed by node public key. They are used:
- before a direct path exists (the first packets always flow),
- when a direct path is impossible (symmetric NAT, CGNAT, strict firewalls),
- for the call-me-maybe signaling.

DERP sees only WireGuard ciphertext. `derper` is open source and self-hostable.

## Features we want

| Feature | Mechanism |
|---|---|
| Exit node | A node advertises `0.0.0.0/0` and `::/0`, and the admin approves it. A client that picks it routes its default route into the tunnel. The exit node forwards and masquerades. |
| Subnet router | A node advertises e.g. `192.168.1.0/24`. Same approval, so peers reach the LAN behind it. |
| MagicDNS | `100.100.100.100` resolver on each node answers `host.tailnet-name`, and forwards the rest to the configured upstream servers. |
| ACLs | Policy evaluated by the control server, which drops disallowed peers from each netmap and sends packet filters. |
| Key expiry | Node keys expire unless disabled, which forces re-auth. |

## Mapping to toolnet

| Tailscale | toolnet |
|---|---|
| Coordination server | tool-db data on Nostr relays (owner-signed config, device-signed node info) |
| SSO login | Owner approves a device's npub (QR, like homelab-monitor pairing) |
| Netmap stream | tool-db subscriptions to `:O.net` and every `:D.node-O` |
| call-me-maybe via DERP | Ephemeral Nostr events, NIP-44 encrypted to the peer (or keep DERP, see path B) |
| DERP | Self-hosted `derper`, or Tailscale's public DERP map (path B), see decisions |
| ACL evaluation on the server | Each node evaluates the owner-signed policy locally |
