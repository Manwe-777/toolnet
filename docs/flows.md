# Flows (draft)

## Create a tailnet

1. The owner app generates `G` (epoch 1) and writes `:O.net-N` with no members.
2. The owner app shows its npub and tailnet id as a QR code / join link.

## Join a device

Out-of-band approval, like homelab-monitor pairing in reverse:

1. Run `toolnet up --owner npub1... --net home` on the device. It generates `D`
   and `W` (stored locally, NIP-49 encrypted if a password is set) and shows
   `D` as a QR code.
2. The owner scans it (or runs `toolnet approve npub1...`) and assigns a name
   and IPs, then:
   - writes `:O.gk-N-D` (`G` wrapped for `D`),
   - adds `D` to `members` and rewrites `:O.net-N`.
3. The device sees `gk`, unwraps `G`, decrypts `net` and publishes `:D.node-O-N`.
4. Every peer gets the update through its subscriptions and adds the WireGuard peer.

## Endpoint changes (roaming)

Devices republish `node` when their endpoints change (network change, new
STUN result). Rate-limit this to around 1 per 10s, which public relays will
tolerate. Peers update the WireGuard endpoint in place.

## Connecting (path A)

1. Each side tries every endpoint of the other directly.
2. If there's no handshake within ~2s, send `call-me-maybe` (kind 21080) and
   both sides ping each other's candidates at the same time.
3. Still nothing: fall back to the relay. Keep probing for a direct path in the
   background.

In path B, `tailscaled` does all of this using disco keys and DERP. We only
deliver endpoints and disco keys through `node`.

## Remove a device

1. The owner removes `D` from `members`, bumps `epoch`, creates `G'`, wraps it
   for every remaining member and rewrites `net` encrypted with `G'`.
2. The remaining devices pick up `G'`, re-encrypt their `node` with it and drop
   `D` from WireGuard.
3. `D` can still read old (`G`) ciphertext, meaning stale endpoints, but nothing
   new. It can still *try* to reach peers' old endpoints, but they no longer
   accept its WireGuard key.
4. Optional: NIP-09 delete `:O.gk-N-D` (best effort).

**Stale read risk:** a peer that only reaches a relay withholding the new `net`
keeps `D` until it sees the update. Mitigations: several relays, and maybe a
max age on `net` that the owner app refreshes periodically.

## Exit node

1. A node runs `toolnet up --advertise-exit-node`, which adds `0.0.0.0/0, ::/0`
   to its `advertisedRoutes`, enables `net.ipv4.ip_forward` and sets up masquerade.
2. The owner approves it, which adds the routes to `approvedRoutes`.
3. A client runs `toolnet set --exit-node homelab`:
   - AllowedIPs for that peer get `0.0.0.0/0, ::/0`,
   - policy routing as in `wg-quick`: fwmark on WireGuard's own packets, a
     separate table with the default route via the tunnel,
   - **bypass routes** for the Nostr relays' and relay server's IPs, so the
     control plane survives the exit node going down,
   - an optional "allow LAN access" setting keeps local subnets outside the tunnel.

## DNS (MagicDNS-like)

- A local resolver on `100.100.100.100` (or a link-local address) answers
  `<member name>.<domain>` from `net.members`, forwards `splitDns`
  domains to their servers, and forwards the rest to `nameservers`.
- On Linux, register it per link with systemd-resolved (`resolvectl dns`,
  `resolvectl domain toolnet0 ~home.toolnet`).
- With an exit node, the exit node's upstream DNS is used, to avoid leaks.
