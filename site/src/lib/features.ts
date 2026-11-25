import { getEntry } from 'astro:content';
import { parseRows, type FeatureRow } from './feature-rows';

export { groupRows } from './feature-rows';

// The README's Features table, as synced into src/content/docs/features.md.
export async function featureRows(): Promise<FeatureRow[]> {
  const doc = await getEntry('docs', 'features');
  if (!doc?.body) throw new Error('src/content/docs/features.md is missing; run `pnpm docs:sync ../rift`');
  const rows = parseRows(doc.body);
  if (!rows.length) throw new Error('features.md has no feature table');
  return rows;
}
