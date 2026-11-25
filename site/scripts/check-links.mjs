// Fails the build when a built page links to a root-relative path outside the site's base path
// (an href or src written as '/docs/' instead of through src/lib/url.ts): under the project site
// https://botjaeger.github.io/rift/ such a link is a 404. Also fails on an internal link to a page
// or file the build did not produce.
import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { base } from '../base.mjs';

const dist = new URL('../dist/', import.meta.url).pathname;

const pages = [];
const walk = (dir) => {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) walk(p);
    else if (name.endsWith('.html')) pages.push(p);
  }
};
walk(dist);

const bad = [];
for (const page of pages) {
  const html = readFileSync(page, 'utf8');
  for (const [, attr, value] of html.matchAll(/\s(href|src|action)="(\/[^"]*)"/g)) {
    if (value.startsWith('//')) continue;
    const where = `${page.slice(dist.length)}: ${attr}="${value}"`;
    if (base && value !== base && !value.startsWith(`${base}/`)) {
      bad.push(`${where} is outside ${base}/`);
      continue;
    }
    const path = decodeURI(value.slice(base.length).split(/[?#]/)[0]) || '/';
    const file = join(dist, path);
    const found = path.endsWith('/') ? existsSync(join(file, 'index.html')) : existsSync(file) || existsSync(join(file, 'index.html'));
    if (!found) bad.push(`${where} points at nothing in the build`);
  }
}

if (bad.length) {
  console.error(`check-links: ${bad.length} broken link(s)\n  ${bad.join('\n  ')}`);
  process.exit(1);
}
console.log(`check-links: ${pages.length} pages, every internal link resolves under ${base || '/'}`);
