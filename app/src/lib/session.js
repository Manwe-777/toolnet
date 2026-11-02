import { getPublicKey, nsecEncode, normalizeSecretKey } from "tool-db";
import { DEFAULT_RELAYS } from "./net.js";

const KEY = "toolnet-owner-key";
const RELAYS_KEY = "toolnet-relays";
const NET_KEY = "toolnet-net";

function read(key) {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

/**
 * The owner's secret key (hex), kept in this browser
 */
export function getSecretKey() {
  return read(KEY);
}

/**
 * Saves the owner key: nsec, or hex like the CLI's owner.key
 * @returns the hex secret key
 */
export function saveSecretKey(secretKey) {
  const cleaned = secretKey.replace(/\s+/g, "");
  const hex = normalizeSecretKey(cleaned.startsWith("nsec1") ? cleaned : cleaned.toLowerCase());
  getPublicKey(hex); // throws on an invalid key
  localStorage.setItem(KEY, hex);
  return hex;
}

export function forgetSecretKey() {
  localStorage.removeItem(KEY);
}

export const exportSecretKey = (hex) => nsecEncode(hex);

export function getRelays() {
  try {
    const list = JSON.parse(read(RELAYS_KEY));
    return Array.isArray(list) && list.length ? list : DEFAULT_RELAYS;
  } catch {
    return DEFAULT_RELAYS;
  }
}

export function saveRelays(relays) {
  localStorage.setItem(RELAYS_KEY, JSON.stringify(relays));
}

/** The network we manage, "home" like the CLI's default */
export const getNet = () => read(NET_KEY) || "home";

export function saveNet(net) {
  localStorage.setItem(NET_KEY, net);
}
