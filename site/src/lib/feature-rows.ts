
// The features page reads the README's Features table (synced into src/content/docs/features.md),
// so it never drifts from the docs. Each row is rendered to small, escaped HTML.
export interface FeatureRow {
  plain: string; // the feature's name as text, for grouping
  what: string; // HTML
  how: string; // HTML
}

const escape = (s: string) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');

// Key chords such as ⇧⌘C, ⌘↑, ⌃⇧1…9 or ⌥⇧F become <span class="kbd">; ⌘-click and ⌥⌘arrows stay text.
const CHORD = /([⌘⇧⌥⌃]+(?:[A-Z0-9,=.]|[↑↓←→⏎⌫]|Tab|Space)(?:…[0-9])?)(?![\w-])/g;

export function inline(md: string): string {
  return escape(md)
    .split(/(`[^`]*`)/)
    .map((part, i) =>
      i % 2
        ? `<code>${part.slice(1, -1)}</code>`
        : part
            .replace(/\*\*([^*]+)\*\*/g, '$1')
            .replace(/\*([^*]+)\*/g, '<em>$1</em>')
            .replace(CHORD, '<span class="kbd">$1</span>'),
    )
    .join('');
}

export function parseRows(body: string): FeatureRow[] {
  return body
    .split('\n')
    .filter((line) => line.startsWith('| ') && !line.startsWith('| Feature |'))
    .map((line) => {
      const cells = line.replace(/^\| /, '').replace(/ \|$/, '').split(' | ');
      if (cells.length !== 2) throw new Error(`features table: expected 2 cells in "${line}"`);
      const what = cells[0].replace(' — ', ': ');
      return { plain: what.replace(/[`*]/g, ''), what: inline(what), how: inline(cells[1]) };
    });
}


// Puts each row in the one group whose prefix its name starts with; any row left over, or a
// prefix that matches nothing, stops the build so a new README row is never silently dropped.
export function groupRows<G extends { prefixes: string[] }>(rows: FeatureRow[], groups: G[]): (G & { rows: FeatureRow[] })[] {
  const starts = (name: string, prefix: string) => name.startsWith(prefix) && !/[A-Za-z]/.test(name.charAt(prefix.length));
  const out = groups.map((g) => ({ ...g, rows: [] as FeatureRow[] }));
  for (const row of rows) {
    const hits = out.filter((g) => g.prefixes.some((p) => starts(row.plain, p)));
    if (hits.length !== 1) throw new Error(`features: "${row.plain}" matches ${hits.length} groups; edit the prefixes in src/pages/features.astro`);
    hits[0].rows.push(row);
  }
  for (const g of out)
    for (const p of g.prefixes)
      if (!rows.some((r) => starts(r.plain, p))) throw new Error(`features: prefix "${p}" matches no README row`);
  return out;
}
