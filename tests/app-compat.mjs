// The owner app's data logic (app/src/lib) against the Rust daemon.
//   node tests/app-compat.mjs selftest
//   node tests/app-compat.mjs <relay url> <owner state dir> <net> <command> [args]
// Commands:
//   approve <qr payload | npub | hex> [name] [ip]
//   routes <member> [cidr...]
//   remove <member>
//   show                       prints the owner's view, fails without node info
//   expect-epoch <n>           fails unless the network is at epoch n
//   not-member <device state dir>   fails if that device can open the network
// Needs app/node_modules (cd app && npm install).
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";
import {
  approveDevice,
  connect,
  createNetwork,
  listNetworks,
  loadNetwork,
  removeDevice,
  setRoutes,
  gkKey,
  netKey,
} from "../app/src/lib/net.js";
import { isExitRoute, nextFreeIp, parseCidr, parseJoin, checkMemberIp } from "../app/src/lib/parse.js";

// The app's own tool-db (npm), the one net.js uses
const appDir = path.resolve(import.meta.dirname, "../app");
const { ToolDb, unwrapGroupKey, getPublicKey, npubEncode } = await import(
  pathToFileURL(path.join(appDir, "node_modules/tool-db/dist/index.js")).href
);

const fail = (msg) => {
  console.error("FAIL:", msg);
  process.exit(1);
};

function selftest() {
  // Cidr::from_str
  assert.equal(parseCidr("192.168.1.7/24"), "192.168.1.0/24");
  assert.equal(parseCidr("10.0.0.5"), "10.0.0.5/32");
  assert.equal(parseCidr("0.0.0.0/0"), "0.0.0.0/0");
  assert.equal(parseCidr("8.8.8.8/0"), "0.0.0.0/0");
  assert.equal(parseCidr("10.1.2.3/8"), "10.0.0.0/8");
  for (const bad of ["10.0.0.0/33", "10.0.0/8", "10.0.0.256/8", "010.0.0.0/8", "fd00::/8", "10.0.0.0/", "10.0.0.0/-1", ""]) {
    assert.throws(() => parseCidr(bad), undefined, bad);
  }
  assert.ok(isExitRoute("0.0.0.0/0"));

  // next_free_ip
  const config = { ipv4: "10.213.0.0/24", members: {} };
  assert.equal(nextFreeIp(config), "10.213.0.1");
  config.members.a = { name: "a", ip: "10.213.0.1" };
  config.members.c = { name: "c", ip: "10.213.0.3" };
  assert.equal(nextFreeIp(config), "10.213.0.2");
  assert.equal(nextFreeIp({ ipv4: "10.213.0.77/24", members: {} }), "10.213.0.1");
  assert.throws(() => nextFreeIp({ ipv4: "10.0.0.0/31", members: {} }));
  assert.throws(() => checkMemberIp(config, "10.213.0.3"));
  assert.throws(() => checkMemberIp(config, "10.213.0.255"));
  assert.throws(() => checkMemberIp(config, "10.214.0.9"));
  assert.equal(checkMemberIp(config, "10.213.0.50"), "10.213.0.50");

  // Join QR payloads
  const hex = getPublicKey("1".repeat(64));
  const n = npubEncode(hex);
  assert.deepEqual(parseJoin(`toolnet:join?device=${n}&name=laptop`), { device: hex, name: "laptop" });
  assert.deepEqual(parseJoin(`toolnet:join?name=my%20pc&device=${n}`), { device: hex, name: "my pc" });
  assert.deepEqual(parseJoin(` ${n}\n`), { device: hex, name: "" });
  assert.deepEqual(parseJoin(`nostr:${n}`), { device: hex, name: "" });
  assert.deepEqual(parseJoin(hex.toUpperCase()), { device: hex, name: "" });
  assert.throws(() => parseJoin("toolnet:join?name=x"));
  assert.throws(() => parseJoin(n.slice(0, -1) + (n.endsWith("q") ? "p" : "q")));
  assert.throws(() => parseJoin("nsec1abc"));
  console.log("selftest OK");
}

const [relayOrCmd, ownerDir, net, cmd, ...args] = process.argv.slice(2);
if (relayOrCmd === "selftest") {
  selftest();
  process.exit(0);
}

const ownerSecret = fs.readFileSync(path.join(ownerDir, "owner.key"), "utf8").trim();
const db = await connect(ownerSecret, [relayOrCmd]);
if (!db.isConnected) fail("not connected to the relay");

const describe = (view) =>
  Object.entries(view.config.members)
    .map(([k, m]) => `${m.name}=${m.ip}${m.routes ? ` routes=${m.routes.join(",")}` : ""}${view.nodes[k] ? ` [${view.nodes[k].hostname} v${view.nodes[k].version}]` : ""}`)
    .join(", ");

try {
  switch (cmd) {
    case "approve": {
      const [payload, name, ip] = args;
      const join = parseJoin(payload);
      const done = await approveDevice(db, net, { device: join.device, name: name || join.name, ip });
      console.log(`JS approved ${done.name} at ${done.ip}`);
      break;
    }
    case "routes": {
      const [member, ...routes] = args;
      const set = await setRoutes(db, net, member, routes);
      console.log(`JS set routes of ${member}: ${JSON.stringify(set)}`);
      break;
    }
    case "remove": {
      const before = await loadNetwork(db, net, { withNodes: false });
      const removed = await removeDevice(db, net, args[0]);
      console.log(`JS removed ${removed.name} (${removed.ip}); epoch ${before.epoch} -> ${removed.epoch}`);
      // The removed device must not have the new group key
      const O = db.userAccount.getAddress();
      const theirs = await db.getData(`:${O}.${gkKey(net, removed.device)}`, false, 4000);
      if (theirs && theirs.epoch === removed.epoch) fail("the removed device got the new group key");
      const after = await loadNetwork(db, net, { withNodes: false });
      if (after.status !== "ok" || after.epoch !== removed.epoch) fail("network not at the new epoch");
      if (after.group === before.group) fail("group key not rotated");
      if (after.config.members[removed.device]) fail("removed device still listed");
      for (const member of Object.keys(after.config.members)) {
        const gk = await db.getData(`:${O}.${gkKey(net, member)}`, false, 4000);
        if (gk?.epoch !== removed.epoch) fail(`member ${member} has no key for epoch ${removed.epoch}`);
      }
      break;
    }
    case "show": {
      const view = await loadNetwork(db, net);
      if (view.status !== "ok") fail(`network status ${view.status}`);
      console.log(`JS view: epoch ${view.epoch}, ${describe(view)}`);
      const want = Number(args[0] || 0);
      if (Object.keys(view.nodes).length < want) fail(`expected node info of ${want} members`);
      break;
    }
    case "list": {
      // The app's network list: Rust's network, plus one the app creates
      await createNetwork(db, "lab", { ipv4: "10.214.0.0/24" });
      const nets = await listNetworks(db);
      console.log(`JS networks: ${nets.map((n) => `${n.name}(${n.ipv4}, ${n.members} devices, epoch ${n.epoch})`).join(", ")}`);
      const names = nets.map((n) => n.name).join(",");
      const want = ["lab", net].sort().join(",");
      if (names !== want) fail(`expected networks ${want}, got ${names}`);
      if (!nets.every((n) => n.status === "ok")) fail("a network the owner can't open");
      try {
        await createNetwork(db, "Bad.Name");
        fail("accepted an invalid network name");
      } catch (e) {
        if (!/lowercase/.test(e.message)) throw e;
      }
      break;
    }
    case "expect-epoch": {
      const view = await loadNetwork(db, net, { withNodes: false });
      if (view.epoch !== Number(args[0])) fail(`epoch ${view.epoch}, expected ${args[0]}`);
      console.log(`JS: epoch ${view.epoch}`);
      break;
    }
    case "not-member": {
      // Open the network with the removed device's own key, like model::view
      const secret = fs.readFileSync(path.join(args[0], "device.key"), "utf8").trim();
      const O = db.userAccount.getAddress();
      const dev = new ToolDb({ relays: [relayOrCmd], topic: "toolnet" });
      await dev.ready;
      await dev.keysSignIn(secret, "device");
      await new Promise((r) => setTimeout(r, 500));
      const env = await dev.getData(`:${O}.${netKey(net)}`, false, 4000);
      const gk = await dev.getData(`:${O}.${gkKey(net, getPublicKey(secret))}`, false, 4000);
      if (gk && gk.epoch === env.epoch) fail("removed device has a key for the current epoch");
      if (gk) {
        const old = await unwrapGroupKey(dev.userAccount, gk.key, O);
        const view = await loadNetwork(db, net, { withNodes: false });
        if (old === view.group) fail("removed device's old key is the current group key");
      }
      console.log("JS: removed device can't open the current network");
      await dev.close();
      break;
    }
    default:
      fail(`unknown command ${cmd}`);
  }
} catch (e) {
  fail(e.message);
}
await new Promise((r) => setTimeout(r, 500));
await db.close();
process.exit(0);
