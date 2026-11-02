# toolnet daemon

The `toolnet` binary: control plane on tool-db (Nostr), data plane on iroh
(QUIC with hole punching and relay fallback) and a TUN device.

```
src/tooldb.rs    native ToolDB values over Nostr (kind 30078, user namespaces)
src/crypto.rs    NIP-44 v2, pairwise and group, compatible with tool-db's groups.ts
src/model.rs     network config, members, node info; the verified "view"
src/control.rs   follows the network live, publishes our node info
src/engine.rs    iroh endpoint, TUN, peer set, connections
src/main.rs      CLI
```

## Quick start

Owner (once, on any machine; keep `owner.key` safe):

```bash
toolnet owner create --net home          # prints the owner npub
```

Each device:

```bash
toolnet init --owner npub1... --net home  # prints this device's npub
# owner:  toolnet owner approve npub1<device> --name laptop
sudo setcap cap_net_admin+ep $(which toolnet)
toolnet up                                # or: toolnet up --no-tun (no privileges)
```

`toolnet status` shows the network as this device sees it; `toolnet owner list`
shows it as the owner. `toolnet owner remove <name>` removes a device and
rotates the group key.

State lives in `~/.local/share/toolnet` (`--state` / `TOOLNET_STATE` to
override): `device.key`, `endpoint.key`, `owner.key` (owner only, all
`0600`) and `config.json` (owner, network, Nostr relays).

## What's in tool-db

Topic `toolnet`. O = owner, D = device, N = network. All values are
user-namespaced, so only their writer can change them:

| Key | Writer | Value |
|---|---|---|
| `:O.net-N` | owner | `{v, epoch, data}`: NetConfig, sealed with the group key |
| `:O.gk-N-D` | owner | `{v, epoch, key}`: the group key, NIP-44 from O to D (also for O itself) |
| `:D.node-O-N` | device | `{v, epoch, data}`: NodeInfo (endpoint id, addresses, relay), sealed |

Relays see who writes, when, and ciphertext. A device is a member only if
the newest owner-signed network lists it and its group key opens it; node
info only counts from listed members and at the current epoch.

## Tests

```bash
cargo test -p toolnet          # crypto, approve/remove/rotation logic
tests/e2e-local.sh             # local ToolDbRelay, owner + 2 devices, connect, remove
tests/compat.sh                # JS tool-db reads Rust's data and approves a device Rust accepts
```

The two scripts need a built `../tool-db` checkout (`TOOLDB_DIR` to override)
and Node 22.
