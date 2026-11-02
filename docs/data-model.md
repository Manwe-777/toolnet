# Data model (draft)

Topic: `toolnet`. `O` is the owner pubkey, `D`/`P` are device pubkeys and `N`
is a tailnet id chosen by the owner (default `default`). Keys use tool-db user
namespaces (`:pubkey.key`), so only that pubkey can write them, the same
pattern as `homelab-monitor/shared/src/index.js`.

Values are kind 30078 (latest per key wins, by `created_at`, ties to the
lowest id).

## Keys

| Key | Writer | Encryption | Value |
|---|---|---|---|
| `:O.net-N` | owner | `G` (current epoch) | `NetConfig` |
| `:O.gk-N-D` | owner | NIP-44 `O`→`D` (`wrapGroupKey`) | `{ epoch, key }` |
| `:D.node-O-N` | device | `G` | `NodeInfo` |
| `:D.prefs-O-N` | device | NIP-44 `D`→`D` (self) | local prefs worth syncing, e.g. chosen exit node (optional) |

The current epoch lives in the plaintext envelope of `net` (`{ epoch, payload }`),
so a device knows which `G` it needs before decrypting.

## Types

```ts
type NetConfig = {
  name: string;                 // "home"
  domain: string;               // MagicDNS suffix, "home.toolnet"
  epoch: number;                // group key version, bumps on removal
  ipv4: string;                 // "10.213.0.0/16" by default, NOT 100.64.0.0/10 (Tailscale's range: must coexist)
  ipv6: string;                 // random ULA fdXX:XXXX:XXXX::/48 per tailnet (fd7a:115c:a1e0::/48 is Tailscale's)
  members: Record<DevicePubkey, {
    name: string;               // "homelab", unique, becomes the DNS name
    ip4: string;                // assigned by the owner, no collisions
    ip6: string;
    approvedRoutes: string[];   // subset of what the node advertises: "0.0.0.0/0", "192.168.1.0/24"
    tags: string[];             // for ACLs: "server", "phone"
    addedAt: number;
  }>;
  dns: {
    magicDns: boolean;
    nameservers: string[];      // upstream for everything else
    searchDomains: string[];
    splitDns?: Record<string, string[]>;   // "corp.example" -> ["192.168.1.1"]
  };
  acl: AclRule[];               // empty = all to all (MVP)
  relays: string[];             // Nostr relays for this tailnet
  derp?: DerpMap;               // relay map (path B, or our own relay)
  updatedAt: number;
};

type NodeInfo = {
  wgPubkey: string;             // Curve25519, base64
  discoKey?: string;            // path B: tailscaled's disco key
  endpoints: string[];          // "203.0.113.4:41641", "192.168.1.20:41641", "[2001:db8::1]:41641"
  homeRelay?: number;           // preferred DERP region
  advertisedRoutes: string[];   // what this node offers; the owner approves a subset
  hostname: string;
  os: string;
  version: string;
  updatedAt: number;
};
```

## Validity rules (each device evaluates them locally)

A peer `P` gets configured if and only if:
1. the newest `:O.net-N` that decrypts lists `P` in `members`, and
2. `:P.node-O-N` exists, is signed by `P` (namespace guarantees it) and
   decrypts with the current `G`, and
3. the ACL allows it (MVP: always).

The peer's AllowedIPs are its `ip4/ip6` plus `approvedRoutes ∩ advertisedRoutes`.

## Signaling (ephemeral, never stored)

Kind **21080** (next free after tool-db's 21078/21079 function kinds, to be
confirmed): tags `["p", target]` and `["t", "toolnet"]`. Content is NIP-44
from sender to target:

```ts
type Signal =
  | { type: "call-me-maybe"; endpoints: string[]; txid: string; ts: number }
  | { type: "pong"; txid: string; observed: string; ts: number };
```

Receivers drop signals older than 60s and signals from non-members, the same
replay rule as tool-db server functions. Published with `RelayPool.publish`.

## Open points

- Should `members` be a MapCRDT instead of one value? A single writer (the
  owner) makes a plain value fine, unless several owner devices edit at once.
- The size of `net` grows with members. That's fine for tens of devices, and
  NIP-11 limits apply.
- Instead of storing the wrapped group key per device, a NIP-17/59 gift wrap
  could hide who is in the tailnet from relays.
