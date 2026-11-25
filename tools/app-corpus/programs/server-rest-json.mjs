// JSON REST server on port 0, exercised with http.request and fetch.
import http from 'node:http';

const items = new Map();
let nextId = 1;

async function readJson(req) {
  const chunks = [];
  for await (const c of req) chunks.push(c);
  const raw = Buffer.concat(chunks).toString('utf8');
  return raw ? JSON.parse(raw) : undefined;
}

const server = http.createServer(async (req, res) => {
  const send = (status, body) => {
    const text = JSON.stringify(body);
    res.writeHead(status, { 'Content-Type': 'application/json', 'Content-Length': Buffer.byteLength(text) });
    res.end(text);
  };
  try {
    const { pathname } = new URL(req.url, 'http://x');
    const m = /^\/items(?:\/(\d+))?$/.exec(pathname);
    if (!m) return send(404, { error: 'no route' });
    const id = m[1] ? Number(m[1]) : undefined;
    if (req.method === 'GET' && id === undefined) return send(200, [...items.values()]);
    if (req.method === 'GET') return items.has(id) ? send(200, items.get(id)) : send(404, { error: 'missing' });
    if (req.method === 'POST' && id === undefined) {
      const body = await readJson(req);
      const item = { id: nextId++, name: String(body.name), tags: body.tags ?? [] };
      items.set(item.id, item);
      return send(201, item);
    }
    if (req.method === 'DELETE' && id !== undefined) {
      return send(items.delete(id) ? 200 : 404, { deleted: id });
    }
    send(405, { error: 'method' });
  } catch (e) {
    send(400, { error: e.message.slice(0, 20) });
  }
});

await new Promise((r) => server.listen(0, '127.0.0.1', r));
const port = server.address().port;

function request(method, path, body) {
  return new Promise((resolve, reject) => {
    const req = http.request({ host: '127.0.0.1', port, method, path, headers: { 'content-type': 'application/json' } }, (res) => {
      let data = '';
      res.setEncoding('utf8');
      res.on('data', (d) => (data += d));
      res.on('end', () => resolve({ status: res.statusCode, body: JSON.parse(data) }));
    });
    req.on('error', reject);
    if (body !== undefined) req.write(JSON.stringify(body));
    req.end();
  });
}

console.log(await request('POST', '/items', { name: 'a', tags: ['x'] }));
console.log(await request('POST', '/items', { name: 'b' }));
console.log(await request('GET', '/items'));
console.log(await request('GET', '/items/2'));
console.log(await request('DELETE', '/items/1'));
console.log(await request('GET', '/items/1'));
console.log(await request('PUT', '/items', {}));
const f = await fetch(`http://127.0.0.1:${port}/items`);
console.log(f.status, await f.json());
const bad = await fetch(`http://127.0.0.1:${port}/items`, { method: 'POST', body: '{oops' });
console.log(bad.status);
server.close();
