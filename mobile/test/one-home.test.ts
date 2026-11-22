import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

// One home per constant: a value two files spell for themselves drifts apart (a "Copied" that
// stayed 1600 ms on one button and 1200 ms on a selection; the handshake prologue written out
// again by the bench). A constant shared between files is exported from one and imported.
const SRC = fileURLToPath(new URL("../src", import.meta.url));

function files(dir: string): string[] {
  return readdirSync(dir).flatMap((n) => {
    const p = join(dir, n);
    return statSync(p).isDirectory() ? files(p) : /\.(ts|tsx)$/.test(n) && !n.endsWith(".data.ts") ? [p] : [];
  });
}

describe("one home per constant", () => {
  it("no UPPER_CASE constant is declared in two source files", () => {
    const where = new Map<string, string[]>();
    for (const f of files(SRC)) {
      for (const m of readFileSync(f, "utf8").matchAll(/^(?:export )?const ([A-Z][A-Z0-9_]*) = /gm)) {
        where.set(m[1]!, [...(where.get(m[1]!) ?? []), f.slice(SRC.length + 1)]);
      }
    }
    const twins = [...where].filter(([, at]) => at.length > 1);
    expect(twins).toEqual([]);
  });
});
