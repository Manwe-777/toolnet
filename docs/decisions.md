# Decisions and open questions

Log decisions here with a date. Update when resuming work.

## Settled

| Date | Decision | Why |
|---|---|---|
| 2026-11-02 | tool-db is the **control plane only**. Packets go over WireGuard. | Nostr relays are WebSocket + JSON, rate limited and high latency. They're fine for keys and endpoints, not for IP traffic. |
| 2026-11-02 | **Owner key = the tailnet's authority.** Membership is an owner-signed list, and peers trust a WireGuard key only via a listed device key. | Identity chain instead of policy flags, and it reuses tool-db user namespaces. |
| 2026-11-02 | **All shared state is encrypted with a group key.** It rotates on removal. | Endpoints reveal home IPs, and relays must only see ciphertext. |
| 2026-11-02 | **The owner assigns tailnet IPs.** | Single writer, so no collisions. Hash-derived IPs would need collision handling. |
| 2026-11-02 | **Path A: fully independent of Tailscale.** Path B is dropped. | The project is a backup plan in case Tailscale gets expensive or goes away, so no dependency on its client or control protocol. |
| 2026-11-02 | **All peers are equal, and roles are capabilities.** "Reachable" peers (public IP or port forward) can serve STUN and relay. Others, such as phones, are plain peers. The owner is a key, not a server, and only needs to be online to change membership. | Same model as Hyperswarm/HyperDHT. There's no server-like node, which also settles the old mobile question (phones are just peers). |
| 2026-11-02 | **Don't use 100.64.0.0/10 by default.** Default tailnet range is configurable, 10.213.0.0/16 in docs and spike. | toolnet is a Tailscale backup, so it must run next to Tailscale without route conflicts (the homelab runs Tailscale). |
| 2026-11-02 | **Self-host the relay** (homelab, port forwarded) and drop pkarr lookup, using tool-db `node` info for addresses. | LTE spike: the n0 US relay added ~385 ms RTT, carrier NAT blocked hole punching so the relay carries all traffic, and redials failed while n0's lookup was unreachable. |
| 2026-11-02 | **Relays need no domain, DNS or public CA.** For now: plain HTTP by IP (`http://IP:3340`). Daemon: TLS with a tailnet-owned CA (iroh `ca_tls_config`), the relay reissuing its cert for its current IP. The relay URL is published in tool-db `node` info, so IP changes propagate. | A Let's Encrypt cert needs a domain, so a DNS record, so a fixed IP or DDNS. That's an external requirement toolnet shouldn't have. Peer traffic is E2E encrypted anyway. |
| 2026-11-02 | **Spike A1 started** in `spike/`: iroh 1.4 + tun-rs works, 640 Mbit/s direct in netns. | Validates Rust + iroh as viable; real NAT test pending. |

## Proposed, not decided

- **Daemon stack:** spike A1 (Rust + iroh + tun-rs + nostr-sdk). Fall back to
  A2 or A3 if QUIC instead of WireGuard turns out to be a problem. See
  [daemon-options.md](daemon-options.md).
- **Signaling kind 21080:** check against tool-db's kind registry and NIP ranges.
  May be unnecessary with iroh, which coordinates hole punching through its relays.

## Open questions

1. **Where does the owner key live?** On the phone (like homelab-monitor), with
   a CLI using NIP-46 to approve from the terminal? Or does the homelab hold it?
2. **Relay when no peer is reachable:** if every peer is behind NAT, someone
   has to run a relay (homelab port forward, a cheap VPS, or a public iroh
   relay). Which default should we ship?
3. ~~Path B on mobile~~: resolved by path A, where phones are plain peers. They
   still need native VPN app shells (Android VpnService, iOS NetworkExtension).
4. ~~Tailscale control protocol stability~~: dropped together with path B.
5. **Is QUIC instead of WireGuard acceptable?** This decides A1 vs A2/A3. See
   [daemon-options.md](daemon-options.md).
6. **Relays:** public relays only, or a ToolDbRelay per tailnet (durability,
   fewer metadata leaks, and it could host `derper` alongside)?
7. **Multiple tailnets / sharing a node** with another owner (Tailscale's
   "node sharing")?
8. **Key expiry:** do we want it? The owner could set `expiresAt` per member.
