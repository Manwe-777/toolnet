# toolnet

A Tailscale-style private network whose **control plane lives on Nostr**, through
[tool-db](../tool-db). There is no coordination server. Peers find each other, exchange
WireGuard keys and endpoints, and get routes and DNS config from signed,
encrypted tool-db data on public or self-hosted Nostr relays.

> Status: **design phase**. Nothing is implemented yet. "toolnet" is a working name.

## The idea in one picture

```
            control plane (tool-db over Nostr relays, ciphertext only)
   ┌───────────────────────────────────────────────────────────────────┐
   │  :OWNER.net        members, IPs, routes, DNS   (signed by owner)  │
   │  :DEVICE.node-O    wg key, endpoints, routes   (signed by device) │
   │  ephemeral 2108x   hole-punch signaling        (NIP-44, p-tagged) │
   └───────────▲─────────────────────▲─────────────────────▲───────────┘
               │                     │                     │
         ┌─────┴─────┐         ┌─────┴─────┐         ┌─────┴─────┐
         │  laptop   │◄═══════►│  homelab  │◄═══════►│   phone   │
         └───────────┘   WG    └───────────┘   WG    └───────────┘
                    data plane: WireGuard, direct or via a relay
```

Tailscale's coordination server only exchanges keys and metadata, never
traffic. toolnet keeps that split. tool-db replaces the coordination server,
and WireGuard still carries the packets.

## Docs

| Doc | What's in it |
|---|---|
| [docs/tailscale-primer.md](docs/tailscale-primer.md) | How Tailscale works, and which parts we have to replace or reuse |
| [docs/architecture.md](docs/architecture.md) | Components, the two implementation paths, the hard parts |
| [docs/data-model.md](docs/data-model.md) | tool-db keys, namespaces, encryption, event kinds |
| [docs/flows.md](docs/flows.md) | Join, endpoint updates, hole punching, revocation, exit nodes, DNS |
| [docs/daemon-options.md](docs/daemon-options.md) | Rust/Go/Node stack options for the daemon |
| [docs/decisions.md](docs/decisions.md) | Decision log and open questions. **Read this first when resuming.** |
| [docs/roadmap.md](docs/roadmap.md) | Milestones, starting from the smallest proof of concept |

## Related projects in this workspace

- `../tool-db`: the database (Nostr-first branch, see `NOSTR_FIRST.md`).
- `../tool-db-docs`: tool-db documentation site.
- `../homelab-monitor`: a working tool-db app. Its owner npub, QR pairing and
  NIP-44 values are the patterns toolnet reuses.
