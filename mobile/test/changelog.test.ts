import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { CHANGELOG } from "../src/changelog.data";
import { parseChangelog, plain, releaseKey, releaseName, releasesSince } from "../src/changelog";
import { APP_UPDATE } from "../src/release";

const md = `# Changelog

Intro text.

## 0.2.0 — 2026-11-01

A new build.

### New

- **Big thing** — does a lot,
  over two lines.

## 0.1.0 · update 2 — 2026-10-15

Short summary.

### Improved

- **Tab bar** — always there.

### Fixed

- **Gap** — gone.

## 0.1.0 — 2026-10-14

### New

- **Pairing** — scan a code.
`;

describe("parseChangelog", () => {
  it("leaves an Unreleased section out, wherever it stands", () => {
    const notes = "## Unreleased\n\nNext.\n\n### New\n\n- **Soon** — not yet.\n\n## 0.1.0 — 2026-10-14\n\n### New\n\n- **Pairing** — scan.\n\n## Unreleased\n\n### Fixed\n\n- **Later** — no.\n";
    const all = parseChangelog(notes);
    expect(all.map((r) => r.version)).toEqual(["0.1.0"]);
    expect(all[0]!.groups.map((g) => g.items.map((i) => i.title))).toEqual([["Pairing"]]);
  });

  it("reads releases, summaries, groups and wrapped items", () => {
    const all = parseChangelog(md);
    expect(all.map((r) => releaseName(r))).toEqual(["0.2.0", "0.1.0 · update 2", "0.1.0"]);
    expect(all[0]).toMatchObject({ date: "2026-11-01", summary: "A new build." });
    expect(all[0]!.groups[0]!.items[0]).toEqual({ title: "Big thing", text: "does a lot, over two lines." });
    expect(all[1]!.groups.map((g) => g.title)).toEqual(["Improved", "Fixed"]);
    expect(all[2]!.summary).toBeNull();
  });

  it("parses the app's own notes, and they are the embedded copy of CHANGELOG.md", () => {
    expect(CHANGELOG).toBe(readFileSync(join(__dirname, "../CHANGELOG.md"), "utf8")); // pnpm changelog
    const all = parseChangelog(CHANGELOG);
    expect(all.length).toBeGreaterThan(0);
    for (const r of all) expect(r.groups.flatMap((g) => g.items).length).toBeGreaterThan(0);
    // the newest section is the JavaScript that ships (release.ts and app.json, by /release)
    expect(all[0]!.update).toBe(APP_UPDATE);
    expect(all[0]!.version).toBe(JSON.parse(readFileSync(join(__dirname, "../app.json"), "utf8")).expo.version);
  });
});

describe("plain", () => {
  it("drops italics, keeps a lone star", () => {
    expect(plain("offers *Restart now* once")).toBe("offers Restart now once");
    expect(plain("rm * here")).toBe("rm * here");
    expect(plain("a * b * c")).toBe("a * b * c");
    // the shipped notes show no stray stars
    for (const r of parseChangelog(CHANGELOG)) {
      for (const it of r.groups.flatMap((g) => g.items)) expect(it.text).not.toMatch(/\*[^\s*][^*]*\*/);
    }
  });
});

describe("releasesSince", () => {
  const all = parseChangelog(md);
  const k = (v: string, u: number) => releaseKey(v, u);

  it("lists every release after the one last seen, newest first", () => {
    expect(releasesSince(all, k("0.1.0", 0), k("0.2.0", 0)).map(releaseName)).toEqual(["0.2.0", "0.1.0 · update 2"]);
    expect(releasesSince(all, k("0.1.0", 2), k("0.2.0", 0)).map(releaseName)).toEqual(["0.2.0"]);
  });

  it("shows nothing on a first launch, without a change, or for a build not in the notes", () => {
    expect(releasesSince(all, null, k("0.2.0", 0))).toEqual([]);
    expect(releasesSince(all, k("0.2.0", 0), k("0.2.0", 0))).toEqual([]);
    expect(releasesSince(all, k("0.1.0", 0), k("9.9.9", 0))).toEqual([]);
  });

  it("shows only the current release when the one seen is unknown or newer", () => {
    expect(releasesSince(all, k("0.0.1", 0), k("0.1.0", 2)).map(releaseName)).toEqual(["0.1.0 · update 2"]);
    expect(releasesSince(all, k("0.2.0", 0), k("0.1.0", 2)).map(releaseName)).toEqual(["0.1.0 · update 2"]);
  });
});
