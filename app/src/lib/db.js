import { ToolDbIndexedDb } from "@tool-db/indexeddb-store";
import { connect as connectNet } from "./net.js";
import { getRelays } from "./session.js";

/**
 * Connects as the owner, keeping what we saw in IndexedDB
 */
export const connect = (secretKey) =>
  connectNet(secretKey, getRelays(), { storageAdapter: ToolDbIndexedDb, storageName: "toolnet" });
