import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const mobile = fileURLToPath(new URL("..", import.meta.url));
const src = join(mobile, "src");

function sources(dir: string): string[] {
  return readdirSync(dir).flatMap((n) => {
    const p = join(dir, n);
    return statSync(p).isDirectory() ? sources(p) : /\.tsx?$/.test(n) ? [p] : [];
  });
}

/** The faces theme.ts names (FONT_FACES), read from the source (it imports react-native). */
const theme = readFileSync(join(src, "ui/theme.ts"), "utf8");
const faces = [...(/FONT_FACES = \{([^}]*)\}/.exec(theme)?.[1] ?? "").matchAll(/"([^"]+)"/g)].map((m) => m[1]!);

/** `name: { … }` entries of a file's style sheets (`makeStyles((colors) => ({ … }))`, or `StyleSheet.create`). */
function styleEntries(text: string): Map<string, string> {
  const out = new Map<string, string>();
  for (const block of text.matchAll(/(?:StyleSheet\.create\(\{|makeStyles\(\(\w+\) => \(\{)([\s\S]*?)\n\}\)\)?;/g))
    for (const m of block[1]!.matchAll(/^\s{2}(\w+):\s*(\{[^{}]*(?:\{[^{}]*\}[^{}]*)*\})/gm)) out.set(m[1]!, m[2]!);
  return out;
}

describe("fonts (the mockups' Inter and JetBrains Mono, embedded)", () => {
  const app = JSON.parse(readFileSync(join(mobile, "app.json"), "utf8")) as { expo: { plugins: unknown[] } };
  const plugin = app.expo.plugins.find((p) => Array.isArray(p) && p[0] === "expo-font") as [string, { fonts: string[] }] | undefined;
  const embedded = plugin?.[1].fonts ?? [];

  it("embeds every face theme.ts names, each file present, named as its family", () => {
    expect(faces).toEqual(["Inter-Regular", "Inter-Medium", "Inter-SemiBold", "JetBrainsMono-Regular", "JetBrainsMono-Medium", "JetBrainsMono-Bold"]);
    for (const face of faces) {
      const file = `./assets/fonts/${face}.ttf`;
      expect(embedded).toContain(file);
      expect(existsSync(join(mobile, file))).toBe(true);
    }
    expect(embedded).toHaveLength(faces.length);
  });

  it("no style sets fontWeight: Android takes no weight from a custom family, the family is the weight", () => {
    const offenders = sources(src).filter((f) => !f.endsWith("theme.ts") && /\bfontWeight\s*:/.test(readFileSync(f, "utf8")));
    expect(offenders).toEqual([]);
  });

  it("every style with a font size names its family", () => {
    const missing: string[] = [];
    for (const f of sources(src))
      for (const [name, body] of styleEntries(readFileSync(f, "utf8")))
        if (/\bfontSize\s*:/.test(body) && !/\bfontFamily\s*:/.test(body)) missing.push(`${f.slice(src.length)}: ${name}`);
    expect(missing).toEqual([]);
  });

  it("every outermost Text and TextInput gets a family from its styles (nested Text inherits)", () => {
    const missing: string[] = [];
    // ui/Marked.tsx draws only spans its callers nest in their Text (checked below: every <Marked
    // sits inside a Text) or in a field (SelectableText's children)
    for (const f of sources(src).filter((p) => p.endsWith(".tsx") && !p.endsWith("/ui/Marked.tsx"))) {
      const text = readFileSync(f, "utf8");
      const entries = styleEntries(text);
      // walk the Text tags in order, tracking how deep inside a Text each one is
      let depth = 0;
      for (const m of text.matchAll(/<(\/?)(Text|TextInput|Marked)\b/g)) {
        const [, close, tag] = m;
        if (tag === "Marked") {
          if (!close && depth === 0) missing.push(`${f.slice(src.length)}:${text.slice(0, m.index).split("\n").length} <Marked> outside a Text`);
          continue;
        }
        if (close) {
          if (tag === "Text") depth--;
          continue;
        }
        // the tag's attributes: up to the ">" outside any {…}
        let i = m.index + m[0].length;
        let braces = 0;
        for (; i < text.length; i++) {
          const c = text[i];
          if (c === "{") braces++;
          else if (c === "}") braces--;
          else if (c === ">" && braces === 0) break;
        }
        const attrs = text.slice(m.index + m[0].length, i);
        const selfClose = attrs.trimEnd().endsWith("/");
        const nested = tag === "Text" && depth > 0;
        if (tag === "Text" && !selfClose) depth++;
        if (nested) continue;
        // the style={…} value, braces balanced
        let style = "";
        const at = attrs.indexOf("style={");
        if (at !== -1) {
          let d = 0;
          for (let j = at + 6; j < attrs.length; j++) {
            if (attrs[j] === "{") d++;
            else if (attrs[j] === "}" && --d === 0) {
              style = attrs.slice(at + 7, j);
              break;
            }
          }
        }
        const named = [...style.matchAll(/styles\.(\w+)/g)].map((s) => entries.get(s[1]!) ?? "");
        const ok = /fontFamily/.test(style) || named.some((b) => /fontFamily/.test(b));
        if (!ok) missing.push(`${f.slice(src.length)}:${text.slice(0, m.index).split("\n").length} <${tag}>`);
      }
    }
    expect(missing).toEqual([]);
  });
});
