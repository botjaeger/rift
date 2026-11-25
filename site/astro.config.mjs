import { readFileSync } from 'node:fs';
import { defineConfig } from 'astro/config';

// public/CNAME holds the custom domain, used here for canonical, OG and sitemap URLs. A GitHub Actions
// Pages deploy ignores the file: set the same domain under Settings › Pages › Custom domain.
const domain = readFileSync(new URL('./public/CNAME', import.meta.url), 'utf8').trim();

export default defineConfig({
  site: `https://${domain}`,
  output: 'static',
  trailingSlash: 'ignore',
  build: { format: 'directory' },
  // github-dark-default: its comment colour passes 4.5:1 on the black code background (github-dark's does not).
  markdown: { shikiConfig: { theme: 'github-dark-default' } },
});
