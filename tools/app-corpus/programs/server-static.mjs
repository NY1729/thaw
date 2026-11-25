// Static file server with an entry guard, HEAD/405 handling and self-requests on an ephemeral port.
import { createServer } from 'node:http';
import { readFile, mkdir, writeFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { join, extname, normalize } from 'node:path';

const types = { '.html': 'text/html; charset=utf-8', '.css': 'text/css', '.json': 'application/json' };
const root = join(process.cwd(), 'public');

export function makeServer(dir) {
  return createServer(async (req, res) => {
    if (req.method !== 'GET' && req.method !== 'HEAD') {
      res.writeHead(405).end();
      return;
    }
    const url = new URL(req.url ?? '/', 'http://localhost');
    const rel = normalize(url.pathname === '/' ? '/index.html' : url.pathname);
    try {
      const path = join(dir, rel);
      const body = await readFile(path);
      res.writeHead(200, { 'content-type': types[extname(path)] ?? 'application/octet-stream' });
      res.end(req.method === 'HEAD' ? undefined : body);
    } catch {
      res.statusCode = 404;
      res.end('not found');
    }
  });
}

async function main() {
  await mkdir(root, { recursive: true });
  await writeFile(join(root, 'index.html'), '<p>index</p>');
  await writeFile(join(root, 'data.json'), '{"a":1}');
  const server = makeServer(root);
  server.on('error', (err) => {
    console.error('server error', err.message);
    process.exitCode = 1;
  });
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  const base = `http://127.0.0.1:${server.address().port}`;
  for (const [method, path] of [['GET', '/'], ['GET', '/data.json'], ['HEAD', '/'], ['POST', '/'], ['GET', '/missing.txt']]) {
    const r = await fetch(base + path, { method });
    const body = await r.text();
    console.log(method, path, r.status, r.headers.get('content-type'), JSON.stringify(body));
  }
  await new Promise((resolve) => server.close(resolve));
  console.log('closed');
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  await main();
}
