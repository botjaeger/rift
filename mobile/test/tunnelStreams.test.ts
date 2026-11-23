import { describe, expect, it } from "vitest";
import { MAX_DATA, WINDOW, type Frame } from "../src/tunnel/frames";
import { Streams, type LocalSide } from "../src/tunnel/streams";

function setup(opts: { failWrites?: boolean } = {}) {
  const sent: Frame[] = [];
  const written: [number, Uint8Array][] = [];
  const closed: number[] = [];
  const local: LocalSide = {
    write: (stream, data) => {
      if (opts.failWrites) return Promise.reject(new Error("gone"));
      written.push([stream, data]);
      return Promise.resolve();
    },
    close: (stream) => {
      closed.push(stream);
    },
  };
  const streams = new Streams(5173, (f) => (sent.push(f), true), local);
  return { streams, sent, written, closed };
}

const bytes = (n: number, v = 1) => new Uint8Array(n).fill(v);
const flushPromises = () => new Promise((r) => setTimeout(r, 0));

describe("tunnel streams", () => {
  it("opens the port, holds the request until opened, then relays both ways with acks", async () => {
    const { streams, sent, written, closed } = setup();
    streams.localOpen(3);
    streams.localData(3, new TextEncoder().encode("GET / HTTP/1.1\r\n\r\n"));
    expect(sent).toEqual([{ op: "open", stream: 3, port: 5173 }]);
    streams.onFrame({ op: "opened", stream: 3 });
    expect(sent[1]).toMatchObject({ op: "data", stream: 3 });
    streams.onFrame({ op: "data", stream: 3, data: bytes(10, 7) });
    streams.onFrame({ op: "end", stream: 3 });
    await flushPromises();
    expect(written).toEqual([[3, bytes(10, 7)]]);
    expect(sent[2]).toEqual({ op: "ack", stream: 3, bytes: 10 });
    // the reply is whole: the browser's connection closes, then reports it
    expect(closed).toEqual([3]);
    streams.localClose(3);
    expect(sent[3]).toEqual({ op: "end", stream: 3 });
    expect(streams.size).toBe(0);
  });

  it("keeps the browser's bytes within the window until acknowledged", () => {
    const { streams, sent } = setup();
    streams.localOpen(1);
    streams.onFrame({ op: "opened", stream: 1 });
    streams.localData(1, bytes(WINDOW + 5000));
    const data = () => sent.filter((f) => f.op === "data") as Extract<Frame, { op: "data" }>[];
    const inFlight = () => data().reduce((n, f) => n + f.data.length, 0);
    expect(inFlight()).toBe(WINDOW);
    expect(data().every((f) => f.data.length <= MAX_DATA)).toBe(true);
    streams.onFrame({ op: "ack", stream: 1, bytes: 3000 });
    expect(inFlight()).toBe(WINDOW + 3000);
    streams.onFrame({ op: "ack", stream: 1, bytes: WINDOW });
    expect(inFlight()).toBe(WINDOW + 5000);
  });

  it("an end from the browser waits for what is held", () => {
    const { streams, sent } = setup();
    streams.localOpen(1);
    streams.onFrame({ op: "opened", stream: 1 });
    streams.localData(1, bytes(WINDOW + 1));
    streams.localClose(1);
    expect(sent.some((f) => f.op === "end")).toBe(false);
    streams.onFrame({ op: "ack", stream: 1, bytes: 1 });
    expect(sent.at(-1)).toEqual({ op: "end", stream: 1 });
  });

  it("a browser that half-closes before opened still gets its reply (nc sends, then EOF)", async () => {
    const { streams, sent, written } = setup();
    streams.localOpen(4);
    streams.localData(4, new TextEncoder().encode("GET / HTTP/1.0\r\n\r\n"));
    streams.localEnd(4);
    expect(sent.map((f) => f.op)).toEqual(["open"]);
    streams.onFrame({ op: "opened", stream: 4 });
    expect(sent.map((f) => f.op)).toEqual(["open", "data", "end"]);
    streams.onFrame({ op: "data", stream: 4, data: bytes(3) });
    await flushPromises();
    expect(written).toEqual([[4, bytes(3)]]);
  });

  it("a refused port closes the browser's connection; a close before opened aborts", () => {
    const { streams, sent, closed } = setup();
    streams.localOpen(1);
    streams.onFrame({ op: "reset", stream: 1, reason: "refused" });
    expect(closed).toEqual([1]);
    expect(streams.size).toBe(0);
    streams.localOpen(2);
    streams.localClose(2);
    expect(sent.at(-1)).toEqual({ op: "reset", stream: 2, reason: "aborted" });
    expect(streams.size).toBe(0);
  });

  it("a browser connection that cannot be written resets its stream, others go on", async () => {
    const { streams, sent, closed } = setup({ failWrites: true });
    streams.localOpen(1);
    streams.localOpen(2);
    streams.onFrame({ op: "opened", stream: 1 });
    streams.onFrame({ op: "opened", stream: 2 });
    streams.onFrame({ op: "data", stream: 1, data: bytes(4) });
    await flushPromises();
    expect(sent.at(-1)).toEqual({ op: "reset", stream: 1, reason: "error" });
    expect(closed).toEqual([1]);
    expect(streams.size).toBe(1);
  });

  it("frames for unknown streams are ignored; closeAll closes every connection", () => {
    const { streams, closed } = setup();
    streams.onFrame({ op: "data", stream: 99, data: bytes(1) });
    streams.localOpen(1);
    streams.localOpen(2);
    streams.closeAll();
    expect(closed.sort()).toEqual([1, 2]);
    expect(streams.size).toBe(0);
  });

  it("a browser flooding a stream past what may be held is reset", () => {
    const { streams, sent, closed } = setup();
    streams.localOpen(1);
    for (let i = 0; i < 5; i++) streams.localData(1, bytes(1024 * 1024));
    expect(sent.at(-1)).toEqual({ op: "reset", stream: 1, reason: "aborted" });
    expect(closed).toEqual([1]);
  });
});
