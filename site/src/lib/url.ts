// The site's own path for a root-relative one: '/docs/' becomes '/rift/docs/' when the site is
// served under a base path (a GitHub Pages project site), and stays '/docs/' at a domain root.
// Every internal link and asset goes through it; scripts/check-links.mjs fails the build on one
// that does not.
const base = import.meta.env.BASE_URL.replace(/\/$/, '');

export const url = (path: string): string => (path.startsWith('/') ? `${base}${path}` : path);
