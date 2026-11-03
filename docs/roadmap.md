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
- **Hole punching LTE hotspot -> home works**, with a low-TTL trick learned
  from HyperDHT (`daemon/src/punch.rs`). Home is behind the ISP's carrier-grade
  NAT (router WAN 10.x.x.x) and LTE behind the carrier's; both map
  consistently, yet plain punching (iroh's, or raw UDP) never connected: our
  first packets reach the far side before it has sent anything, an ICMP error
  comes back, and our own NAT drops the mapping. Sending the first packets
  with TTL 5 opens our NATs without reaching the far side. Raw UDP A/B: TTL 64
  never connects, TTL 5 at once. HyperDHT between the same machines also went
  direct. toolnet: relay -> direct as soon as both sides knew each other's
  current address; ~100-200 ms instead of ~410 ms through n0's US relay.
  Throughput matched the hotspot's own download (~1.6 Mbit/s at the time).
- **Exit node from LTE** through the homelab: the internet sees the home IP.
- **First real run with TUN** (laptop + homelab, home Wi-Fi, public Nostr
  relays): tailnet ping both ways (direct, 2-3 ms), MagicDNS, 44 Mbit/s TCP
  over Wi-Fi (58 plain), the home LAN route accepted while at home without
  breaking the LAN, and the homelab as exit node (20 MB download through it).
  Bugs it found, all fixed and covered by tests: subnet routes replaced the
  local LAN route; helpers (`ip`, `nft`) didn't inherit file capabilities and
  `nft` isn't on a Debian user's PATH; `resolvectl` blocked the main loop on a
  polkit prompt; Docker's iptables FORWARD DROP broke the exit node.

## Next

1. **Confirm coordinated hole punching on LTE.** Root cause found and fixed
   in namespaces (`tests/netns-nat.sh`, direct in ~5 s every run; relayed
   forever with `TOOLNET_PUNCH_HOLD=0`): if a full-size packet reaches the far
   NAT before that side has sent toward us, the NAT records the flow as aimed
   at itself and the far device's own packets get a different public port.
   iroh's first probes did exactly that. Now a firewall rule holds iroh's
   full-size packets to a relayed peer's public address, both sides open their
   NATs with TTL-5 packets, agree over the relay connection (`\0PUN`), then
   release iroh and start a round. **First real LTE session (2026-11-03):
   direct, tailnet ping 92-130 ms instead of ~400 via the relay.** The
   hotspot link itself was flaky that evening (a dip of a few minutes, relay
   round trips up to 19 s), so time-to-direct still needs a clean measurement.
   Android will need another way to hold iroh's probes (no nft).
2. **Android app** (`android/`, WIP). Works: the Rust core over JNI in a
   VpnService, setup + join QR/code, approved from the owner side, tailnet and
   subnet routes. **First test (2026-11-03): phone on LTE (carrier NAT) ->
   homelab direct, ping 76-86 ms, LAN via the subnet route, Navidrome over
   `10.213.0.2:4533`.** Network switches work (measured over adb: names
   resolve 1-4 s after a switch, direct again over LTE in 9-13 s); MagicDNS
   works inside the tunnel (100.100.100.100: tailnet names answered, the rest
   forwarded to the network's DNS, public resolvers as fallback; bare names via
   the search domain). Tests start the VPN with `am start ... --ez connect true`.
   Next: always-on VPN / reboot, exit node, smaller APK (strip the .so),
   release signing, and a hold for iroh's first probes without nft.
3. **Owner key safety**: NIP-46 remote signer or the PWA, instead of
   `owner.key` on a laptop.
4. **IPv6** inside the tailnet (ULA per network), and IPv6 exit traffic.
5. **Exit node DNS**: send all DNS through the exit node while it's in use
   (today only the tailnet domain is split; other queries use local DNS).
6. **ACLs** (who may reach whom), evaluated on each device from the
   owner-signed policy.
7. **Relay**: a toolnet-owned TLS CA for relays reached by IP (decisions.md),
   and relays listed in the network config.
8. **Mobile**: Android VpnService / iOS NetworkExtension shells around the Rust
   core.
9. **Performance**: batch TUN reads, GSO, multiple queues. Today: ~800 Mbit/s
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
- After a network switch the Nostr websockets used to take 1-3 min to time
  out. Now a local address change (or a failed publish) recreates the relay
  connections at once (measured on LTE: switch noticed instantly, new address
  published ~30 s later with the first, racy version; < 5 s in namespaces now).
- `setcap` is lost on every rebuild: run from a copy (`~/.local/bin/toolnet`)
  and re-grant only when replacing it.
