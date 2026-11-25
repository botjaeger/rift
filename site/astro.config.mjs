import { defineConfig } from 'astro/config';
import { base, site } from './base.mjs';

// base.mjs says where the site is served (a custom domain from public/CNAME, else
// https://botjaeger.github.io/rift/). Internal links go through src/lib/url.ts.
export default defineConfig({
  site,
  base: base || '/',
  output: 'static',
  trailingSlash: 'ignore',
  build: { format: 'directory' },
  // github-dark-default: its comment colour passes 4.5:1 on the black code background (github-dark's does not).
  markdown: { shikiConfig: { theme: 'github-dark-default' } },
});
