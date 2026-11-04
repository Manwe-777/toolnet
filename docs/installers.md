# Installers

Goal: one command installs toolnet on a Linux or macOS box and pairs it with
a network, with no package repository and no app store. Windows gets the same
shape through PowerShell.

**Linux is built** (2026-11-04); macOS and Windows below are still plans.
Downloads are GitHub Release assets (`releases/latest/download/...`), and the
owner app is on GitHub Pages (https://manwe-777.github.io/toolnet/): no
website of our own. Test the installer offline with `TOOLNET_DOWNLOAD=<dir>`.

| Piece | Where |
|---|---|
| `toolnet install` / `uninstall` | `daemon/src/install.rs` (units embedded from `packaging/`) |
| static musl builds, x86_64 + aarch64 | `packaging/build-linux.sh` (Docker, cargo-zigbuild) |
| the `curl \| sh` script | `packaging/install.sh` |
| GitHub release (binaries, `SHA256SUMS`, `install.sh`, `VERSION`) | `packaging/release.sh VERSION` |
| `toolnet update` | `daemon/src/install.rs` |
| the copyable command | owner app, "Add a device" |
| distro tests (Debian, Fedora, Arch, with systemd) | `tests/installer.sh` |

Checksums only catch broken downloads: the script, the binaries and
`SHA256SUMS` all come from the same release, so a signature (minisign, key
embedded in the script) only adds something once the script is served from
somewhere else.

## Principle: a thin script, a smart binary

The shell script only detects the platform, downloads and verifies the
binary, then hands over to `toolnet install`. Everything else (service unit,
state directory, `init`, pairing) lives in the binary, so the same logic serves
Linux, macOS and Windows, and the script stays small enough to read before
piping it into a shell.

## The one-liner

```bash
curl -fsSL https://github.com/Manwe-777/toolnet/releases/latest/download/install.sh \
  | sh -s -- --owner npub1... --net test1
```

The owner app's "Add device" screen shows this command with the owner npub and
network filled in, ready to copy. An npub is public, so it's fine on a command
line.

`install.sh`:

1. Detect `uname -s` (linux, darwin) and `uname -m` (x86_64, aarch64).
2. Download `toolnet-linux-<arch>` (and `toolnet-tray-linux-<arch>` when
   there's a desktop) and check them against `SHA256SUMS`.
3. Run `sudo toolnet install --owner … --net …` with the same arguments.

`toolnet install`:

- install iproute2 and nftables if missing (apt, dnf, pacman, zypper, apk);
  warn when systemd-resolved isn't running (no MagicDNS names locally)
- copy itself, and `toolnet-tray` if it sits next to it, to `/usr/local/bin`
- write the service definition (embedded in the binary) and drop-ins
  (`args.conf`, `operator.conf` = the sudo user, `log.conf`)
- create the state directory (`/var/lib/toolnet`, 0700) and run `init`, or
  copy an identity with `--state-from`
- enable and start the service; the tray unit is enabled `--global` (every
  user's graphical session) and started for the sudo user
- print the join QR in the terminal (`toolnet id --qr`) on a new identity;
  the owner scans it in the owner app and the device joins, the same flow as
  the phone

### Updates

`sudo toolnet update` (`--check` only reports, no root needed; `--force`
reinstalls):

1. read the latest release's `VERSION` and compare it with its own
2. download `toolnet-linux-<arch>`, and `toolnet-tray-linux-<arch>` if the
   tray is installed (even over SSH, unlike `install.sh`, which only fetches
   it with a desktop session), and check them against `SHA256SUMS`
3. run the new binary's `install`, which keeps the identity, the `up` flags,
   the log level and the operator

HTTPS uses rustls on ring with the Mozilla roots built in, so it doesn't
depend on the system's certificates. Re-running the one-liner still works too
(`--owner` can be left out). `release.sh` refuses a version that isn't the
crates' own, or every device would think it's always out of date.

Later: the tray could show "update available" (`toolnet update --check` is
the building block), and an opt-in timer could run it unattended.

## Linux

- **One binary for every distro**: no OpenSSL (`ring` only), so the static
  musl builds run anywhere.
- **System tools**: the daemon calls `ip`, `nft`, `iptables` and `resolvectl`.
  Longer term, netlink directly (rtnetlink crate) would remove the `ip` and
  `nft` dependencies.
- **No systemd** (Alpine, Void): `toolnet install` refuses and prints how to
  run it by hand. An OpenRC or runit service only if such a box shows up.

## macOS

A real port, but no signing needed: files fetched with `curl` don't get the
quarantine flag, so Gatekeeper doesn't block an unsigned binary. A root daemon
on `utun` works without Apple's Network Extension (Tailscale's open-source
macOS daemon works this way).

| Linux today | macOS equivalent |
|---|---|
| TUN device | `utun` (tun-rs already supports it) |
| `ip route`, table 5213 | `route` command or a routing socket |
| systemd-resolved split DNS | a file `/etc/resolver/<domain>` (easy) |
| policy routing for exit nodes | doesn't exist: two half-default routes (`0/1`, `128/1`) plus iroh's sockets pinned to the real interface (`IP_BOUND_IF`); the hard part |
| nft punch hold | a `pf` anchor |
| systemd unit | launchd plist in `/Library/LaunchDaemons` |
| forwarding + masquerade (subnet router) | `sysctl net.inet.ip.forwarding` + pf NAT; low priority |

## Windows

No MSI needed to start:

```powershell
irm https://github.com/Manwe-777/toolnet/releases/latest/download/install.ps1 | iex
```

It downloads `toolnet.exe` and `wintun.dll` (WireGuard's signed TUN driver,
free to redistribute), then runs `toolnet install` as administrator, which
registers a Windows service (windows-service crate). Downloading through
PowerShell avoids SmartScreen's warning for unsigned files (no Mark of the
Web), so code signing can wait.

Porting work:

- TUN: wintun (tun-rs supports it)
- routes: IP Helper API
- split DNS: NRPT (per-domain DNS rules)
- punch hold: no simple equivalent (WFP filters); same open problem as
  Android
- exit node: like macOS, interface-bound sockets plus default route override

An MSI (cargo-wix) only matters later, for a double-click installer; that's
also when a code-signing certificate becomes worth paying for.

## Order

1. ~~**Linux end to end**~~ (done; public once the repo is).
2. **macOS**, once there's a Mac to test on. Moderate without the exit node;
   more with it.
3. **Windows** last.
