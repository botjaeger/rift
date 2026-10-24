import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { runInNewContext } from "node:vm";
import { describe, expect, it } from "vitest";

// Follow the actual router -> query-string dependency, including pnpm's patch.
const require = createRequire(import.meta.url);
const router = createRequire(require.resolve("expo-router/package.json"));
const query = createRequire(router.resolve("query-string/package.json"));
const source = readFileSync(query.resolve("decode-uri-component"), "utf8");

function decoder() {
  let calls = 0;
  const module = { exports: undefined as unknown };
  runInNewContext(source, {
    module,
    decodeURIComponent: (text: string) => { calls++; return decodeURIComponent(text); },
  });
  return { decode: module.exports as (text: string) => string, calls: () => calls };
}

describe("router query decoder security", () => {
  it("bounds malformed input while preserving router decoding compatibility", () => {
    const d = decoder();
    const text = "%FF".repeat(40);
    expect(d.decode(text)).toBe(text);
    expect(d.calls()).toBeLessThanOrEqual(40 * 4 + 10);
    expect(d.decode("hello+world%20%C3%A9")).toBe("hello world é");
    expect(d.decode("%FF%C3%A9-end%FE%FF")).toBe("%FFé-end��");
    expect(d.decode("%E0%A4%A")).toBe("%E0%A4%A");
    const parse = query("query-string").parse as (text: string) => Record<string, string>;
    expect(parse("name=hello+world&value=%FF%C3%A9")).toMatchObject({ name: "hello world", value: "%FFé" });
  });
});
