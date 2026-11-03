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

| 2026-11-02 | **Daemon stack: Rust + iroh + tun-rs + rust-nostr**, one binary (`daemon/`). | The spike worked; rust-nostr covers NIP-44 and relays. One language for daemon and future mobile cores. |
| 2026-11-02 | **No iroh address lookup** (pkarr/DNS). Peer addresses and relays come only from tool-db node info. | The LTE hotspot's DNS blocks `dns.iroh.link`; the control plane is the point of the project. |
| 2026-11-02 | **Implemented data format** (daemon/README.md): `:O.net-N`, `:O.gk-N-D` (also for O itself), `:D.node-O-N`, envelopes `{v, epoch, data/key}`. | The owner key alone recovers the network. Epoch in plaintext lets devices pick the right group key. |
| 2026-11-02 | **Writes are always newer than anything seen for that key.** | Two approvals in the same second raced: tool-db ties go to the lowest event id, so one approval was lost. Same rule as JS tool-db's `putData`. |
| 2026-11-02 | **Routes need both sides**: the owner allows a prefix (`owner routes`) AND the device advertises it. Exit node = route 0.0.0.0/0, used only by clients that pick it (`--exit-node`). | Identity-based policy, no implicit trust. Matches Tailscale's model. |
| 2026-11-02 | **Exit routing without socket marks**: `ip rule` on iroh's UDP source port, bypass rules for relay and Nostr relay IPs, `suppress_prefixlength 0` for local routes. | iroh has no SO_MARK option. Tested in namespaces: peer traffic stays outside the tunnel. |
| 2026-11-02 | **MagicDNS as split DNS only** (`~<domain>` via systemd-resolved), domain default `<net>.toolnet`. | No DNS takeover, no leaks of tailnet names. Full DNS through an exit node is later work. |
| 2026-11-02 | **No port forwards on the home router.** | Manuh's call: the homelab doesn't accept inbound connections. Direct paths must come from hole punching; otherwise n0's relays (or a future VPS). |

## Proposed, not decided

- **Signaling kind 21080:** check against tool-db's kind registry and NIP ranges.
  May be unnecessary with iroh, which coordinates hole punching through its relays.

## Open questions

1. **Where does the owner key live?** On the phone (like homelab-monitor), with
   a CLI using NIP-46 to approve from the terminal? Or does the homelab hold it?
2. **Relay when no peer is reachable:** with no port forwards at home, n0's
   public iroh relays are the default today. A cheap VPS running `relay/` is
   the self-hosted option. Which default should we ship?
3. ~~Path B on mobile~~: resolved by path A, where phones are plain peers. They
   still need native VPN app shells (Android VpnService, iOS NetworkExtension).
4. ~~Tailscale control protocol stability~~: dropped together with path B.
5. ~~Is QUIC instead of WireGuard acceptable?~~ Built on iroh (QUIC); revisit
   only if WireGuard interop becomes a requirement.
9. ~~Homelab `toolnet-relay` container: keep or remove?~~ Removed (2026-11-03): hole punching works, n0's relays cover the fallback. `relay/` stays for anyone who needs one.
6. **Relays:** public relays only, or a ToolDbRelay per tailnet (durability,
   fewer metadata leaks, and it could host `derper` alongside)?
7. **Multiple tailnets / sharing a node** with another owner (Tailscale's
   "node sharing")?
8. **Key expiry:** do we want it? The owner could set `expiresAt` per member.
