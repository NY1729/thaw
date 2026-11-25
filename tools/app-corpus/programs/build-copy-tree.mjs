// Build script: clean dist/, copy a source tree, URL-object paths, top-level await.
import { rm, mkdir, cp, copyFile, writeFile, readdir } from 'node:fs/promises';

const root = new URL('./', import.meta.url);
const src = new URL('src/', root);
const dist = new URL('dist/', root);

await mkdir(new URL('assets/img/', src), { recursive: true });
await writeFile(new URL('index.html', src), '<html><body>hi</body></html>\n');
await writeFile(new URL('app.js', src), 'console.log("app");\n');
await writeFile(new URL('assets/img/logo.txt', src), 'LOGO\n');

console.log('cleaning dist');
await rm(dist, { recursive: true, force: true });
await mkdir(dist, { recursive: true });

await cp(src, dist, { recursive: true });
await copyFile(new URL('app.js', src), new URL('app.copy.js', dist));
console.log('copied');

const names = (await readdir(dist)).sort();
console.log(names.join(','));
