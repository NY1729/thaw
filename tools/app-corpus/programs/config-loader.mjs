// Config loader: JSON file + dotenv-style file + env overrides, with defaults and coercion.
import { readFile, writeFile } from 'node:fs/promises';

await writeFile('config.json', JSON.stringify({ port: 8080, db: { host: 'localhost', pool: 5 }, features: ['a', 'b'] }));
await writeFile('.env', '# comment\nPORT=9090\nDB_HOST="db.internal"\nDEBUG=true\nEMPTY=\nQUOTED=\'single quoted\'\nBAD LINE\n');

function parseDotenv(text) {
  const out = {};
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim();
    if (!line || line.startsWith('#')) continue;
    const eq = line.indexOf('=');
    if (eq < 0) continue;
    const key = line.slice(0, eq).trim();
    if (!/^[A-Z_][A-Z0-9_]*$/.test(key)) continue;
    let v = line.slice(eq + 1).trim();
    if (/^(["']).*\1$/.test(v)) v = v.slice(1, -1);
    out[key] = v;
  }
  return out;
}

function deepMerge(a, b) {
  const r = { ...a };
  for (const [k, v] of Object.entries(b)) {
    r[k] = v && typeof v === 'object' && !Array.isArray(v) ? deepMerge(a[k] ?? {}, v) : v;
  }
  return r;
}

const base = JSON.parse(await readFile('config.json', 'utf8'));
const env = parseDotenv(await readFile('.env', 'utf8'));
process.env.APP_POOL = '12';
const overrides = {
  port: Number(env.PORT),
  db: { host: env.DB_HOST, pool: Number(process.env.APP_POOL) },
  debug: env.DEBUG === 'true',
};
const cfg = deepMerge(base, overrides);
console.log(JSON.stringify(cfg, null, 2));
console.log(Object.keys(env).sort());
console.log(JSON.stringify(env.EMPTY), env.QUOTED);
