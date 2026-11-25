import { getCollection, render, type CollectionEntry } from 'astro:content';
import { url } from './url';

export type Doc = CollectionEntry<'docs'>;

export interface Section {
  page: string; // slug
  pageTitle: string;
  heading: string;
  anchor: string; // '' for the page's opening text
  text: string;
}

export async function getDocs(): Promise<Doc[]> {
  const docs = await getCollection('docs');
  // An empty collection would publish a docs page with nothing on it; fail loudly instead.
  // In `astro dev`, this also means the server started before src/content.config.ts existed: restart it.
  if (!docs.length) throw new Error('No docs found in src/content/docs. Run `pnpm docs:sync ../rift`, or restart `astro dev`.');
  return docs.sort((a, b) => a.data.order - b.data.order);
}

export const docUrl = (slug: string, anchor = '') => url(`/docs/${slug}/${anchor ? `#${anchor}` : ''}`);

// Markdown to the plain words a reader sees, for the search index.
function plain(md: string): string {
  return md
    .replace(/<\/?(a|br|div|span|kbd|img|p|sub|sup|details|summary|code|pre)\b[^>]*>/gi, ' ') // real tags, not <data dir>
    .replace(/!?\[([^\]]*)\]\([^)]*\)/g, '$1')
    .replace(/^\s*```.*$/gm, ' ')
    .replace(/^\s*[-+] /gm, ' ') // list markers
    .replace(/^\s*\|?\s*-{3,}.*$/gm, ' ')
    .replace(/[`*]/g, '')
    .replace(/\|/g, ' ')
    .replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&amp;/g, '&')
    .replace(/(^|\s)[>#]+(?=\s)/g, ' ')
    .replace(/(^|[\s(])_([^_\n]+)_(?=[\s.,;:)]|$)/g, '$1$2') // _emphasis_, not font_size or _next
    .replace(/\s+/g, ' ')
    .trim();
}

// Splits a page at its h2 and h3 headings. The anchors come from Astro's own rendered headings,
// matched in order, so they are exactly the ids on the page.
export async function sections(doc: Doc): Promise<Section[]> {
  const { headings } = await render(doc);
  const wanted = headings.filter((h) => h.depth === 2 || h.depth === 3);
  const out: Section[] = [{ page: doc.id, pageTitle: doc.data.title, heading: doc.data.title, anchor: '', text: '' }];
  let fence = false;
  let i = 0;
  const body: string[] = [];
  const flush = () => {
    out[out.length - 1].text = plain(body.join('\n'));
    body.length = 0;
  };
  for (const line of (doc.body ?? '').split('\n')) {
    if (/^\s*(```|~~~)/.test(line)) fence = !fence;
    const m = !fence && /^(#{2,3}) (.*)$/.exec(line);
    if (m) {
      const h = wanted[i++];
      if (!h) throw new Error(`${doc.id}: more headings in the markdown than Astro rendered (at "${m[2]}")`);
      flush();
      out.push({ page: doc.id, pageTitle: doc.data.title, heading: h.text, anchor: h.slug, text: '' });
    } else {
      body.push(line);
    }
  }
  flush();
  if (i !== wanted.length) throw new Error(`${doc.id}: matched ${i} of ${wanted.length} headings`);
  if (!wanted.length) {
    out[0].text = plain((doc.body ?? '').split('\n').filter((l) => !l.startsWith('|')).join('\n'));
    return [...out, ...tableRows(doc)];
  }
  return out;
}

// Text fragments treat these as syntax, so they are percent-encoded.
const fragment = (s: string) => encodeURIComponent(s).replace(/-/g, '%2D').replace(/,/g, '%2C');

// A page without headings (the feature table) is one long table: each row becomes its own result,
// linked with a text fragment (#:~:text=) that scrolls to the row in browsers that support it.
function tableRows(doc: Doc): Section[] {
  const rows: Section[] = [];
  for (const line of (doc.body ?? '').split('\n')) {
    const cells = /^\|(.+)\|\s*$/.exec(line)?.[1].split(/(?<!\\)\|/).map((c) => plain(c));
    if (!cells || cells.length < 2 || !cells[0] || /^-+$/.test(cells[0].replace(/[\s:]/g, '')) || cells[0] === 'Feature') continue;
    const title = cells[0].split(/ — /)[0];
    // The rendered page has typographic apostrophes (smartypants); a fragment stops before quotes.
    const words = title.split('"')[0].trim().split(' ').slice(0, 6).join(' ').replace(/'/g, '’');
    rows.push({
      page: doc.id,
      pageTitle: doc.data.title,
      heading: title,
      anchor: `:~:text=${fragment(words)}`,
      text: [cells[0].slice(title.length).replace(/^ — /, ''), cells.slice(1).join(' ')].filter(Boolean).join(' · '),
    });
  }
  return rows;
}
