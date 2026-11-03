# Installers (plan, not built yet)

Goal: one command installs toolnet on a Linux or macOS box and pairs it with
a network, with no package repository and no app store. Windows gets the same
shape through PowerShell. Downloads are served by the owner app, like the
Android APK (`toolnet.manuh.app/download/`), because the repo is private and
GitHub Releases can't serve public downloads.

## Principle: a thin script, a smart binary

The shell script only detects the platform, downloads and verifies the
binary, then hands over to `toolnet install`. Everything else (service unit,
state directory, `init`, pairing) lives in the binary, so the same logic serves
Linux, macOS and Windows, and the script stays small enough to read before
piping it into a shell.

## The one-liner

```bash
curl -fsSL https://toolnet.manuh.app/install.sh | sh -s -- --owner npub1... --net test1
```

The owner app's "Add device" screen shows this command with the owner npub and
network filled in, ready to copy. An npub is public, so it's fine on a command
line.

`install.sh`:

1. Detect `uname -s` (linux, darwin) and `uname -m` (x86_64, aarch64).
2. Download `toolnet-<os>-<arch>` plus a signed checksum from
   `/download/`. The signing public key is embedded in the script, so a
   tampered binary on the server is rejected, not only a corrupted download.
   minisign or a plain ed25519 signature is enough.
3. Run `sudo toolnet install --owner … --net …`.

`toolnet install` (new subcommand, absorbs `packaging/install.sh`):

- copy itself to `/usr/local/bin/toolnet`
- write the service definition (embedded in the binary) and drop-ins
  (`args.conf`, `log.conf`)
- create the state directory (`/var/lib/toolnet`, 0700) and run `init`
- enable and start the service
- print the join QR in the terminal (`toolnet id --qr`); the owner scans it
  in the owner app and the device joins, the same flow as the phone

Updates: re-run the same command. Later, `toolnet update` checks a
`version.json` (like the Android download card) and replaces the binary.

## Linux

Mostly packaging.

- **One binary for every distro**: the only crypto dependency is `ring` (no
  OpenSSL), so static musl builds for `x86_64` and `aarch64` should work.
  `publish.sh` builds both (cargo-zigbuild or cross) and uploads them next to
  the APK.
- **System tools**: the daemon calls `ip`, `nft`, `iptables` and `resolvectl`.
  The script checks for them and installs what's missing (apt, dnf, pacman,
  apk), or warns. Longer term, netlink directly (rtnetlink crate) would remove
  the `ip` and `nft` dependencies.
- **No systemd** (Alpine, Void): fall back to an OpenRC or runit service, or
  print how to run it by hand. Only if such a box actually shows up.
- **Test**: a clean container or VM per distro (Debian, Fedora, Arch).

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
irm https://toolnet.manuh.app/install.ps1 | iex
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

1. **Linux end to end**: `toolnet install`, musl builds in `publish.sh`,
   `install.sh` + signed checksums served by the owner app, the copyable
   command in "Add device". About an afternoon, tested in containers.
2. **macOS**, once there's a Mac to test on. Moderate without the exit node;
   more with it.
3. **Windows** last.
