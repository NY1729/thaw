// Routed fetch corpus (Q9 plan-q9-thawjs-fetch.md, test 2): global fetch next to a node:http server.
// Node is the oracle. Regenerate only from node:
//   node tools/parity/routed/fetch.js > tools/parity/routed/fetch.expected
// Not in tools/parity/run.sh (its corpus glob would count these as parity gaps). Thaw side is a routed
// build run by hand or by the diag workflow:
//   thaw build tools/parity/routed/fetch.js -o "$D/fetch" && "$D/fetch" | diff - tools/parity/routed/fetch.expected
import * as http from 'node:http';
import * as zlib from 'node:zlib';

const routes = {
  '/plain': (req, res) => { res.setHeader('x-a', 'b'); res.end('hello'); },
  '/echo': (req, res) => {
    let body = '';
    req.setEncoding('utf8');
    req.on('data', (chunk) => { body += chunk; });
    req.on('end', () => { res.end(`${req.method}:${body}`); });
  },
  '/r302': (req, res) => { res.statusCode = 302; res.setHeader('location', '/final'); res.end(); },
  '/final': (req, res) => { res.end('final'); },
  '/gzip': (req, res) => {
    res.setHeader('content-encoding', 'gzip');
    res.end(zlib.gzipSync('gzipped body'));
  },
  '/chunked': (req, res) => {
    res.setHeader('transfer-encoding', 'chunked');
    res.write('chunk-one,');
    res.end('chunk-two');
  },
  '/slow': (req, res) => {
    res.setHeader('content-length', '100');
    res.write('part');
  },
};

const server = http.createServer((req, res) => {
  const handler = routes[req.url.split('?')[0]];
  if (handler) handler(req, res);
  else { res.statusCode = 404; res.end('missing'); }
});

async function main() {
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  const base = `http://127.0.0.1:${server.address().port}`;

  console.log(typeof fetch);
  let r = await fetch(`${base}/plain`);
  console.log(r.status, r.ok, r.statusText, r.headers.get('x-a'), r.redirected, r.type, new URL(r.url).pathname);
  console.log(JSON.stringify(await r.text()));

  r = await fetch(`${base}/echo`, { method: 'POST', body: 'payload' });
  console.log(await r.text());

  r = await fetch(`${base}/r302`);
  console.log(r.status, r.redirected, new URL(r.url).pathname, await r.text());

  r = await fetch(`${base}/r302`, { redirect: 'manual' });
  console.log(r.status, r.redirected, r.headers.get('location'));

  try {
    await fetch(`${base}/r302`, { redirect: 'error' });
    console.log('no error');
  } catch (e) {
    console.log(e.name);
  }

  r = await fetch(`${base}/gzip`);
  console.log(await r.text());

  r = await fetch(`${base}/chunked`);
  console.log(await r.text());

  const ac = new AbortController();
  r = await fetch(`${base}/slow`, { signal: ac.signal });
  setTimeout(() => ac.abort(), 20);
  try {
    await r.text();
    console.log('no abort');
  } catch (e) {
    console.log(e.name);
  }

  try {
    await fetch(`${base}/plain`, { signal: AbortSignal.abort() });
    console.log('no abort');
  } catch (e) {
    console.log(e.name);
  }

  const closed = http.createServer();
  await new Promise((resolve) => closed.listen(0, '127.0.0.1', resolve));
  const closedPort = closed.address().port;
  await new Promise((resolve) => closed.close(resolve));
  try {
    await fetch(`http://127.0.0.1:${closedPort}/`);
    console.log('no error');
  } catch (e) {
    console.log(e.name, e.message, e.cause?.code);
  }

  server.closeAllConnections?.();
  await new Promise((resolve) => server.close(resolve));
}

main();
