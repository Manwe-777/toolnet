// Cross-implementation check: JS tool-db reads what the Rust daemon wrote,
// and writes an approval the Rust daemon must accept.
//   node tests/compat.mjs <relay url> <owner state dir> <net> [<device hex> <name> <ip>]
// With the last three arguments, also approves that device from JS.
import fs from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";

const toolDbDir = process.env.TOOLDB_DIR ?? path.resolve(import.meta.dirname, "../../tool-db");
const tdb = await import(pathToFileURL(path.join(toolDbDir, "packages/tool-db/dist/index.js")).href);
const { ToolDb, unwrapGroupKey, decryptForGroup, encryptForGroup, wrapGroupKey, getPublicKey } = tdb;

const [relay, ownerDir, net, newDevice, newName, newIp] = process.argv.slice(2);
const ownerSecret = fs.readFileSync(path.join(ownerDir, "owner.key"), "utf8").trim();
const O = getPublicKey(ownerSecret);

const db = new ToolDb({ relays: [relay], topic: "toolnet" });
await db.ready;
await db.keysSignIn(ownerSecret, "owner");
await new Promise((r) => setTimeout(r, 500));

const get = (author, key) => db.getData(`:${author}.${key}`, false, 4000);
const fail = (msg) => {
  console.error("FAIL:", msg);
  process.exit(1);
};

const env = await get(O, `net-${net}`);
if (!env) fail("JS can't read the network written by Rust");
const gk = await get(O, `gk-${net}-${O}`);
if (!gk || gk.epoch !== env.epoch) fail("JS can't read the owner's group key");
const group = await unwrapGroupKey(db.userAccount, gk.key, O);
const config = JSON.parse(decryptForGroup(group, env.data));
console.log(`JS read network "${config.name}" epoch ${env.epoch}:`, Object.values(config.members).map((m) => `${m.name}=${m.ip}`).join(", "));

let nodes = 0;
for (const device of Object.keys(config.members)) {
  const node = await get(device, `node-${O}-${net}`);
  if (!node) continue;
  const info = JSON.parse(decryptForGroup(group, node.data));
  console.log(`JS read ${config.members[device].name}'s node info: endpoint ${info.endpointId.slice(0, 10)}…, addrs ${JSON.stringify(info.addrs)}`);
  nodes++;
}
if (nodes === 0) fail("JS couldn't read any node info written by Rust");

if (newDevice) {
  config.members[newDevice] = { name: newName, ip: newIp, addedAt: Math.floor(Date.now() / 1000) };
  config.updatedAt = Math.floor(Date.now() / 1000);
  const wrapped = await wrapGroupKey(db.userAccount, group, newDevice);
  await db.putData(`gk-${net}-${newDevice}`, { v: 1, epoch: env.epoch, key: wrapped }, true);
  await db.putData(`net-${net}`, { v: 1, epoch: env.epoch, data: encryptForGroup(group, JSON.stringify(config)) }, true);
  console.log(`JS approved ${newName} at ${newIp}`);
}
await new Promise((r) => setTimeout(r, 1000));
console.log("COMPAT OK");
process.exit(0);
