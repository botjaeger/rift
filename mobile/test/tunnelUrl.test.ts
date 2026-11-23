import { describe, expect, it } from "vitest";
import { base64ToBytes, bytesToBase64, localLinksIn, localUrl } from "../src/tunnel/url";

describe("which links open through the tunnel", () => {
  it.each([
    ["http://localhost:5173/", { port: 5173, path: "/" }],
    ["http://localhost:3000", { port: 3000, path: "/" }],
    ["http://127.0.0.1:8000/docs?x=1#top", { port: 8000, path: "/docs?x=1#top" }],
    ["http://0.0.0.0:4321/", { port: 4321, path: "/" }],
    ["http://[::1]:5000/a", { port: 5000, path: "/a" }],
    ["HTTP://LOCALHOST:8080/", { port: 8080, path: "/" }],
    ["https://localhost:8443/", { port: 8443, path: "/" }],
  ])("%s", (url, want) => {
    expect(localUrl(url)).toEqual(want);
  });

  it.each([
    "http://localhost/",
    "http://localhost:0/",
    "http://localhost:65536/",
    "http://example.com:3000/",
    "http://localhost.example.com:3000/",
    "http://192.168.1.4:3000/",
    "ftp://localhost:21/",
    "http://localhost:3000/ and more",
  ])("%s is not one", (url) => {
    expect(localUrl(url)).toBeNull();
  });
});

describe("the local pages a pane printed", () => {
  it("finds a dev server's links, one per port, newest first", () => {
    const vite = "  VITE v6.0.0  ready in 300 ms\n\n  ➜  Local:   http://localhost:5173/\n  ➜  Network: http://192.168.1.4:5173/\n";
    const next = "▲ Next.js 15\n- Local:        http://localhost:3000\n";
    const again = "served at http://127.0.0.1:5173/app, and http://localhost:8000/docs.";
    expect(localLinksIn([again, next, vite])).toEqual([
      { url: "http://localhost:8000/docs", port: 8000, path: "/docs" },
      { url: "http://127.0.0.1:5173/app", port: 5173, path: "/app" },
      { url: "http://localhost:3000", port: 3000, path: "/" },
    ]);
  });

  it("keeps to `max` and ignores other hosts", () => {
    const text = Array.from({ length: 9 }, (_, i) => `http://localhost:${4000 + i}/`).join(" ");
    expect(localLinksIn([text], 2).map((l) => l.port)).toEqual([4008, 4007]);
    expect(localLinksIn(["http://example.com:3000/ http://10.0.0.2:80/"])).toEqual([]);
  });
});

describe("base64 for the listener", () => {
  it("round-trips every byte value and large buffers", () => {
    const all = Uint8Array.from({ length: 256 }, (_, i) => i);
    expect(base64ToBytes(bytesToBase64(all))).toEqual(all);
    const big = new Uint8Array(100_000).map((_, i) => (i * 31) & 255);
    expect(base64ToBytes(bytesToBase64(big))).toEqual(big);
  });
});
