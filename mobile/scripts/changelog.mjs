// Embeds CHANGELOG.md in the app as src/changelog.data.ts (Metro does not import Markdown).
// Runs before every `pnpm ota`; test/changelog.test.ts fails while the two differ.
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const md = readFileSync(join(root, "CHANGELOG.md"), "utf8");
writeFileSync(
  join(root, "src/changelog.data.ts"),
  `// Generated from CHANGELOG.md by scripts/changelog.mjs (pnpm changelog); do not edit.\nexport const CHANGELOG = ${JSON.stringify(md)};\n`,
);
