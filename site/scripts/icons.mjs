// Regenerates the logo sizes and favicons in public/ from the 1024 px app icon.
// Usage: node scripts/icons.mjs [path/to/rift-1024.png]   (default: scripts/rift-1024.png)
import { writeFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import sharp from 'sharp';

const root = fileURLToPath(new URL('..', import.meta.url));
const src = process.argv[2] ?? `${root}scripts/rift-1024.png`;
const png = (size) => sharp(src).resize(size, size, { kernel: 'lanczos3' }).png({ compressionLevel: 9 }).toBuffer();

const outputs = {
  'public/rift-40.png': 40, // footer, 20 px at 2x
  'public/rift-56.png': 56, // nav, 28 px at 2x
  'public/rift-144.png': 144, // closing call to action, 72 px at 2x
  'public/apple-touch-icon.png': 180,
  'public/favicon-32.png': 32,
};
for (const [file, size] of Object.entries(outputs)) await writeFile(`${root}${file}`, await png(size));

// favicon.ico: an ICO container holding PNG images (supported by every current browser).
const sizes = [16, 32, 48];
const images = await Promise.all(sizes.map(png));
const header = Buffer.alloc(6 + 16 * sizes.length);
header.writeUInt16LE(0, 0);
header.writeUInt16LE(1, 2);
header.writeUInt16LE(sizes.length, 4);
let offset = header.length;
sizes.forEach((size, i) => {
  const e = 6 + 16 * i;
  header.writeUInt8(size, e);
  header.writeUInt8(size, e + 1);
  header.writeUInt16LE(1, e + 4); // colour planes
  header.writeUInt16LE(32, e + 6); // bits per pixel
  header.writeUInt32LE(images[i].length, e + 8);
  header.writeUInt32LE(offset, e + 12);
  offset += images[i].length;
});
await writeFile(`${root}public/favicon.ico`, Buffer.concat([header, ...images]));
console.log('icons written');
