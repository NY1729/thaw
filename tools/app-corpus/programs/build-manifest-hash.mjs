// Build script: walk a tree, hash every file with sha256, write a manifest.
import { mkdir, writeFile, readFile, readdir, stat } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { join, relative } from 'node:path';

await mkdir('site/css', { recursive: true });
await writeFile('site/index.html', '<h1>home</h1>\n');
await writeFile('site/css/main.css', 'body { margin: 0 }\n');
await writeFile('site/css/theme.css', ':root { --c: #fff }\n');

async function* walk(dir) {
  for (const entry of (await readdir(dir, { withFileTypes: true })).sort((a, b) => a.name.localeCompare(b.name))) {
    const p = join(dir, entry.name);
    if (entry.isDirectory()) yield* walk(p);
    else yield p;
  }
}

const files = {};
let total = 0;
for await (const f of walk('site')) {
  const data = await readFile(f);
  const info = await stat(f);
  total += info.size;
  files[relative('site', f)] = {
    size: info.size,
    sha256: createHash('sha256').update(data).digest('hex').slice(0, 16),
  };
}
const manifest = { version: 1, total, files };
await writeFile('site/manifest.json', JSON.stringify(manifest, null, 2) + '\n');
console.log(JSON.stringify(manifest));
