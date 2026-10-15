// Behaviour contracts for node:http / node:http2 recorded from real Node.js 22 (the expected JSON is the output of the identical
// JavaScript body under Node). Runs through the same `p2c_run` helper as network_contracts.rs.

#[test]
fn h2_http1_flush_headers() {
    // expected: node22
    assert_eq!(
        p2c_run("h2_http1_flush_headers", r#####"const http = require('node:http');
const out = { order: [] };
const server = http.createServer((req, res) => {
  out.order.push('server-headers'); out.te = req.headers['transfer-encoding']; out.cl = req.headers['content-length'];
  res.writeHead(200, { 'x-early': '1' }); res.flushHeaders();
  let body = ''; req.on('data', d => body += d);
  req.on('end', () => { out.order.push('server-end'); res.end('done:' + body); });
});
await new Promise(r => server.listen(0, '127.0.0.1', r));
const port = server.address().port;
const req = http.request({ port, host: '127.0.0.1', method: 'POST', path: '/p' });
req.on('error', e => { out.err = e.message; });
let early; const respSeen = new Promise(r => { early = r; });
req.on('response', (res) => { out.order.push('client-response:' + res.headers['x-early']); res.setEncoding('utf8'); let d = ''; res.on('data', x => d += x); res.on('end', () => { out.body = d; out.order.push('client-end'); out.finished = true; }); early(); });
const ret = req.flushHeaders();
out.flushReturns = ret === undefined;
await respSeen; out.order.push('writing-body');
req.write('a'); req.end('b');
await new Promise(r => { const t = setInterval(() => { if (out.finished) { clearInterval(t); r(); } }, 5); });
out.headersFlushed = req.headersSent;
await new Promise(r => server.close(r));
return out;
"#####),
        r#####"{"order":["server-headers","client-response:1","writing-body","server-end","client-end"],"flushReturns":true,"te":"chunked","body":"done:ab","finished":true,"headersFlushed":true}"#####
    );
}

#[test]
fn h2_http1_flush_variants() {
    // expected: node22
    assert_eq!(
        p2c_run("h2_http1_flush_variants", r#####"const http = require('node:http');
const out = { seen: [] };
const server = http.createServer((req, res) => {
  let body = ''; req.on('data', d => body += d);
  req.on('end', () => { out.seen.push([req.method + ' ' + req.url, req.headers['transfer-encoding'] || null, req.headers['content-length'] || null, body]); res.end('ok:' + body); });
});
await new Promise(r => server.listen(0, '127.0.0.1', r));
const port = server.address().port;
const run = (opts, act) => new Promise((res, rej) => { const req = http.request(Object.assign({ port, host: '127.0.0.1' }, opts)); req.on('error', rej); req.on('response', r => { let d = ''; r.setEncoding('utf8'); r.on('data', x => d += x); r.on('end', () => res(d)); }); act(req); });
out.contentLength = await run({ method: 'POST', path: '/cl', headers: { 'content-length': 5 } }, req => { req.flushHeaders(); req.write('he'); req.end('llo'); });
out.getFlush = await run({ method: 'GET', path: '/get' }, req => { req.flushHeaders(); req.end(); });
out.writeThenFlush = await run({ method: 'POST', path: '/wf' }, req => { req.write('x'); req.flushHeaders(); req.write('y'); req.end('z'); });
out.flushTwice = await run({ method: 'POST', path: '/twice' }, req => { req.flushHeaders(); req.flushHeaders(); req.end('t'); });
out.endCallback = await run({ method: 'POST', path: '/cb' }, req => { req.flushHeaders(); req.write('c'); req.end('d', () => { out.endCbCalled = true; }); });
// agent limit: second flushed request waits for the first socket
const agent = new http.Agent({ maxSockets: 1 });
const order = [];
const r1 = http.request({ port, host: '127.0.0.1', method: 'POST', path: '/a1', agent }); r1.on('response', r => { r.resume(); r.on('end', () => order.push('r1-done')); });
const r2 = http.request({ port, host: '127.0.0.1', method: 'POST', path: '/a2', agent }); r2.on('response', r => { r.resume(); r.on('end', () => order.push('r2-done')); });
r1.flushHeaders(); r2.flushHeaders();
await new Promise(r => setTimeout(r, 80));
out.seenBeforeEnd = out.seen.length;
r1.end('1'); await new Promise(r => setTimeout(r, 100)); r2.end('2');
await new Promise(r => setTimeout(r, 150));
out.order = order; agent.destroy();
out.seen = out.seen.map(x => x.join('|'));
await new Promise(r => server.close(r));
return out;
"#####),
        r#####"{"seen":["POST /cl||5|hello","GET /get|||","POST /wf|chunked||xyz","POST /twice|chunked||t","POST /cb|chunked||cd","POST /a1|chunked||1","POST /a2|chunked||2"],"contentLength":"ok:hello","getFlush":"ok:","writeThenFlush":"ok:xyz","flushTwice":"ok:t","endCbCalled":true,"endCallback":"ok:cd","seenBeforeEnd":5,"order":["r1-done","r2-done"]}"#####
    );
}

#[test]
fn h2_http2_abort_signal() {
    // expected: node22
    assert_eq!(
        p2c_run("h2_http2_abort_signal", r#####"const http2 = require('node:http2');
const out = {};
const server = http2.createServer();
const seen = [];
server.on('stream', (stream, headers) => { seen.push(headers[':path']); stream.on('close', () => seen.push('closed:' + headers[':path'] + ':' + stream.rstCode)); if (headers[':path'] === '/fast') { stream.respond({ ':status': 200 }); stream.end('ok'); } });
await new Promise(r => server.listen(0, '127.0.0.1', r));
const client = http2.connect('http://127.0.0.1:' + server.address().port);
client.on('error', () => {});
const ac = new AbortController();
const req = client.request({ ':path': '/slow' }, { signal: ac.signal });
req.on('error', e => { out.err = [e.name, e.code]; }); req.on('close', () => { out.closed = true; out.rst = req.rstCode; });
await new Promise(r => setTimeout(r, 60));
ac.abort();
await new Promise(r => setTimeout(r, 120));
const pre = new AbortController(); pre.abort();
const req2 = client.request({ ':path': '/never' }, { signal: pre.signal });
req2.on('error', e => { out.preErr = [e.name, e.code]; }); req2.on('close', () => { out.preClosed = true; });
await new Promise(r => setTimeout(r, 120));
const ac3 = new AbortController();
const req3 = client.request({ ':path': '/fast' }, { signal: ac3.signal });
out.fast = await new Promise(res => { let d = ''; req3.setEncoding('utf8'); req3.on('data', x => d += x); req3.on('end', () => res(d)); req3.on('error', () => res('error')); req3.end(); });
await new Promise(r => setTimeout(r, 40)); ac3.abort(); await new Promise(r => setTimeout(r, 40));
out.afterEndAbortNoError = true;
out.seen = seen.filter(x => !String(x).startsWith('/never'));
client.close(); await new Promise(r => setTimeout(r, 30)); await new Promise(r => server.close(r));
return out;
"#####),
        r#####"{"err":["AbortError","ABORT_ERR"],"closed":true,"rst":8,"preErr":["AbortError","ABORT_ERR"],"preClosed":true,"fast":"ok","afterEndAbortNoError":true,"seen":["/slow","closed:/slow:8","/fast","closed:/fast:0"]}"#####
    );
}

#[test]
fn h2_http2_compat_server() {
    // expected: node22
    assert_eq!(
        p2c_run("h2_http2_compat_server", r#####"const http2 = require('node:http2');
const out = {};
const server = http2.createServer((req, res) => {
  out.kinds = [req.constructor.name, res.constructor.name];
  out.req = { method: req.method, url: req.url, httpVersion: req.httpVersion, scheme: req.scheme, authority: typeof req.authority, hdr: req.headers['x-test'], pseudo: req.headers[':path'], rawHasPath: req.rawHeaders.includes(':path') };
  res.statusCode = 201; res.setHeader('X-Out', 'yes'); out.hasHeader = res.hasHeader('x-out'); out.getHeader = res.getHeader('x-out');
  let body = ''; req.setEncoding('utf8');
  req.on('data', d => body += d);
  req.on('end', () => {
    out.body = body; out.headersSentBefore = res.headersSent;
    res.write('part1,'); out.headersSentAfter = res.headersSent;
    res.on('finish', () => { out.finish = true; });
    res.end('part2');
    out.finished = res.writableEnded || res.finished;
  });
});
await new Promise(r => server.listen(0, '127.0.0.1', r));
const client = http2.connect('http://127.0.0.1:' + server.address().port);
client.on('error', () => {});
out.resp = await new Promise((res, rej) => { const req = client.request({ ':path': '/compat?x=1', ':method': 'POST', 'x-test': 'v' }); let d = ''; let h; req.on('response', x => { h = x; }); req.setEncoding('utf8'); req.on('data', x => d += x); req.on('end', () => res([h[':status'], h['x-out'], d])); req.on('error', rej); req.write('in-'); req.end('body'); });
// second: writeHead + 'request' event style + res.end only
const server2 = http2.createServer(); let n = 0;
server2.on('request', (req, res) => { n++; res.writeHead(404, { 'x-w': 'h' }); res.end('nf'); });
await new Promise(r => server2.listen(0, '127.0.0.1', r));
const client2 = http2.connect('http://127.0.0.1:' + server2.address().port); client2.on('error', () => {});
out.resp2 = await new Promise((res, rej) => { const req = client2.request({ ':path': '/' }); let d = ''; let h; req.on('response', x => { h = x; }); req.setEncoding('utf8'); req.on('data', x => d += x); req.on('end', () => res([h[':status'], h['x-w'], d])); req.on('error', rej); req.end(); });
out.n = n;
client.close(); client2.close(); await new Promise(r => setTimeout(r, 30)); await new Promise(r => server.close(r)); await new Promise(r => server2.close(r));
return out;
"#####),
        r#####"{"kinds":["Http2ServerRequest","Http2ServerResponse"],"req":{"method":"POST","url":"/compat?x=1","httpVersion":"2.0","scheme":"http","authority":"string","hdr":"v","pseudo":"/compat?x=1","rawHasPath":true},"hasHeader":true,"getHeader":"yes","body":"in-body","headersSentBefore":false,"headersSentAfter":true,"finished":true,"finish":true,"resp":[201,"yes","part1,part2"],"resp2":[404,"h","nf"],"n":1}"#####
    );
}

#[test]
fn h2_http2_create_connection() {
    // expected: node22
    assert_eq!(
        p2c_run("h2_http2_create_connection", r#####"const http2 = require('node:http2'); const net = require('node:net');
const out = { calls: [] };
const server = http2.createServer();
server.on('stream', (stream, headers) => { stream.respond({ ':status': 200 }); stream.end('via:' + headers[':path']); });
await new Promise(r => server.listen(0, '127.0.0.1', r));
const port = server.address().port;
const client = http2.connect('http://example.invalid:' + port, {
  createConnection: (authority, options) => { out.calls.push([authority.hostname, String(authority.port) === String(port), typeof options]); return net.connect(port, '127.0.0.1'); }
});
client.on('error', (e) => { out.clientError = e.code || e.message; });
out.body = await new Promise((res, rej) => { const req = client.request({ ':path': '/cc' }); let d = ''; req.setEncoding('utf8'); req.on('data', x => d += x); req.on('end', () => res(d)); req.on('error', rej); req.end(); });
client.close(); await new Promise(r => setTimeout(r, 30)); await new Promise(r => server.close(r));
return out;
"#####),
        r#####"{"calls":[["example.invalid",true,"object"]],"body":"via:/cc"}"#####
    );
}

#[test]
fn h2_http2_informational() {
    // expected: node22
    assert_eq!(
        p2c_run("h2_http2_informational", r#####"const http2 = require('node:http2');
const out = { events: [] };
const server = http2.createServer();
server.on('stream', (stream) => {
  stream.additionalHeaders({ ':status': 103, link: '</style.css>; rel=preload' });
  stream.additionalHeaders({ ':status': 100 });
  stream.respond({ ':status': 200, 'x-final': 'yes' });
  stream.end('body');
});
await new Promise(r => server.listen(0, '127.0.0.1', r));
const client = http2.connect('http://127.0.0.1:' + server.address().port);
client.on('error', () => {});
const req = client.request({ ':path': '/' });
await new Promise(res => {
  req.on('headers', (h) => out.events.push(['headers', h[':status'], h.link || null]));
  req.on('continue', () => out.events.push(['continue']));
  req.on('response', (h) => out.events.push(['response', h[':status'], h['x-final']]));
  req.on('trailers', (h) => out.events.push(['trailers', Object.keys(h)]));
  req.setEncoding('utf8'); req.on('data', d => out.events.push(['data', d])); req.on('end', () => { out.events.push(['end']); res(); }); req.end();
});
client.close(); await new Promise(r => setTimeout(r, 30)); await new Promise(r => server.close(r));
return out;
"#####),
        r#####"{"events":[["headers",103,"</style.css>; rel=preload"],["continue"],["headers",100,null],["response",200,"yes"],["data","body"],["end"]]}"#####
    );
}

#[test]
fn h2_http2_max_concurrent() {
    // expected: node22
    assert_eq!(
        p2c_run("h2_http2_max_concurrent", r#####"const http2 = require('node:http2');
const out = {};
let active = 0, maxActive = 0;
const server = http2.createServer({ settings: { maxConcurrentStreams: 1 } });
server.on('stream', (stream, headers) => {
  active++; maxActive = Math.max(maxActive, active);
  setTimeout(() => { stream.respond({ ':status': 200 }); stream.end(headers[':path']); active--; }, 40);
});
await new Promise(r => server.listen(0, '127.0.0.1', r));
const client = http2.connect('http://127.0.0.1:' + server.address().port);
client.on('error', () => {});
await new Promise(r => client.once('remoteSettings', () => r()));
out.remoteMax = client.remoteSettings.maxConcurrentStreams;
const reqs = ['/a', '/b', '/c'].map(p => client.request({ ':path': p }));
out.pendingAtStart = reqs.map(r => r.pending);
const bodies = await Promise.all(reqs.map(r => new Promise((res, rej) => { let d = ''; r.setEncoding('utf8'); r.on('data', x => d += x); r.on('end', () => res(d)); r.on('error', rej); r.end(); })));
out.bodies = bodies; out.maxActive = maxActive; out.ids = reqs.map(r => typeof r.id); out.pendingAfter = reqs.map(r => r.pending);
client.close(); await new Promise(r => setTimeout(r, 30)); await new Promise(r => server.close(r));
return out;
"#####),
        r#####"{"remoteMax":1,"pendingAtStart":[false,false,false],"bodies":["/a","/b","/c"],"maxActive":1,"ids":["number","number","number"],"pendingAfter":[false,false,false]}"#####
    );
}

#[test]
fn h2_http2_rawheaders() {
    // expected: node22
    assert_eq!(
        p2c_run("h2_http2_rawheaders", r#####"const http2 = require('node:http2');
const out = {};
const names = (raw) => raw.filter((v, i) => i % 2 === 0 && !v.startsWith(':') && v.startsWith('x-'));
const pairs = (raw) => { const r = []; for (let i = 0; i < raw.length; i += 2) if (raw[i].startsWith('x-')) r.push(raw[i] + '=' + raw[i + 1]); return r; };
const server = http2.createServer();
server.on('stream', (stream, headers, flags, rawHeaders) => {
  out.serverRaw = pairs(rawHeaders); out.serverRawIsArray = Array.isArray(rawHeaders);
  stream.respond({ ':status': 200, 'x-dup': ['c', 'd'], 'X-Mixed': 'Case' }, { waitForTrailers: true });
  stream.on('wantTrailers', () => stream.sendTrailers({ 'x-tr': ['1', '2'] }));
  stream.end('ok');
});
await new Promise(r => server.listen(0, '127.0.0.1', r));
const client = http2.connect('http://127.0.0.1:' + server.address().port);
client.on('error', () => {});
const req = client.request({ ':path': '/', 'x-dup': ['a', 'b'], 'x-single': 'z' });
await new Promise(res => {
  req.on('response', (h, f, raw) => { out.responseRaw = pairs(raw); });
  req.on('trailers', (h, f, raw) => { out.trailersRaw = pairs(raw); out.trailerHeaders = h; });
  req.resume(); req.on('end', res); req.end();
});
client.close(); await new Promise(r => setTimeout(r, 30)); await new Promise(r => server.close(r));
return out;
"#####),
        r#####"{"serverRaw":["x-dup=a","x-dup=b","x-single=z"],"serverRawIsArray":true,"responseRaw":["x-dup=c","x-dup=d","x-mixed=Case"],"trailersRaw":["x-tr=1","x-tr=2"],"trailerHeaders":{"x-tr":"1, 2"}}"#####
    );
}

#[test]
fn h2_http2_server_timeout() {
    // expected: node22
    assert_eq!(
        p2c_run("h2_http2_server_timeout", r#####"const http2 = require('node:http2');
const out = { calls: 0, args: [] };
const server = http2.createServer();
const ret = server.setTimeout(120, (a) => { out.calls++; out.args.push(a === undefined ? 'undef' : (a && a.constructor && a.constructor.name)); });
out.returnsServer = ret === server;
out.timeoutValue = server.timeout;
const events = [];
server.on('timeout', () => events.push('timeout-listener'));
server.on('stream', (stream) => { stream.respond({ ':status': 200 }); stream.end('x'); });
await new Promise(r => server.listen(0, '127.0.0.1', r));
const client = http2.connect('http://127.0.0.1:' + server.address().port);
client.on('error', () => {});
await new Promise(res => { const req = client.request({ ':path': '/' }); req.resume(); req.on('end', res); req.end(); });
await new Promise(r => setTimeout(r, 700));
out.fired = out.calls > 0; out.events = events.length > 0;
client.close(); await new Promise(r => setTimeout(r, 30)); await new Promise(r => server.close(r));
return out;
"#####),
        r#####"{"calls":1,"args":["ServerHttp2Session"],"returnsServer":true,"timeoutValue":120,"fired":true,"events":true}"#####
    );
}

#[test]
fn h2_http2_session_timeout() {
    // expected: node22
    assert_eq!(
        p2c_run("h2_http2_session_timeout", r#####"const http2 = require('node:http2');
const out = { client: 0, server: 0 };
const server = http2.createServer();
server.on('stream', (stream) => { stream.respond({ ':status': 200 }); stream.end('x'); });
server.on('session', (s) => { s.setTimeout(120, () => { out.server++; }); });
await new Promise(r => server.listen(0, '127.0.0.1', r));
const client = http2.connect('http://127.0.0.1:' + server.address().port);
client.on('error', () => {});
const ret = client.setTimeout(120, () => { out.client++; });
out.returnsSession = ret === client;
out.timeoutProp = typeof client.timeout;
await new Promise(r => setTimeout(r, 600));
out.idleFired = [out.client > 0, out.server > 0];
const before = out.client;
await new Promise(res => { const req = client.request({ ':path': '/' }); req.resume(); req.on('end', res); req.end(); });
out.afterActivity = out.client - before;
await new Promise(r => setTimeout(r, 450));
out.refiredAfterActivity = out.client > before;
client.setTimeout(0);
const frozen = out.client; await new Promise(r => setTimeout(r, 400)); out.disabled = out.client === frozen;
client.close(); await new Promise(r => setTimeout(r, 30)); await new Promise(r => server.close(r));
return out;
"#####),
        r#####"{"client":1,"server":1,"returnsSession":true,"timeoutProp":"number","idleFired":[true,true],"afterActivity":0,"refiredAfterActivity":false,"disabled":true}"#####
    );
}

#[test]
fn h2_http2_stream_duplex() {
    // expected: node22
    assert_eq!(
        p2c_run("h2_http2_stream_duplex", r#####"const http2 = require('node:http2'); const stream = require('node:stream');
const out = {};
const server = http2.createServer();
server.on('stream', (s, headers) => {
  if (headers[':path'] === '/echo') { s.respond({ ':status': 200 }); s.pipe(s); return; }
  s.respond({ ':status': 200 }); s.write('alpha,'); s.write('beta,'); s.end('gamma');
});
await new Promise(r => server.listen(0, '127.0.0.1', r));
const client = http2.connect('http://127.0.0.1:' + server.address().port);
client.on('error', () => {});
const r1 = client.request({ ':path': '/' }); r1.end();
out.instanceof = [r1 instanceof stream.Duplex, r1 instanceof stream.Readable, r1 instanceof stream.Writable, typeof r1.pipe, typeof r1.read, typeof r1[Symbol.asyncIterator]];
let all = ''; for await (const chunk of r1) all += chunk.toString();
out.asyncIter = all;
const r2 = client.request({ ':path': '/' }); r2.end();
const sink = []; const w = new stream.Writable({ write(c, e, cb) { sink.push(c.toString()); cb(); } });
await new Promise(res => { r2.pipe(w); w.on('finish', res); });
out.piped = sink.join('');
const r3 = client.request({ ':path': '/echo', ':method': 'POST' });
let echoed = ''; r3.setEncoding('utf8'); r3.on('data', d => echoed += d);
await new Promise(res => { r3.on('end', res); r3.write('ping-'); r3.end('pong'); });
out.echo = echoed;
const r4 = client.request({ ':path': '/' }); r4.end();
const closed4 = new Promise(r => r4.on('close', r));
let chunks4 = []; await new Promise(res => { r4.on('readable', () => { let c; while ((c = r4.read()) !== null) chunks4.push(c.toString()); }); r4.on('end', res); });
out.readViaReadable = chunks4.join('');
await closed4;
out.flags = [r4.readableEnded, r4.writableEnded, r4.destroyed];
client.close(); await new Promise(r => setTimeout(r, 30)); await new Promise(r => server.close(r));
return out;
"#####),
        r#####"{"instanceof":[true,true,true,"function","function","function"],"asyncIter":"alpha,beta,gamma","piped":"alpha,beta,gamma","echo":"ping-pong","readViaReadable":"alpha,beta,gamma","flags":[true,true,true]}"#####
    );
}

#[test]
fn h2_http2_wait_for_trailers() {
    // expected: node22
    assert_eq!(
        p2c_run("h2_http2_wait_for_trailers", r#####"const http2 = require('node:http2');
const out = { events: [] };
const server = http2.createServer();
server.on('stream', (stream, headers) => {
  if (headers[':path'] === '/endstream') {
    stream.respond({ ':status': 200 }, { endStream: false, waitForTrailers: true });
    stream.on('wantTrailers', () => { out.events.push('server-wantTrailers-2'); stream.sendTrailers({ 'x-sum': 'q' }); });
    stream.end(); return;
  }
  stream.respond({ ':status': 200, 'content-type': 'text/plain' }, { waitForTrailers: true });
  stream.on('wantTrailers', () => { out.events.push('server-wantTrailers'); stream.sendTrailers({ 'x-checksum': 'abc123' }); });
  stream.on('finish', () => out.events.push('server-finish'));
  stream.write('hello ');
  stream.end('world');
});
await new Promise(r => server.listen(0, '127.0.0.1', r));
const client = http2.connect('http://127.0.0.1:' + server.address().port);
client.on('error', () => {});
const run = (path) => new Promise(res => {
  const req = client.request({ ':path': path }); const ev = [];
  req.on('response', h => ev.push('response:' + h[':status']));
  req.on('trailers', h => ev.push('trailers:' + JSON.stringify(h)));
  req.setEncoding('utf8'); req.on('data', d => ev.push('data:' + d));
  req.on('end', () => { ev.push('end'); });
  req.on('close', () => res(ev)); req.end();
});
out.main = await run('/'); out.empty = await run('/endstream');
const t = http2.constants; out.hasConst = typeof t.HTTP2_HEADER_STATUS;
client.close(); await new Promise(r => setTimeout(r, 30)); await new Promise(r => server.close(r));
return out;
"#####),
        r#####"{"events":["server-wantTrailers","server-finish","server-wantTrailers-2"],"main":["response:200","data:hello ","data:world","trailers:{\"x-checksum\":\"abc123\"}","end"],"empty":["response:200","trailers:{\"x-sum\":\"q\"}","end"],"hasConst":"string"}"#####
    );
}

#[test]
fn h2_http2_zero_streams() {
    // expected: node22
    assert_eq!(
        p2c_run("h2_http2_zero_streams", r#####"const http2 = require('node:http2');
const out = {};
const server = http2.createServer({ settings: { maxConcurrentStreams: 0 } });
const got = [];
server.on('stream', (stream, headers) => { got.push(headers[':path']); stream.respond({ ':status': 200 }); stream.end('ok'); });
await new Promise(r => server.listen(0, '127.0.0.1', r));
let serverSession; server.on('session', s => { serverSession = s; });
const client = http2.connect('http://127.0.0.1:' + server.address().port);
client.on('error', () => {});
await new Promise(r => client.once('remoteSettings', () => r()));
out.remoteMax = client.remoteSettings.maxConcurrentStreams;
const req = client.request({ ':path': '/zero' });
let body = ''; req.setEncoding('utf8'); req.on('data', d => body += d); req.on('error', e => { out.error = e.code; });
let ended = new Promise(r => req.on('end', r)); req.end();
await new Promise(r => setTimeout(r, 120));
out.serverSawWhileZero = got.slice(); out.pendingWhileZero = req.pending;
serverSession.settings({ maxConcurrentStreams: 5 });
await Promise.race([ended, new Promise(r => setTimeout(r, 800))]);
out.afterRaise = got.slice(); out.body = body;
client.close(); await new Promise(r => setTimeout(r, 30)); await new Promise(r => server.close(r));
return out;
"#####),
        r#####"{"remoteMax":0,"serverSawWhileZero":[],"pendingWhileZero":false,"afterRaise":["/zero"],"body":"ok"}"#####
    );
}

#[test]
fn h2_stream_duplex_instanceof() {
    // expected: node22
    assert_eq!(
        p2c_run("h2_stream_duplex_instanceof", r#####"const stream = require('node:stream');
const d = new stream.Duplex({ read() {}, write(c, e, cb) { cb(); } });
class Custom extends stream.Duplex { _read() {} _write(c, e, cb) { cb(); } }
return [d instanceof stream.Writable, d instanceof stream.Readable, d instanceof stream.Duplex, new stream.PassThrough() instanceof stream.Writable, new stream.Transform({ transform(c, e, cb) { cb(); } }) instanceof stream.Writable, new Custom() instanceof stream.Writable, new stream.Readable({ read() {} }) instanceof stream.Writable, new stream.Writable({ write(c, e, cb) { cb(); } }) instanceof stream.Readable, ({}) instanceof stream.Writable, null instanceof stream.Writable];
"#####),
        r#####"[true,true,true,true,true,true,false,false,false,false]"#####
    );
}
