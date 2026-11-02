# Roadmap

## Done (2026-11-02)

- **Connectivity spike** (`spike/`): Rust + iroh + TUN. 640 Mbit/s direct in
  namespaces; laptop on LTE <-> homelab through a relay; network switches
  migrate in ~2 s.
- **M0, control plane on tool-db** (`daemon/`): native ToolDB values over
  Nostr, NIP-44 group encryption compatible with JS tool-db (tested both ways),
  owner create / approve / remove with group key rotation, live netmap, node
  info publishing, no dependency on iroh's address lookup. Works over public
  Nostr relays (laptop + homelab connected ~10 s after approval).
- **Subnet routes**: device advertises, owner approves, peers install routes;
  forwarding + masquerade on the router; anti-spoofing on received packets.
- **Exit nodes**: same approval model; policy routing on the client keeps
  iroh's UDP and the relays out of the tunnel.
- **MagicDNS**: `<member>.<domain>` served on the tailnet IP, split DNS via
  systemd-resolved.
- **systemd unit** with only `CAP_NET_ADMIN` and `CAP_NET_BIND_SERVICE`.

## Next

1. **Hole punching on LTE.** Both NATs measure as consistent, yet iroh never
   got a direct path from the LTE hotspot to home. Run `tools/punchtest.py`
   (see spike/README.md) to tell NAT filtering from an iroh issue.
2. **Real-world run with TUN**: laptop + homelab as services, exit node
   through the homelab from LTE.
3. **Owner app**: a PWA (like homelab-monitor's) holding the owner key, with
   approve / remove / routes. JS tool-db compatibility is already tested.
   Pairing by QR code: the device shows its npub, the owner scans it.
4. **Owner key safety**: NIP-46 remote signer or the PWA, instead of
   `owner.key` on a laptop.
5. **IPv6** inside the tailnet (ULA per network), and IPv6 exit traffic.
6. **Exit node DNS**: send all DNS through the exit node while it's in use
   (today only the tailnet domain is split; other queries use local DNS).
7. **ACLs** (who may reach whom), evaluated on each device from the
   owner-signed policy.
8. **Relay**: a toolnet-owned TLS CA for relays reached by IP (decisions.md),
   and relays listed in the network config.
9. **Mobile**: Android VpnService / iOS NetworkExtension shells around the Rust
   core.
10. **Performance**: batch TUN reads, GSO, multiple queues. Today: ~800 Mbit/s
    TCP through the full daemon in namespaces (release build, one core).

## Known issues

- iroh advertises every local interface for hole punching, including Docker
  bridges, and caps the list at 32. On the homelab the last bridge address is
  dropped. Harmless, but iroh has no option to filter them yet.
- Some networks' DNS blocks `dns.iroh.link`. toolnet doesn't use iroh's
  address lookup, so this only matters for the spike.
- Docker's iptables `FORWARD` policy can drop routed traffic on a Docker host;
  toolnet's nft accept rule doesn't override another table's drop. Needs a
  `DOCKER-USER` rule on such hosts.
