import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { formatPairLink, isAddress, normalizeRelayUrl, parsePairLink } from "../src/pairing";

const fixture = readFileSync(fileURLToPath(new URL("../../crates/rift-remote/fixtures/pairing-link.txt", import.meta.url)), "utf8");
const relayFixture = readFileSync(fileURLToPath(new URL("../../crates/rift-remote/fixtures/pairing-link-relay.txt", import.meta.url)), "utf8");
const K = "3q2-7wAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
const S = "q83vEjRWeJCrze8SNFZ4kA";
const link = (q: string) => `rift-remote://pair?${q}`;
const good = `a=192.168.1.29:7878&k=${K}&s=${S}&n=studio-mac&p=operate`;

describe("parsePairLink", () => {
  it("parses the shared fixture", () => {
    expect(parsePairLink(fixture)).toEqual({
      addresses: ["192.168.1.29:7878", "10.0.0.5:7878"],
      key: K,
      secret: S,
      name: "studio-mac",
      permission: "operate",
      relay: null,
    });
  });

  it("accepts view, any parameter order, percent-encoded names, IPv6 and DNS hosts, unknown params", () => {
    expect(parsePairLink(link(`p=view&n=Sam%27s%20Mac&s=${S}&k=${K}&a=[fe80::1]:7878,studio.local:9000&x=1`))).toEqual({
      addresses: ["[fe80::1]:7878", "studio.local:9000"],
      key: K,
      secret: S,
      name: "Sam's Mac",
      permission: "view",
      relay: null,
    });
    expect(parsePairLink(link(good.replace("studio-mac", "my+mac")))?.name).toBe("my mac");
    expect(parsePairLink(`  ${link(good)}\n`)).not.toBeNull();
  });

  it("round-trips through formatPairLink", () => {
    const p = parsePairLink(link(good.replace("studio-mac", "Sam%27s%20Mac")))!;
    expect(parsePairLink(formatPairLink(p))).toEqual(p);
  });

  it.each([
    ["empty", ""],
    ["another scheme", `rift://pair?${good}`],
    ["the v1 link", "rift-remote://192.168.1.20:7878?token=abc"],
    ["another path", `rift-remote://connect?${good}`],
    ["no query", "rift-remote://pair"],
    ["missing a", link(good.replace("a=192.168.1.29:7878&", ""))],
    ["missing k", link(good.replace(`k=${K}&`, ""))],
    ["missing s", link(good.replace(`s=${S}&`, ""))],
    ["missing n", link(good.replace("n=studio-mac&", ""))],
    ["missing p", link(good.replace("&p=operate", ""))],
    ["empty address list", link(good.replace("a=192.168.1.29:7878", "a="))],
    ["address without port", link(good.replace("192.168.1.29:7878", "192.168.1.29"))],
    ["port 0", link(good.replace(":7878", ":0"))],
    ["port 70000", link(good.replace(":7878", ":70000"))],
    ["address with a path", link(good.replace("192.168.1.29:7878", "host:7878/x"))],
    ["short key", link(good.replace(K, K.slice(0, 42)))],
    ["key with standard base64", link(good.replace(K, K.replace("-", "+")))],
    ["padded key", link(good.replace(K, `${K}=`))],
    ["15-byte secret", link(good.replace(S, "q83vEjRWeJCrze8SNFZ4"))],
    ["empty name", link(good.replace("n=studio-mac", "n="))],
    ["control character in name", link(good.replace("studio-mac", "a%0Ab"))],
    ["bad percent escape", link(good.replace("studio-mac", "a%zz"))],
    ["unknown permission", link(good.replace("p=operate", "p=admin"))],
    ["repeated parameter", link(`${good}&p=view`)],
    ["parameter without =", link(`${good}&junk`)],
  ])("rejects %s", (_name, text) => {
    expect(parsePairLink(text)).toBeNull();
  });
});

describe("isAddress", () => {
  it.each(["10.0.0.5:7878", "[::1]:7878", "studio.local:1", "a:65535"])("accepts %s", (a) => expect(isAddress(a)).toBe(true));
  it.each(["10.0.0.5", ":7878", "host:", "host:65536", "-host:1", "ho st:1", "host:1:2"])("rejects %s", (a) =>
    expect(isAddress(a)).toBe(false),
  );
});

describe("relay in pairing links (r=)", () => {
  it("parses the shared relay fixture", () => {
    expect(parsePairLink(relayFixture)).toEqual({
      addresses: ["192.168.1.29:7878"],
      key: K,
      secret: S,
      name: "studio-mac",
      permission: "operate",
      relay: "wss://relay.example.com",
    });
  });

  it("round-trips a relay through formatPairLink", () => {
    const p = parsePairLink(relayFixture)!;
    expect(formatPairLink(p)).toContain("r=wss%3A%2F%2Frelay.example.com");
    expect(parsePairLink(formatPairLink(p))).toEqual(p);
    const withPath = parsePairLink(link(`${good}&r=${encodeURIComponent("ws://10.0.0.9:8080/rift/")}`))!;
    expect(withPath.relay).toBe("ws://10.0.0.9:8080/rift");
    expect(parsePairLink(formatPairLink(withPath))).toEqual(withPath);
  });

  it.each([
    ["an https URL", "https://relay.example.com"],
    ["no scheme", "relay.example.com"],
    ["a query", "wss://relay.example.com/?x=1"],
    ["a fragment", "wss://relay.example.com/#x"],
    ["credentials", "wss://user:pw@relay.example.com"],
    ["port 0", "wss://relay.example.com:0"],
    ["an empty value", ""],
    ["a space", "wss://relay example.com"],
  ])("rejects a link whose relay has %s", (_name, r) => {
    expect(parsePairLink(link(`${good}&r=${encodeURIComponent(r)}`))).toBeNull();
  });

  it("rejects a repeated r", () => {
    expect(parsePairLink(link(`${good}&r=wss%3A%2F%2Fa.example&r=wss%3A%2F%2Fb.example`))).toBeNull();
  });

  it("normalizes: lower-case scheme and host, no trailing slash, path kept", () => {
    expect(normalizeRelayUrl("WSS://Relay.Example.com/")).toBe("wss://relay.example.com");
    expect(normalizeRelayUrl("wss://[2001:db8::1]:443/Path//")).toBe("wss://[2001:db8::1]:443/Path");
    expect(normalizeRelayUrl(`wss://a.example/${"x".repeat(600)}`)).toBeNull();
  });
});
