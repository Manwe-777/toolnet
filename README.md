# toolnet

**A private mesh VPN with no coordination server.** Devices find each other,
exchange keys and learn routes and DNS names through signed, encrypted
records on [Nostr](https://nostr.com) relays (via
[tool-db](https://github.com/Manwe-777/tool-db)). Traffic goes peer to peer
over [iroh](https://iroh.computer): QUIC, with hole punching and a relay
fallback.

Think Tailscale, except the control plane is data you own, not a service
someone runs.

[![Latest release](https://img.shields.io/github/v/release/Manwe-777/toolnet?label=linux)](https://github.com/Manwe-777/toolnet/releases/latest)
[![Owner app](https://img.shields.io/badge/owner%20app-manwe--777.github.io%2Ftoolnet-blue)](https://manwe-777.github.io/toolnet/)

> **Status: working prototype, not audited.** Running daily on Linux and
> Android, including hole punching through carrier NAT on both ends (phone on
> LTE to a home server, direct, ~80–110 ms). Expect breaking changes.

## Features

- **No server to run or trust.** The network is a set of records signed by
  the owner's key. Relays store and forward them but only ever see ciphertext.
- **Owner-approved membership.** A device joins by showing a QR code; the
  owner scans it in the owner app. Removing a device rotates the group key.
- **Direct connections.** NAT hole punching, with an iroh relay when a direct
  path isn't possible (self-hostable, see [relay/](relay)).
- **MagicDNS.** Every member is reachable as `<name>.<network>.toolnet`.
- **Subnet routes and exit nodes.** Opt in on the device *and* approved by
  the owner.
- **One-line install** on Linux, with a tray icon on desktops; Android app;
  phone-friendly owner app.

## Getting started

### 1. Create a network

On any Linux machine, as your normal user:

```bash
curl -fsSLo toolnet https://github.com/Manwe-777/toolnet/releases/latest/download/toolnet-linux-x86_64
chmod +x toolnet
./toolnet owner create --net home
```

This prints the network's **owner npub** (public, used to join devices) and
stores the **owner key** in `~/.local/share/toolnet/owner.key`. Keep that key
safe: it controls the network. Use `toolnet-linux-aarch64` on ARM.

### 2. Open the owner app

**https://manwe-777.github.io/toolnet/** — sign in with the owner key (the hex
in `owner.key`, or an `nsec1…`). The key stays in that browser's local
storage. From there you approve devices, set their routes and remove them.
It's a PWA: add it to your phone's home screen.

### 3. Add devices

**Linux.** In the owner app, open **Add a device** and copy the command. It
looks like this:

```bash
curl -fsSL https://github.com/Manwe-777/toolnet/releases/latest/download/install.sh \
  | sh -s -- --owner npub1… --net home
```

It downloads the static binary for your architecture, checks it against the
release's `SHA256SUMS`, installs `iproute2` / `nftables` if missing, sets up
the systemd service (plus the tray icon on a desktop) and prints a QR code.
Scan it in the owner app, and the device joins on its own. Works on any
systemd distro (tested on Debian, Fedora and Arch).

- Update: `sudo toolnet update` (`--check` to only look). Re-running the
  one-liner works too; the identity and settings are kept either way.
- Remove: `sudo toolnet uninstall` keeps the identity and settings, so a
  reinstall comes back the same; `--purge` deletes them too.
- Options: `--args "--advertise-exit-node"`, `--operator <user>`, `--no-tray`;
  see `toolnet install --help`.

**Android.** Download the app from the owner app's home screen, pick the
network, and approve the join code it shows.

Check a device with `toolnet status`, or ping another one:
`ping laptop.home.toolnet`.

## Routes and exit nodes

Both sides have to agree: the device offers, the owner allows (in the owner
app, or with the CLI).

```bash
# Share the home LAN through the server
sudo toolnet install --args "--advertise-routes 192.168.1.0/24"   # on the server
toolnet owner routes server 192.168.1.0/24                         # owner allows it

# Use the server as an exit node
sudo toolnet install --args "--advertise-exit-node"                # on the server
toolnet owner routes server 0.0.0.0/0                              # owner allows it
toolnet exit-node server                                           # on the laptop (or from the tray)
```

## How it works

```
            control plane: tool-db on Nostr relays (relays only see ciphertext)
   ┌────────────────────────────────────────────────────────────────────┐
   │  :OWNER.net-N      members, IPs, routes, DNS   (signed by owner)   │
   │  :OWNER.gk-N-DEV   group key, wrapped per device                   │
   │  :DEV.node-OWNER-N endpoint id, addresses, relay (signed by device)│
   └───────────▲──────────────────────▲───────────────────────▲─────────┘
               │                      │                       │
         ┌─────┴─────┐  QUIC/iroh ┌───┴───────┐  QUIC/iroh ┌──┴────────┐
         │  laptop   │◄══════════►│  server   │◄══════════►│   phone   │
         └───────────┘            └───────────┘            └───────────┘
              direct after hole punching, or through an iroh relay
```

- The **owner** publishes the network document: members, their IPs, allowed
  routes and DNS names, signed with the owner key.
- Each **device** has its own key. It reads the network document, unwraps the
  group key the owner encrypted for it, and publishes its current endpoint and
  addresses, encrypted with the group key.
- Devices dial each other over iroh and carry IP packets from a TUN
  interface. When a member is removed, the owner rotates the group key, and
  the removed device can no longer read anything new.

The exact data format is in [daemon/README.md](daemon/README.md); the design
and its trade-offs are in [docs/](docs).

## Platforms

| Platform | Status |
|---|---|
| Linux (x86_64, aarch64) | Daemon, systemd service, tray icon, one-line installer |
| Android | App (VpnService), including exit node |
| Owner app (any browser) | PWA on GitHub Pages, or self-hosted ([app/](app)) |
| macOS, Windows | Planned ([docs/installers.md](docs/installers.md)) |

## Self-hosting

Nothing has to run on our side, but every piece can run on yours:

- **Nostr relays:** pass `--relay wss://…` (repeatable) to `toolnet install` /
  `toolnet owner create`, and set the same relays in the owner app's settings.
- **iroh relay:** [relay/](relay) has a Docker setup for a machine reachable
  from the internet.
- **Owner app:** a static build; `app/Dockerfile` serves it with nginx. Point
  the tray at it with `TOOLNET_OWNER_APP=<url>`.

## Building from source

```bash
cargo build --release -p toolnet -p toolnet-tray
sudo target/release/toolnet install --owner npub1… --net home   # installs the tray too

packaging/build-linux.sh        # static musl binaries for x86_64 + aarch64 (Docker)
packaging/release.sh 0.3.0      # build and publish a GitHub release
android/publish.sh 0.1.2        # build, sign and publish the Android app
```

The owner app: `cd app && npm install && npm run dev`. It deploys to GitHub
Pages from `main` ([.github/workflows/owner-app.yml](.github/workflows/owner-app.yml)).

Without the service, the CLI runs a device by hand:

```bash
B=target/release/toolnet
$B init --owner npub1… --net home
$B id --qr                                   # join code for the owner app
$B owner approve npub1<device> --name laptop # or approve from the CLI
sudo setcap cap_net_admin,cap_net_bind_service+ep $B
$B up
```

## Tests

```bash
cargo test -p toolnet        # unit: crypto, membership/rotation, CIDR, DNS
tests/e2e-local.sh           # owner + 2 devices on a local relay: join, connect, remove
tests/compat.sh              # JS tool-db reads Rust's data; Rust accepts a JS approval
tests/app-compat.sh          # owner app logic (JS) drives Rust devices: approve, routes, remove
tests/netns-tun.sh           # TUN traffic in namespaces, MagicDNS, subnet routes, connect/disconnect
tests/netns-exit.sh          # exit node: internet only through the exit, switched at runtime
tests/netns-nat.sh           # two devices behind NAT routers: hole punching goes direct (needs iroh-relay)
tests/netns-roam.sh          # network switch: reconnect, republish, keep following
tests/installer.sh           # one-line installer on Debian, Fedora, Arch (Docker, public relays)
```

The namespace tests are unprivileged (user namespaces) and offline. They need
a built `../tool-db` checkout and Node 22.

## Repository layout

| Path | What |
|---|---|
| [daemon/](daemon) | The `toolnet` binary (Rust): control plane, crypto, iroh + TUN engine, MagicDNS, installer |
| [tray/](tray) | `toolnet-tray`: tray icon for Linux desktops (connect, exit node, devices) |
| [app/](app) | Owner app (React PWA): approve devices by QR, routes, removal with key rotation |
| [android/](android) | Android app: Kotlin VpnService around the Rust core |
| [packaging/](packaging) | systemd units, `install.sh`, static builds, releases |
| [relay/](relay) | Self-hostable iroh relay (Docker) |
| [tests/](tests) | End-to-end tests (see above) |
| [docs/](docs) | Design docs |
| [spike/](spike), [tools/](tools) | Early connectivity experiments and NAT probes |

## Documentation

| Doc | |
|---|---|
| [architecture.md](docs/architecture.md) | Components, peer roles, the hard parts |
| [flows.md](docs/flows.md) | Join, roaming, removal, exit node, DNS |
| [data-model.md](docs/data-model.md) | Original data model design (the implemented format is in [daemon/README.md](daemon/README.md)) |
| [decisions.md](docs/decisions.md) | Decision log and open questions |
| [roadmap.md](docs/roadmap.md) | What's done, what's next |
| [installers.md](docs/installers.md) | Installers: Linux (done), macOS, Windows |
| [daemon-options.md](docs/daemon-options.md) | Why Rust and iroh |
| [tailscale-primer.md](docs/tailscale-primer.md) | How Tailscale works, mapped to toolnet |
