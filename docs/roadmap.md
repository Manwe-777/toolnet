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
- **Hole punching, LTE hotspot -> home: not possible on these networks.**
  Raw UDP tests (`tools/punchtest.py`), no iroh: home filters by exact address
  and port, and even a simultaneous punch to both sides' exact STUN-reported
  addresses gets nothing through in either direction. LTE traffic to home
  crosses an international backbone (~190 ms one way). So the relay is the
  only path here, and iroh isn't at fault.
- **Exit node from LTE** through the homelab, over an n0 relay: the internet
  sees the home IP. Slow (~1.7 Mbit/s, 410 ms) because of the relay.
- **First real run with TUN** (laptop + homelab, home Wi-Fi, public Nostr
  relays): tailnet ping both ways (direct, 2-3 ms), MagicDNS, 44 Mbit/s TCP
  over Wi-Fi (58 plain), the home LAN route accepted while at home without
  breaking the LAN, and the homelab as exit node (20 MB download through it).
  Bugs it found, all fixed and covered by tests: subnet routes replaced the
  local LAN route; helpers (`ip`, `nft`) didn't inherit file capabilities and
  `nft` isn't on a Debian user's PATH; `resolvectl` blocked the main loop on a
  polkit prompt; Docker's iptables FORWARD DROP broke the exit node.

## Next

1. **A relay close to home.** From the LTE hotspot, home is only reachable
   through a relay (see the hole punching result below), and n0's public relays
   are in the US and rate limited: ~410 ms and ~1.7 Mbit/s through the exit
   node. Options, none needing a port forward or DNS record: a small VPS relay
   in South America reached by IP, or a relay on any device with a public IP.
   Relays listed in the network config (item 8) go with it.
2. **systemd service on both machines**, so MagicDNS registers with resolved
   and nothing needs `setcap` by hand.
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
- Run by hand with `setcap`, systemd-resolved refuses the MagicDNS
  registration (it wants polkit authorization, not just `CAP_NET_ADMIN`):
  names resolve with `dig @<tailnet ip>`; the systemd service (root) is fine.
- MagicDNS can't bind port 53 on a host that already runs a DNS server on
  `0.0.0.0:53` (the homelab). That host's own lookups of tailnet names fail;
  other devices are unaffected.
- After a network switch, the Nostr websockets take ~75 s to time out and
  reconnect; until then nothing is published or received (peers keep
  working over iroh meanwhile). The node info publisher now retries; a faster
  reconnect on network change would be better.
- `setcap` is lost on every rebuild: run from a copy (`~/.local/bin/toolnet`)
  and re-grant only when replacing it.
