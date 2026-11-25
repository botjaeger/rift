import { existsSync, readFileSync } from 'node:fs';

// Where the site is served. With a custom domain, public/CNAME holds it and the site lives at its
// root (a GitHub Actions Pages deploy ignores the file, so set the same domain under Settings ›
// Pages › Custom domain). Without one, it is the repo's project site: https://botjaeger.github.io/rift/.
// Read by astro.config.mjs, the docs sync (Markdown links) and scripts/check-links.mjs.
const cname = new URL('./public/CNAME', import.meta.url);
export const domain = existsSync(cname) ? readFileSync(cname, 'utf8').trim() : '';
export const site = domain ? `https://${domain}` : 'https://botjaeger.github.io';
// '' at a domain root, '/rift' on the project site: prefix it to a root-relative path.
export const base = domain ? '' : '/rift';
