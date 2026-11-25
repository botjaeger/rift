import type { APIRoute } from 'astro';
import { getDocs, sections } from '../../lib/docs';

// Every h2/h3 section of every docs page, fetched by the search box on first use.
export const GET: APIRoute = async () => {
  const all = (await Promise.all((await getDocs()).map(sections))).flat();
  const rows = all.map((s) => [s.page, s.pageTitle, s.heading, s.anchor, s.text]);
  return new Response(JSON.stringify(rows), { headers: { 'Content-Type': 'application/json; charset=utf-8' } });
};
