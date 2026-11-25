// fs operations: sync + promise mix, rename, append, readdir withFileTypes, stat, symlink, rm.
import fs from 'node:fs';
import { appendFile, readFile, rename, readdir, stat, rm, mkdtemp, access, truncate } from 'node:fs/promises';
import { join, basename, extname } from 'node:path';
import { tmpdir } from 'node:os';

fs.mkdirSync('data/nested/deep', { recursive: true });
fs.writeFileSync('data/a.txt', 'one\n');
await appendFile('data/a.txt', 'two\n');
fs.appendFileSync('data/a.txt', 'three\n');
await rename('data/a.txt', 'data/nested/b.txt');
fs.copyFileSync('data/nested/b.txt', 'data/nested/deep/c.log');
fs.symlinkSync('b.txt', 'data/nested/link');
console.log(fs.readFileSync('data/nested/link', 'utf8').split('\n').length);

const entries = await readdir('data', { recursive: true, withFileTypes: true });
console.log(entries.map((e) => `${e.isDirectory() ? 'd' : e.isSymbolicLink() ? 'l' : 'f'}:${e.name}`).sort());
const st = await stat('data/nested/b.txt');
console.log(st.size, st.isFile(), st.isDirectory(), (st.mode & 0o777).toString(8).length);
await truncate('data/nested/deep/c.log', 4);
console.log(JSON.stringify(await readFile('data/nested/deep/c.log', 'utf8')));
console.log(fs.existsSync('data/nope'), await access('data/nope').then(() => 'yes', (e) => e.code));
try {
  fs.readFileSync('data/nope');
} catch (e) {
  console.log(e.code, e.syscall, e.message.startsWith('ENOENT: no such file or directory, open'));
}
try {
  fs.mkdirSync('data');
} catch (e) {
  console.log(e.code);
}
const tmp = await mkdtemp(join(tmpdir(), 'ops-'));
console.log(basename(tmp).startsWith('ops-'), extname('x/y.tar.gz'));
await rm(tmp, { recursive: true });
await rm('data/nested/deep', { recursive: true });
fs.rmSync('data/missing', { force: true });
console.log(fs.readdirSync('data/nested').sort());
