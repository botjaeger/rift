// Connection paths to a gateway (docs/remote-protocol.md, Relay): its LAN addresses, tried
// first, then the relay, which splices the WebSocket to the gateway and only sees ciphertext.

import { sha256 } from "@noble/hashes/sha2.js";
import { toBase64Url } from "../noise/bytes";

export type Endpoint = { kind: "lan"; address: string } | { kind: "relay"; url: string };

/** The gateway's name at the relay: base64url of the first 16 bytes of SHA-256(static key). */
export function routingId(gatewayKey: Uint8Array): string {
  return toBase64Url(sha256(gatewayKey).slice(0, 16));
}

/** `<relay>/connect?id=<routing id>`; `relay` is a normalized relay URL (no trailing slash). */
export function relayConnectUrl(relay: string, gatewayKey: Uint8Array): string {
  return `${relay}/connect?id=${routingId(gatewayKey)}`;
}

/** The host part of a relay URL (`wss://relay.example.com:443/x` → `relay.example.com`). */
export function relayHost(url: string): string {
  const m = /^wss?:\/\/(\[[^\]]+\]|[^/:]+)/i.exec(url);
  return m ? m[1]! : url;
}

/** The host part of `host:port` (`192.168.1.29:7878` → `192.168.1.29`, `[fe80::1]:1` → `fe80::1`). */
export function addressHost(address: string): string {
  const i = address.lastIndexOf(":");
  const host = i === -1 ? address : address.slice(0, i);
  return host.startsWith("[") && host.endsWith("]") ? host.slice(1, -1) : host;
}

/** "LAN · 192.168.1.29" or "Relay · relay.example.com". */
export function pathLabel(path: Endpoint): string {
  return path.kind === "lan" ? `LAN · ${addressHost(path.address)}` : `Relay · ${relayHost(path.url)}`;
}

/** Every path in trying order: the LAN addresses, then the relay. */
export function endpointsFor(addresses: string[], relay: string | null): Endpoint[] {
  const out: Endpoint[] = addresses.map((address) => ({ kind: "lan", address }));
  if (relay) out.push({ kind: "relay", url: relay });
  return out;
}
