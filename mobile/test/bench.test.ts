import { randomBytes as nodeRandom } from "node:crypto";
import { describe, expect, it } from "vitest";
import { runCryptoBench } from "../src/bench";

// Like expo-crypto, which refuses more than 1024 bytes per draw.
const random = (n: number) => {
  if (n > 1024) throw new Error(`draw of ${n} bytes`);
  return new Uint8Array(nodeRandom(n));
};

describe("crypto benchmark", () => {
  it("round-trips the payload and reports positive timings", () => {
    const r = runCryptoBench(random, () => performance.now(), 200_000);
    expect(r.bytes).toBe(200_000);
    expect(r.handshakeMs).toBeGreaterThan(0);
    expect(r.sealMs).toBeGreaterThan(0);
    expect(r.openMs).toBeGreaterThan(0);
    expect(r.mibPerSecond).toBeGreaterThan(0);
    expect(r.engine).toBe("JavaScript");
  });
});
