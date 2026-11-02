// toolnet's data in tool-db, as the network owner. Same keys, envelopes and
// semantics as the Rust daemon (daemon/src/main.rs `owner`, daemon/src/model.rs):
//
//   :O.net-N       {v:1, epoch:E, data: group(E) ciphertext of NetConfig}
//   :O.gk-N-D      {v:1, epoch:E, key: NIP-44 O->D ciphertext of group key E}
//   :D.node-O-N    {v:1, epoch:E, data: group(E) ciphertext of NodeInfo}
//
// Plain JS, no browser APIs: the tests use it from Node (tests/app-compat.mjs).
import {
  ToolDb,
  createGroupKey,
  decryptForGroup,
  encryptForGroup,
  unwrapGroupKey,
  wrapGroupKey,
} from "tool-db";
import { checkMemberIp, nextFreeIp, parseCidr, parsePrefix, parsePubkey } from "./parse.js";

export const TOPIC = "toolnet";

/** Rust's DEFAULT_RELAYS (daemon/src/main.rs) */
export const DEFAULT_RELAYS = [
  "wss://relay.damus.io",
  "wss://nos.lol",
  "wss://relay.primal.net",
  "wss://nostr.mom",
];

export const DEFAULT_IPV4 = "10.213.0.0/24";

export const netKey = (net) => `net-${net}`;
export const gkKey = (net, device) => `gk-${net}-${device}`;
export const nodeKey = (owner, net) => `node-${owner}-${net}`;

/** Seconds, like Rust's `now()` */
export const now = () => Math.floor(Date.now() / 1000);

const GET_TIMEOUT = 4000;

/**
 * Connects to the relays and signs in as the owner
 * @param options extra ToolDb options (storageAdapter, storageName in the browser)
 */
export async function connect(secretKey, relays, options = {}) {
  const db = new ToolDb({ relays, topic: TOPIC, ...options });
  await db.ready;
  await db.keysSignIn(secretKey, "owner");
  await waitForRelays(db);
  return db;
}

export function waitForRelays(db, timeoutMs = 5000) {
  return new Promise((resolve) => {
    const started = Date.now();
    const poll = () => {
      if (db.isConnected || Date.now() - started > timeoutMs) resolve(db.isConnected);
      else setTimeout(poll, 100);
    };
    poll();
  });
}

export const ownerOf = (db) => db.userAccount.getAddress();

/** The newest value of a user-namespaced key (also remembered locally) */
const get = (db, author, key) => db.getData(`:${author}.${key}`, false, GET_TIMEOUT);

/**
 * Writes one of our values and waits until a relay accepts it, like Rust's
 * `Db::put`. Read the key first (`get`): putData makes the new value newer
 * than the one it knows, even within the same second
 */
async function put(db, key, value) {
  const event = await db.putData(key, value, true);
  // putData publishes without waiting; publishing the same event again is
  // harmless and tells us whether any relay accepted it
  const { accepted, rejected } = await db.relays.publish(event);
  if (accepted.length === 0) {
    const why = rejected.map((r) => `${r.url}: ${r.reason}`).join(", ") || "no relay answered";
    throw new Error(`No relay accepted ${key} (${why})`);
  }
  return event;
}

const isEpoch = (e) => Number.isInteger(e) && e >= 0;

/** Seals a value for the group: `seal` in model.rs */
export const seal = (group, epoch, value) => ({ v: 1, epoch, data: encryptForGroup(group, JSON.stringify(value)) });

function open(group, envelope) {
  if (!envelope || !isEpoch(envelope.epoch) || typeof envelope.data !== "string") throw new Error("bad envelope");
  return { epoch: envelope.epoch, value: JSON.parse(decryptForGroup(group, envelope.data)) };
}

async function wrapFor(db, device, group, epoch) {
  return { v: 1, epoch, key: await wrapGroupKey(db.userAccount, group, device) };
}

/**
 * The network as its owner sees it: `model::view` with me = owner
 * @returns {Promise<
 *   { status: "none" } |
 *   { status: "locked", epoch: number } |
 *   { status: "ok", epoch: number, group: string, config: object, nodes: Record<string, object> }
 * >} "locked": the network exists but this key can't open it (not its owner key?)
 */
export async function loadNetwork(db, net, { withNodes = true } = {}) {
  const owner = ownerOf(db);
  const [envelope, wrapped] = await Promise.all([get(db, owner, netKey(net)), get(db, owner, gkKey(net, owner))]);
  if (!envelope || !isEpoch(envelope.epoch)) return { status: "none" };
  const { epoch } = envelope;

  let group;
  try {
    if (wrapped?.epoch !== epoch || typeof wrapped.key !== "string") throw new Error("no key for this epoch");
    group = await unwrapGroupKey(db.userAccount, wrapped.key, owner);
  } catch {
    return { status: "locked", epoch };
  }
  let config;
  try {
    config = open(group, envelope).value;
  } catch {
    return { status: "locked", epoch };
  }
  config.members ||= {};

  const nodes = {};
  if (withNodes) {
    await Promise.all(
      Object.keys(config.members).map(async (device) => {
        try {
          const info = open(group, await get(db, device, nodeKey(owner, net)));
          // Only node info sealed at the current epoch counts
          if (info.epoch === epoch) nodes[device] = info.value;
        } catch {
          // no node info yet, or not readable
        }
      })
    );
  }
  return { status: "ok", epoch, group, config, nodes };
}

async function openNetwork(db, net) {
  const view = await loadNetwork(db, net, { withNodes: false });
  if (view.status === "none") throw new Error(`No network "${net}" for this owner key`);
  if (view.status === "locked") throw new Error(`Can't open network "${net}": is this the right owner key?`);
  return view;
}

/**
 * Publishes the group key for the owner and every member of the config,
 * then the config: `publish_network` in main.rs. Only the members listed in
 * `config` get this group key
 */
async function publishNetwork(db, net, epoch, group, config) {
  const owner = ownerOf(db);
  const recipients = [owner, ...Object.keys(config.members).filter((k) => k !== owner)];
  await Promise.all(recipients.map((device) => get(db, owner, gkKey(net, device))));
  await Promise.all(
    recipients.map(async (device) => put(db, gkKey(net, device), await wrapFor(db, device, group, epoch)))
  );
  await put(db, netKey(net), seal(group, epoch, config));
}

/**
 * Finds a member by name or key, like Rust: first match in key order
 * @returns the member's key (hex)
 */
export function findMember(config, nameOrKey) {
  let asKey = null;
  try {
    asKey = parsePubkey(nameOrKey);
  } catch {
    // a name
  }
  const key = Object.keys(config.members)
    .sort()
    .find((k) => config.members[k].name === nameOrKey || k === asKey);
  if (!key) throw new Error(`No member ${nameOrKey}`);
  return key;
}

/**
 * `toolnet owner create`: a new network at epoch 1
 */
export async function createNetwork(db, net, { ipv4 = DEFAULT_IPV4, domain } = {}) {
  const existing = await loadNetwork(db, net, { withNodes: false });
  if (existing.status === "ok") throw new Error(`Network ${net} already exists for this owner`);
  if (existing.status === "locked") throw new Error(`Network ${net} exists but this key can't open it`);
  parsePrefix(ipv4);
  const config = { name: net, ipv4, members: {}, updatedAt: now(), ...(domain ? { domain } : {}) };
  await publishNetwork(db, net, 1, createGroupKey(), config);
  return config;
}

/**
 * `toolnet owner approve <device> --name <name> [--ip <ip>]`: same epoch,
 * only the new member's group key is new
 * @returns {Promise<{ device: string, name: string, ip: string }>}
 */
export async function approveDevice(db, net, { device, name, ip }) {
  const key = parsePubkey(device);
  name = String(name || "").trim();
  if (!name) throw new Error("The device needs a name");
  const view = await openNetwork(db, net);
  const { config, group, epoch } = view;
  if (Object.values(config.members).some((m) => m.name === name)) {
    throw new Error(`A member is already named ${name}`);
  }
  const address = ip ? checkMemberIp(config, ip) : nextFreeIp(config);
  config.members[key] = { name, ip: address, addedAt: now() };
  config.updatedAt = now();

  await get(db, ownerOf(db), gkKey(net, key));
  await put(db, gkKey(net, key), await wrapFor(db, key, group, epoch));
  await put(db, netKey(net), seal(group, epoch, config));
  return { device: key, name, ip: address };
}

/**
 * `toolnet owner remove <device>`: new epoch and a fresh group key, wrapped
 * for the owner and the remaining members only, so the removed device can't
 * read anything published from now on
 * @returns {Promise<{ device: string, name: string, ip: string, epoch: number }>}
 */
export async function removeDevice(db, net, nameOrKey) {
  const { config, epoch } = await openNetwork(db, net);
  const key = findMember(config, nameOrKey);
  const member = config.members[key];
  delete config.members[key];
  config.updatedAt = now();
  const next = epoch + 1;
  const group = createGroupKey();
  await publishNetwork(db, net, next, group, config);
  return { device: key, name: member.name, ip: member.ip, epoch: next };
}

/**
 * `toolnet owner routes <device> [cidr...]`: the prefixes a member may route
 * (none to clear). 0.0.0.0/0 makes it an exit node
 * @returns {Promise<string[]>} the routes, normalized
 */
export async function setRoutes(db, net, nameOrKey, routes) {
  const parsed = routes.map(parseCidr);
  const { config, group, epoch } = await openNetwork(db, net);
  const member = config.members[findMember(config, nameOrKey)];
  if (parsed.length) member.routes = parsed;
  else delete member.routes;
  config.updatedAt = now();
  await put(db, netKey(net), seal(group, epoch, config));
  return parsed;
}
