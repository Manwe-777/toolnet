// A local ToolDB relay for tests: validates and stores toolnet's events the
// way a real ToolDB relay node would.
//   node tests/local-relay.mjs [port] [host]
// Needs a built tool-db checkout next to this repo (TOOLDB_DIR to override).
import path from "node:path";
import { pathToFileURL } from "node:url";

const toolDb = process.env.TOOLDB_DIR ?? path.resolve(import.meta.dirname, "../../tool-db");
const { ToolDbRelay } = await import(
  pathToFileURL(path.join(toolDb, "packages/nostr-relay/dist/index.js")).href
);
const port = Number(process.argv[2] ?? 7791);
const host = process.argv[3] ?? "127.0.0.1";
const relay = new ToolDbRelay({ topic: "toolnet", port, host });
await relay.start();
console.log(`relay listening on ws://${host}:${port}`);
