// Pairing links (docs/remote-protocol.md, Pairing link and Relay). Kept free of React Native
// imports so the tests run in node.
//
//   rift-remote://pair?a=192.168.1.29:7878,10.0.0.5:7878&k=<key>&s=<secret>&n=studio-mac&p=operate
//   …&r=wss%3A%2F%2Frelay.example.com   (the gateway's relay, tried after the addresses)

import { fromBase64Url } from "./noise/bytes";
import type { Permission } from "./protocol/types";

export const PAIR_SCHEME = "rift-remote";
export const MAX_ADDRESSES = 16;
export const MAX_HOST_NAME = 128;
export const MAX_RELAY_URL = 512;

export interface PairLink {
  /** `host:port` addresses to try in order. */
  addresses: string[];
  /** The gateway static public key to pin (base64url, 32 bytes). */
  key: string;
  /** The one-time pairing secret (base64url, 16 bytes). */
  secret: string;
  /** Host name to show. */
  name: string;
  permission: Permission;
  /** The relay URL (`ws://` or `wss://`, no trailing slash); null when the gateway has none. */
  relay: string | null;
}

/** Checks `host:port`: an IPv4 address, a DNS name or a bracketed IPv6 address, port 1–65535. */
export function isAddress(text: string): boolean {
  const m = /^(\[[0-9a-fA-F:.]+\]|[A-Za-z0-9](?:[A-Za-z0-9.-]{0,251}[A-Za-z0-9])?):(\d{1,5})$/.exec(text);
  if (!m) return false;
  const port = Number(m[2]);
  return port >= 1 && port <= 65535;
}

/**
 * A relay URL in canonical form, or null when `text` is not one: `ws://` or `wss://`, a DNS
 * name, IPv4 or bracketed IPv6 host, an optional port and path; no credentials, query or
 * fragment. Trailing slashes are dropped so `<relay>/connect` is well formed.
 */
export function normalizeRelayUrl(text: string): string | null {
  if (text.length === 0 || text.length > MAX_RELAY_URL) return null;
  const m =
    /^(wss?):\/\/(\[[0-9a-fA-F:.]+\]|[A-Za-z0-9](?:[A-Za-z0-9.-]{0,251}[A-Za-z0-9])?)(?::(\d{1,5}))?((?:\/[A-Za-z0-9._~!$&'()*+,;=:@%-]*)*)$/i.exec(
      text,
    );
  if (!m) return null;
  if (m[3] !== undefined) {
    const port = Number(m[3]);
    if (port < 1 || port > 65535) return null;
  }
  const path = m[4]!.replace(/\/+$/, "");
  return `${m[1]!.toLowerCase()}://${m[2]!.toLowerCase()}${m[3] !== undefined ? `:${m[3]}` : ""}${path}`;
}

/** Decodes %XX escapes and `+` (form encoding); null on a malformed escape. */
function decodeParam(value: string): string | null {
  try {
    return decodeURIComponent(value.replace(/\+/g, " "));
  } catch {
    return null;
  }
}

/** Parses a pairing link; null when it is not one or any field is malformed. */
export function parsePairLink(text: string): PairLink | null {
  const m = /^rift-remote:\/\/pair\/?\?(.*)$/i.exec(text.trim());
  if (!m) return null;
  const params = new Map<string, string>();
  for (const part of m[1]!.split("&")) {
    if (part === "") continue;
    const eq = part.indexOf("=");
    if (eq <= 0) return null;
    const key = part.slice(0, eq);
    const value = decodeParam(part.slice(eq + 1));
    if (value === null || params.has(key)) return null; // malformed or repeated
    params.set(key, value);
  }
  const a = params.get("a");
  const k = params.get("k");
  const s = params.get("s");
  const n = params.get("n");
  const p = params.get("p");
  const r = params.get("r");
  if (a === undefined || k === undefined || s === undefined || n === undefined || p === undefined) return null;

  const addresses = a.split(",");
  if (addresses.length === 0 || addresses.length > MAX_ADDRESSES || !addresses.every(isAddress)) return null;
  if (fromBase64Url(k)?.length !== 32) return null;
  if (fromBase64Url(s)?.length !== 16) return null;
  const name = n.trim();
  // eslint-disable-next-line no-control-regex
  if (name === "" || name.length > MAX_HOST_NAME || /[\u0000-\u001f\u007f]/.test(name)) return null;
  if (p !== "operate" && p !== "view") return null;
  let relay: string | null = null;
  if (r !== undefined) {
    relay = normalizeRelayUrl(r);
    if (relay === null) return null;
  }
  return { addresses, key: k, secret: s, name, permission: p, relay };
}

/** The canonical link text for `link` (round-trips through parsePairLink). */
export function formatPairLink(link: PairLink): string {
  const q = [
    `a=${link.addresses.join(",")}`,
    `k=${link.key}`,
    `s=${link.secret}`,
    `n=${encodeURIComponent(link.name)}`,
    `p=${link.permission}`,
  ];
  if (link.relay !== null) q.push(`r=${encodeURIComponent(link.relay)}`);
  return `${PAIR_SCHEME}://pair?${q.join("&")}`;
}
