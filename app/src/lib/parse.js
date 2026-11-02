// Parsing that must match the Rust daemon (daemon/src/main.rs, model.rs,
// engine.rs). Plain JS, no browser APIs: also used by the tests from Node.
import { npubDecode } from "tool-db";

/**
 * A device public key as npub (with or without "nostr:", stray spaces from
 * copying) or 64-hex, returned as hex. Like Rust's `parse_pubkey`
 * @throws with a message that says what's wrong
 */
export function parsePubkey(value) {
  const cleaned = String(value).replace(/\s+/g, "").replace(/^nostr:/i, "").toLowerCase();
  if (cleaned.startsWith("nsec1")) {
    throw new Error("That's a secret key (nsec). Use the device's npub instead");
  }
  if (/^[0-9a-f]{64}$/.test(cleaned)) return cleaned;
  if (!cleaned.startsWith("npub1")) {
    throw new Error("A device key starts with npub1 (or is 64 hex characters)");
  }
  let hex;
  try {
    hex = npubDecode(cleaned);
  } catch {
    throw new Error("That npub has a typo (its checksum doesn't match)");
  }
  if (!/^[0-9a-f]{64}$/.test(hex)) throw new Error("That npub isn't a public key");
  return hex;
}

/**
 * What a join QR code says. `toolnet id --qr` prints
 *   toolnet:join?device=<npub1...>&name=<suggested-name>
 * A bare npub or 64-hex key is accepted too.
 * @returns {{ device: string, name: string }} device as hex, name may be ""
 */
export function parseJoin(text) {
  const value = String(text).trim();
  const match = value.match(/^toolnet:\/{0,2}join\?(.*)$/is);
  if (!match) return { device: parsePubkey(value), name: "" };
  const params = new URLSearchParams(match[1]);
  const device = params.get("device");
  if (!device) throw new Error("This toolnet code has no device key");
  return { device: parsePubkey(device), name: (params.get("name") || "").trim() };
}

/**
 * An IPv4 address like Rust's `Ipv4Addr::from_str`: four decimal octets,
 * no leading zeros
 * @returns the address as an unsigned 32 bit number, or null
 */
export function parseIpv4(value) {
  const parts = String(value).split(".");
  if (parts.length !== 4) return null;
  let n = 0;
  for (const p of parts) {
    if (!/^(0|[1-9][0-9]{0,2})$/.test(p) || Number(p) > 255) return null;
    n = n * 256 + Number(p);
  }
  return n;
}

export const formatIpv4 = (n) => [24, 16, 8, 0].map((s) => (n >>> s) & 255).join(".");

const mask = (len) => (len === 0 ? 0 : (0xffffffff << (32 - len)) >>> 0);

/**
 * A prefix like Rust's `Cidr::from_str`: "a.b.c.d/len" or a bare address
 * (/32), normalized to its network address: 192.168.1.7/24 -> 192.168.1.0/24
 * @returns {string} the normalized prefix
 * @throws on anything Rust would reject
 */
export function parseCidr(value) {
  const s = String(value).trim();
  const slash = s.indexOf("/");
  const [ip, len] = slash === -1 ? [s, "32"] : [s.slice(0, slash), s.slice(slash + 1)];
  // Rust's u8 parse: digits, optional leading "+"
  if (!/^\+?[0-9]+$/.test(len) || Number(len) > 255) throw new Error(`bad prefix length in ${s}`);
  const n = Number(len);
  if (n > 32) throw new Error(`bad prefix length in ${s}`);
  const addr = parseIpv4(ip);
  if (addr === null) throw new Error(`not an IPv4 prefix: ${s}`);
  return `${formatIpv4((addr & mask(n)) >>> 0)}/${n}`;
}

/** The exit node route */
export const EXIT_ROUTE = "0.0.0.0/0";
export const isExitRoute = (cidr) => /\/0$/.test(cidr);

/**
 * The network's range, like Rust's `NetConfig::prefix`
 * @returns {{ base: number, len: number }}
 */
export function parsePrefix(ipv4) {
  const [ip, len, ...rest] = String(ipv4).split("/");
  if (len === undefined || rest.length) throw new Error("ipv4 must be a CIDR");
  const base = parseIpv4(ip);
  if (base === null || !/^\+?[0-9]+$/.test(len) || Number(len) > 32) {
    throw new Error(`bad network range ${ipv4}`);
  }
  return { base, len: Number(len) };
}

/**
 * The lowest free host address in the range, like Rust's `next_free_ip`
 */
export function nextFreeIp(config) {
  const { base: raw, len } = parsePrefix(config.ipv4);
  const base = (raw & mask(len)) >>> 0;
  const size = 2 ** (32 - len);
  const used = new Set(Object.values(config.members || {}).map((m) => m.ip));
  for (let i = 1; i < size - 1; i++) {
    const ip = formatIpv4(base + i);
    if (!used.has(ip)) return ip;
  }
  throw new Error(`no free addresses in ${config.ipv4}`);
}

/**
 * Checks an IP the owner typed for a new member. Rust takes any address;
 * we also refuse ones outside the range and ones already in use
 * @returns the address, normalized
 */
export function checkMemberIp(config, value) {
  const n = parseIpv4(String(value).trim());
  if (n === null) throw new Error(`not an IPv4 address: ${value}`);
  const { base: raw, len } = parsePrefix(config.ipv4);
  const base = (raw & mask(len)) >>> 0;
  const size = 2 ** (32 - len);
  if (n < base + 1 || n > base + size - 2) throw new Error(`${value} is not a host address in ${config.ipv4}`);
  const ip = formatIpv4(n);
  const taken = Object.values(config.members || {}).find((m) => m.ip === ip);
  if (taken) throw new Error(`${ip} is already used by ${taken.name}`);
  return ip;
}

/** A MagicDNS label, like Rust's `dns::label` */
export const dnsLabel = (name) =>
  String(name)
    .toLowerCase()
    .replace(/[^a-z0-9]/gu, "-")
    .replace(/^-+|-+$/g, "");

/** The network's MagicDNS domain, like `NetConfig::dns_domain` */
export const dnsDomain = (config) => (config.domain ?? `${dnsLabel(config.name)}.toolnet`).replace(/\.+$/, "").toLowerCase();

/** A member's MagicDNS name */
export const dnsName = (config, member) => `${dnsLabel(member.name)}.${dnsDomain(config)}`;
