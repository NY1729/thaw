// Behaviour contracts for node:dns / net / tls / dgram / http / http2 built-ins.
// Each case below runs the same JavaScript body through the bundled Thaw built-in module; the expected JSON was recorded by running the
// identical body under real Node.js 22 ("node22") or, for cases that stub Thaw-internal natives, is the specified result ("thaw-only").

fn p2c_ipv6_available() -> bool {
    std::net::TcpListener::bind("[::1]:0").is_ok()
}

fn p2c_run(label: &str, body: &str) -> String {
    use std::ffi::{CStr, CString};
    let dir = temp_registry(&format!("p2c_{label}"));
    fs::write(dir.join("index.js"), format!("module.exports = async function () {{ {body}\n}};")).unwrap();
    let empty_node_modules = temp_registry(&format!("p2c_{label}_modules"));
    let (bundle, _, _, _) = bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    let global = format!("p2cRun_{label}");
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.{global} = module.exports;");
    let loaded = thaw_quickjs::thaw_js_load(CString::new(script).unwrap().as_ptr());
    let result = if loaded == 1 {
        let name = CString::new(global).unwrap();
        let pointer = thaw_quickjs::thaw_js_call(name.as_ptr(), c"[]".as_ptr());
        unsafe { CStr::from_ptr(pointer) }.to_string_lossy().into_owned()
    } else {
        String::from("load failed")
    };
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
    result
}

#[test]
fn p2c_dgram_bind_state() {
    // expected: node22
    assert_eq!(
        p2c_run("dgram_bind_state", r#####"const dgram = require('node:dgram');
const out = {};
const s = dgram.createSocket('udp4');
out.bindCb = await new Promise(r => s.bind(0, () => r('cb')));
const first = s.address().port;
try { s.bind(0); out.rebind = 'no throw'; } catch (e) { out.rebind = e.code; }
out.samePort = s.address().port === first;
const t = dgram.createSocket('udp4'); out.bindAddrCb = await new Promise(r => t.bind(0, '127.0.0.1', () => r(t.address().address)));
const u = dgram.createSocket('udp4'); out.bindOptCb = await new Promise(r => u.bind({ port: 0, address: '127.0.0.1' }, () => r(u.address().address)));
for (const x of [s, t, u]) await new Promise(r => x.close(r));
return out;
"#####),
        r#####"{"bindCb":"cb","rebind":"ERR_SOCKET_ALREADY_BOUND","samePort":true,"bindAddrCb":"127.0.0.1","bindOptCb":"127.0.0.1"}"#####
    );
}

#[test]
fn p2c_dgram_broadcast_connect() {
    // expected: node22
    assert_eq!(
        p2c_run("dgram_broadcast_connect", r#####"const dgram = require('node:dgram');
const out = {};
const recv = dgram.createSocket({ type: 'udp4', reuseAddr: true });
await new Promise(r => recv.bind(0, '0.0.0.0', r)); const port = recv.address().port;
const got = []; recv.on('message', (m, rinfo) => got.push(String(m)));
const send = (s, msg, host, p) => new Promise(r => s.send(msg, p, host, e => r(e ? e.code : 'sent')));
const noBc = dgram.createSocket('udp4'); await new Promise(r => noBc.bind(0, r));
out.withoutBroadcast = await send(noBc, 'a', '127.255.255.255', port);
const bc = dgram.createSocket('udp4'); await new Promise(r => bc.bind(0, r)); bc.setBroadcast(true);
out.withBroadcast = await send(bc, 'b', '127.255.255.255', port);
await new Promise(r => setTimeout(r, 100)); out.received = got.slice();
// connected sockets only accept datagrams from their peer
const peer = dgram.createSocket('udp4'); await new Promise(r => peer.bind(0, '127.0.0.1', r));
const stranger = dgram.createSocket('udp4'); await new Promise(r => stranger.bind(0, '127.0.0.1', r));
const conn = dgram.createSocket('udp4'); await new Promise(r => conn.bind(0, '127.0.0.1', r));
await new Promise(r => conn.connect(peer.address().port, '127.0.0.1', r));
const connPort = conn.address().port; const heard = []; conn.on('message', m => heard.push(String(m)));
await send(stranger, 'from-stranger', '127.0.0.1', connPort);
await send(peer, 'from-peer', '127.0.0.1', connPort);
await new Promise(r => setTimeout(r, 150)); out.connectedHeard = heard;
out.remoteAddress = conn.remoteAddress().port === peer.address().port;
conn.disconnect(); await send(stranger, 'after-disconnect', '127.0.0.1', connPort);
await new Promise(r => setTimeout(r, 150)); out.afterDisconnect = heard.slice();
for (const x of [recv, noBc, bc, peer, stranger, conn]) await new Promise(r => x.close(r));
return out;
"#####),
        r#####"{"withoutBroadcast":"EACCES","withBroadcast":"sent","received":["b"],"connectedHeard":["from-peer"],"remoteAddress":true,"afterDisconnect":["from-peer"]}"#####
    );
}

#[test]
fn p2c_dns_family_forms() {
    // expected: node22
    assert_eq!(
        p2c_run("dns_family_forms", r#####"const dns = require('node:dns'); const dp = require('node:dns/promises');
const res = {};
const cb = (h, o) => new Promise(r => dns.lookup(h, o, (e, a, f) => r(e ? [e.code, e.message.slice(0, 30)] : [a, f])));
res.v6 = await cb('::1', { family: 'IPv6' });
res.v4str = await cb('127.0.0.1', { family: 'IPv4' });
res.mismatch = await cb('127.0.0.1', { family: 'IPv6' });
res.zero = await cb('127.0.0.1', { family: 0 });
res.num = await cb('::1', 6);
for (const bad of [5, 'foo', -1, 4.5]) {
  try { await cb('127.0.0.1', { family: bad }); res['bad' + bad] = 'no throw'; } catch (e) { res['bad' + bad] = e.code; }
}
try { await dp.lookup('127.0.0.1', { family: 'foo' }); res.pbad = 'no throw'; } catch (e) { res.pbad = e.code; }
try { res.pv6 = await dp.lookup('::1', { family: 'IPv6' }); } catch (e) { res.pv6 = e.code; }
return res;
"#####),
        r#####"{"v6":["::1",6],"v4str":["127.0.0.1",4],"mismatch":["127.0.0.1",4],"zero":["127.0.0.1",4],"num":["::1",6],"bad5":"ERR_INVALID_ARG_VALUE","badfoo":"ERR_INVALID_ARG_VALUE","bad-1":"ERR_INVALID_ARG_VALUE","bad4.5":"ERR_INVALID_ARG_VALUE","pbad":"ERR_INVALID_ARG_VALUE","pv6":"ERR_INVALID_ARG_VALUE"}"#####
    );
}

#[test]
fn p2c_dns_ipv6_literal() {
    // expected: node22
    assert_eq!(
        p2c_run("dns_ipv6_literal", r#####"const net = require('node:net'); const dns = require('node:dns');
const out = {};
out.isIP = ['g::1', '::1', '1::g', '::', '1:2:3:4:5:6:7:8', '1:2:3:4:5:6:7:8:9', 'bad:address', '::1::2', 'fe80::1%eth0'].map(v => net.isIP(v));
const results = [];
for (const h of ['bad:address', 'invalid:address', 'host:80', '::g', '1.2.3.4', '::1']) {
  results.push(await new Promise(r => dns.lookup(h, (e, a, f) => r(e ? e.code : [a, f]))));
}
out.lookup = results;
const p = [];
for (const h of ['bad:address', '::g']) p.push(await require('node:dns/promises').lookup(h).then(v => v, e => e.code));
out.promises = p;
return out;
"#####),
        r#####"{"isIP":[0,6,0,6,6,0,0,0,6],"lookup":["ENOTFOUND","ENOTFOUND","ENOTFOUND","ENOTFOUND",["1.2.3.4",4],["::1",6]],"promises":["ENOTFOUND","ENOTFOUND"]}"#####
    );
}

#[test]
fn p2c_dns_promises_parity() {
    // expected: node22
    assert_eq!(
        p2c_run("dns_promises_parity", r#####"const dns = require('node:dns'); const dp = require('node:dns/promises');
const keys = o => Object.keys(o).sort();
return { same: dns.promises === dp, sameKeys: JSON.stringify(keys(dns.promises)) === JSON.stringify(keys(dp)),
  types: ['lookup','resolve','resolve4','resolve6','reverse','Resolver','getServers','setServers','getDefaultResultOrder','setDefaultResultOrder'].map(k => typeof dns.promises[k]),
  ctorSame: dns.promises.Resolver === dp.Resolver, ownResolver: typeof new dns.promises.Resolver().resolve4 };
"#####),
        r#####"{"same":true,"sameKeys":true,"types":["function","function","function","function","function","function","function","function","function","function"],"ctorSame":true,"ownResolver":"function"}"#####
    );
}

#[test]
fn p2c_dns_result_order() {
    // expected: node22
    assert_eq!(
        p2c_run("dns_result_order", r#####"const dns = require('node:dns'); const dp = require('node:dns/promises');
const fam = a => a.map(x => x.family);
const out = {};
out.def = dns.getDefaultResultOrder();
const run = (o) => dp.lookup('localhost', Object.assign({ all: true }, o)).then(fam);
out.ipv4first = await run({ order: 'ipv4first' });
out.ipv6first = await run({ order: 'ipv6first' });
out.verbatimFalse = await run({ verbatim: false });
dns.setDefaultResultOrder('ipv6first');
out.after6 = await run({});
out.single6 = (await dp.lookup('localhost')).family;
out.cb6 = await new Promise(r => dns.lookup('localhost', (e, a, f) => r(f)));
dns.setDefaultResultOrder('ipv4first');
out.single4 = (await dp.lookup('localhost')).family;
out.optOverride = (await dp.lookup('localhost', { order: 'ipv6first' })).family;
try { dns.setDefaultResultOrder('nope'); out.badset = 'no throw'; } catch (e) { out.badset = e.code; }
try { await dp.lookup('localhost', { order: 'nope' }); out.badorder = 'no throw'; } catch (e) { out.badorder = e.code; }
return out;
"#####),
        r#####"{"def":"verbatim","ipv4first":[4,6],"ipv6first":[6,4],"verbatimFalse":[4,6],"after6":[6,4],"single6":6,"cb6":6,"single4":4,"optOverride":6,"badset":"ERR_INVALID_ARG_VALUE","badorder":"ERR_INVALID_ARG_VALUE"}"#####
    );
}

#[test]
fn p2c_http1_client_misc() {
    // expected: node22
    assert_eq!(
        p2c_run("http1_client_misc", r#####"const http = require('node:http'); const net = require('node:net');
const out = {};
// close is emitted once on connection failure
await new Promise(resolve => {
  const probe = net.createServer(); probe.listen(0, '127.0.0.1', () => { const dead = probe.address().port; probe.close(() => {
    const req = http.request({ port: dead, host: '127.0.0.1' }); const ev = [];
    req.on('error', e => ev.push('error:' + e.code)); req.on('close', () => ev.push('close'));
    req.end(); setTimeout(() => { out.failureEvents = ev; resolve(); }, 150);
  }); });
});
// abort is idempotent and recursion-safe
{ const server = http.createServer(() => {}); await new Promise(r => server.listen(0, '127.0.0.1', r));
  const req = http.request({ port: server.address().port, host: '127.0.0.1' }); let aborts = 0; req.on('error', () => {});
  req.on('abort', () => { aborts++; req.abort(); }); req.abort(); req.abort(); await new Promise(r => setTimeout(r, 50)); out.aborts = aborts; out.aborted = req.aborted;
  await new Promise(r => server.close(r)); }
// setTimeout before the socket exists applies once connected
{ const server = http.createServer(() => { /* never responds */ }); await new Promise(r => server.listen(0, '127.0.0.1', r));
  const req = http.request({ port: server.address().port, host: '127.0.0.1' }); const ev = []; req.on('error', () => {});
  req.setTimeout(100, () => { ev.push('timeout'); req.destroy(); }); req.end();
  await new Promise(r => setTimeout(r, 400)); out.requestTimeout = ev; server.closeAllConnections && server.closeAllConnections(); await new Promise(r => server.close(r)); }
// Agent options reach createConnection
{ let seen; class A extends http.Agent { createConnection(o, cb) { seen = o.customFlag; return net.createConnection(o, cb); } }
  const server = http.createServer((req, res) => res.end('ok')); await new Promise(r => server.listen(0, '127.0.0.1', r));
  const agent = new A({ customFlag: 'from-agent' });
  await new Promise((res, rej) => { const req = http.get({ port: server.address().port, host: '127.0.0.1', agent }, r => { r.resume(); r.on('end', res); }); req.on('error', rej); });
  out.agentOptions = seen; agent.destroy(); await new Promise(r => server.close(r)); }
// maxSockets limits concurrent connections
{ let concurrent = 0, maxSeen = 0; const server = http.createServer((req, res) => { concurrent++; maxSeen = Math.max(maxSeen, concurrent); setTimeout(() => { concurrent--; res.end('ok'); }, 80); });
  await new Promise(r => server.listen(0, '127.0.0.1', r)); const agent = new http.Agent({ keepAlive: false, maxSockets: 1 });
  await Promise.all([1, 2, 3].map(() => new Promise((res, rej) => { const req = http.get({ port: server.address().port, host: '127.0.0.1', agent }, r => { r.resume(); r.on('end', res); }); req.on('error', rej); })));
  out.maxConcurrent = maxSeen; agent.destroy(); await new Promise(r => server.close(r)); }
return out;
"#####),
        r#####"{"failureEvents":["error:ECONNREFUSED","close"],"aborts":1,"aborted":true,"requestTimeout":["timeout"],"agentOptions":"from-agent","maxConcurrent":1}"#####
    );
}

#[test]
fn p2c_http1_handler_exception() {
    // expected: node22
    assert_eq!(
        p2c_run("http1_handler_exception", r#####"const http = require('node:http'); const net = require('node:net');
const out = { clientErrors: 0 }; const uncaught = [];
process.once('uncaughtException', e => { uncaught.push(e.message); });
const server = http.createServer((req, res) => { throw new Error('handler boom'); });
server.on('clientError', (err, sock) => { out.clientErrors++; sock.destroy(); });
await new Promise(r => server.listen(0, '127.0.0.1', r));
const c = net.connect(server.address().port, '127.0.0.1', () => c.write('GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n')); c.on('error', () => {}); c.on('data', () => {});
await new Promise(r => setTimeout(r, 300)); c.destroy();
await new Promise(r => server.close(r));
return out;
"#####),
        r#####"{"clientErrors":0}"#####
    );
}

#[test]
fn p2c_http1_server_events() {
    // expected: node22
    assert_eq!(
        p2c_run("http1_server_events", r#####"const http = require('node:http'); const net = require('node:net');
const out = {};
const server = http.createServer(async (req, res) => {
  if (req.url === '/iter') { const parts = []; for await (const chunk of req) parts.push(Buffer.from(chunk).toString()); out.iter = parts.join(''); res.end('ok'); return; }
  if (req.url === '/trailers') { req.on('data', () => {}); req.on('end', () => { out.trailers = [req.trailers, req.rawTrailers]; res.end('ok'); }); return; }
  if (req.url === '/continue') { let body = ''; req.on('data', d => body += d); req.on('end', () => { out.continueBody = body; res.end('ok'); }); return; }
  res.on('finish', () => out.order.push('finish')); res.on('close', () => out.order.push('close')); out.order = [];
  res.end('ok');
});
await new Promise(r => server.listen(0, '127.0.0.1', r)); const port = server.address().port;
const rawKeep = (text) => new Promise((res, rej) => { const bufs = []; const c = net.connect(port, '127.0.0.1', () => c.write(text)); c.on('data', d => bufs.push(d)); c.on('error', rej); setTimeout(() => { c.destroy(); res(Buffer.concat(bufs).toString('latin1')); }, 250); });
// keep-alive response: close is emitted for the response even though the connection stays open
await rawKeep('GET / HTTP/1.1\r\nHost: x\r\n\r\n'); out.responseEvents = out.order;
await rawKeep('POST /iter HTTP/1.1\r\nHost: x\r\nContent-Length: 6\r\nConnection: close\r\n\r\nabcdef');
await rawKeep('POST /trailers HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\nTrailer: X-Sum\r\nConnection: close\r\n\r\n3\r\nabc\r\n0\r\nX-Sum: 99\r\n\r\n');
// Expect: 100-continue without a checkContinue listener is answered automatically
const interim = await new Promise((res, rej) => { const bufs = []; const c = net.connect(port, '127.0.0.1', () => c.write('POST /continue HTTP/1.1\r\nHost: x\r\nContent-Length: 4\r\nExpect: 100-continue\r\nConnection: close\r\n\r\n')); let sent = false; c.on('data', d => { bufs.push(d); if (!sent && Buffer.concat(bufs).toString().includes('100 Continue')) { sent = true; c.write('body'); } }); c.on('error', rej); setTimeout(() => { c.destroy(); res(Buffer.concat(bufs).toString('latin1')); }, 300); });
out.interim = interim.startsWith('HTTP/1.1 100 Continue'); out.final200 = /HTTP\/1\.1 200/.test(interim);
await new Promise(r => server.close(r));
return out;
"#####),
        r#####"{"order":["finish","close"],"responseEvents":["finish","close"],"iter":"abcdef","trailers":[{"x-sum":"99"},["X-Sum","99"]],"continueBody":"body","interim":true,"final200":true}"#####
    );
}

#[test]
fn p2c_http1_server_headers() {
    // expected: node22
    assert_eq!(
        p2c_run("http1_server_headers", r#####"const http = require('node:http'); const net = require('node:net');
const raw = (port, text, waitMs = 200) => new Promise((res, rej) => { const bufs = []; const c = net.connect(port, '127.0.0.1', () => c.write(text)); c.on('data', d => bufs.push(d)); c.on('error', rej); setTimeout(() => { c.destroy(); res(Buffer.concat(bufs)); }, waitMs); });
const out = {};
const seen = [];
const server = http.createServer((req, res) => {
  seen.push([req.url, req.headers.cookie, req.rawHeaders.filter((v, i) => i % 2 === 0 && v.toLowerCase() === 'cookie').length, req.headers['x-multi']]);
  if (req.url === '/array') { res.writeHead(200, ['Content-Type', 'text/plain', 'Set-Cookie', 'a=1', 'Set-Cookie', 'b=2']); res.end('ok'); return; }
  if (req.url === '/nodate') { res.sendDate = false; res.end('ok'); return; }
  if (req.url === '/latin1') { res.setHeader('x-l', 'café'); res.end('ok'); return; }
  res.end('ok');
});
await new Promise(r => server.listen(0, '127.0.0.1', r)); const port = server.address().port;
const text = b => b.toString('latin1');
let r = text(await raw(port, 'GET /cookies HTTP/1.1\r\nHost: x\r\nCookie: a=1\r\nCookie: b=2\r\nX-Multi: 1\r\nX-Multi: 2\r\nConnection: close\r\n\r\n'));
out.cookieJoin = seen[0];
r = text(await raw(port, 'GET /array HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n'));
out.arrayHeaders = [/^content-type: text\/plain/im.test(r), (r.match(/^set-cookie: /gim) || []).length, /\r\nset-cookie: a=1/i.test(r), /\r\nset-cookie: b=2/i.test(r)];
r = text(await raw(port, 'GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n')); out.hasDate = /^date: /im.test(r);
r = text(await raw(port, 'GET /nodate HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n')); out.noDateWhenDisabled = !/^date: /im.test(r);
const lb = await raw(port, 'GET /latin1 HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n'); out.latin1AsUtf8 = lb.toString('hex').includes(Buffer.from('x-l: caf', 'latin1').toString('hex') + 'c3a9');
await new Promise(r => server.close(r));
return out;
"#####),
        r#####"{"cookieJoin":["/cookies","a=1; b=2",2,"1, 2"],"arrayHeaders":[true,2,true,true],"hasDate":true,"noDateWhenDisabled":true,"latin1AsUtf8":true}"#####
    );
}

#[test]
fn p2c_http1_server_timeout() {
    // expected: node22
    assert_eq!(
        p2c_run("http1_server_timeout", r#####"const http = require('node:http'); const net = require('node:net');
const out = { events: [] };
const server = http.createServer((req, res) => res.end('ok'));
server.setTimeout(80, sock => { out.events.push('timeout:' + (sock && typeof sock.destroy)); sock.destroy(); });
await new Promise(r => server.listen(0, '127.0.0.1', r));
const c = net.connect(server.address().port, '127.0.0.1'); c.on('error', () => {});
await new Promise(r => c.on('close', r));
out.timeoutProp = server.timeout;
await new Promise(r => server.close(r));
return out;
"#####),
        r#####"{"events":["timeout:function"],"timeoutProp":80}"#####
    );
}

#[test]
fn p2c_http2_basic_behaviors() {
    // expected: node22
    assert_eq!(
        p2c_run("http2_basic_behaviors", r#####"const http2 = require('node:http2');
const out = {};
const c = http2.constants;
out.constants = [c.NGHTTP2_SESSION_SERVER, c.NGHTTP2_SESSION_CLIENT, c.NGHTTP2_CANCEL, c.NGHTTP2_END_STREAM_FLAG === undefined ? 'n/a' : c.NGHTTP2_FLAG_END_STREAM];
const server = http2.createServer();
const seen = {};
server.on('session', s => { seen.serverType = s.type; });
server.on('stream', (stream, headers, flags) => {
  const p = headers[':path'];
  if (p === '/flags') { seen.flags = flags; seen.hdrKeys = Object.keys(headers).filter(k => !k.startsWith(':')).sort(); stream.respond({ ':status': 200 }); stream.end('ok'); return; }
  if (p === '/sent') { seen.sentBefore = stream.sentHeaders === undefined || Object.keys(stream.sentHeaders).length === 0; stream.respond({ ':status': 201, 'x-a': 'b' }); seen.sentAfter = stream.sentHeaders[':status']; stream.end('x'); return; }
  if (p === '/array') { stream.respond([':status', '202', 'x-arr', '1']); stream.end('x'); return; }
  if (p === '/implicit') { stream.end('implicit'); return; }
  if (p === '/closed') { stream.on('close', () => { try { stream.respond({ ':status': 200 }); seen.closedRespond = 'no throw'; } catch (e) { seen.closedRespond = e.code; } }); return; }
  if (p === '/cookie') { seen.cookie = headers.cookie; seen.proto = [headers.constructor, Object.getPrototypeOf(headers) === null]; stream.respond({ ':status': 200 }); stream.end('ok'); return; }
  stream.respond({ ':status': 200 }); stream.end('default');
});
await new Promise(r => server.listen(0, '127.0.0.1', r));
const port = server.address().port;
const client = http2.connect('http://127.0.0.1:' + port);
client.on('error', () => {});
out.clientType = client.type;
const get = (headers, body, opts) => new Promise((res, rej) => { const req = client.request(headers, opts); const chunks = []; let h; req.on('response', x => { h = x; }); req.setEncoding('utf8'); req.on('data', d => chunks.push(d)); req.on('end', () => res([h && Number(h[':status']), chunks.join('')])); req.on('error', rej); if (body) req.write(body); req.end(); });
out.flags = await get({ ':path': '/flags' }, undefined, { endStream: true }); out.flagsSeen = [seen.flags & 1, seen.hdrKeys];
out.sent = await get({ ':path': '/sent' }); out.sentSeen = [seen.sentBefore, seen.sentAfter];
out.array = await get({ ':path': '/array' });
out.arrayReq = await new Promise((res, rej) => { const req = client.request([':path', '/flags', ':method', 'GET']); req.on('response', h => res(Number(h[':status']))); req.on('error', rej); req.resume(); req.end(); });
out.implicit = await get({ ':path': '/implicit' });
out.cookie = await get({ ':path': '/cookie', cookie: ['a=1', 'b=2'], constructor: 'x' }); out.cookieSeen = [seen.cookie, seen.proto];
const req = client.request({ ':path': '/closed' }); req.on('error', () => {}); req.on('close', () => {}); await new Promise(r => setTimeout(r, 50)); req.close(); await new Promise(r => setTimeout(r, 150)); out.closedRespond = seen.closedRespond;
out.serverType = seen.serverType;
client.close(); await new Promise(r => setTimeout(r, 50)); await new Promise(r => server.close(r));
return out;
"#####),
        r#####"{"constants":[0,1,8,"n/a"],"clientType":1,"flags":[200,"ok"],"flagsSeen":[1,[]],"sent":[201,"x"],"sentSeen":[true,201],"array":[202,"x"],"arrayReq":200,"implicit":[200,"implicit"],"cookie":[200,"ok"],"cookieSeen":["a=1; b=2",["x",true]],"closedRespond":"ERR_HTTP2_INVALID_STREAM","serverType":0}"#####
    );
}

#[test]
fn p2c_http2_goaway_error() {
    // expected: node22
    assert_eq!(
        p2c_run("http2_goaway_error", r#####"const http2 = require('node:http2');
const out = { events: [] };
const server = http2.createServer();
server.on('session', s => { s.on('error', () => {}); s.on('close', () => {}); setTimeout(() => { try { s.goaway(http2.constants.NGHTTP2_PROTOCOL_ERROR); } catch (e) { out.srvGoawayErr = e.code; } }, 80); });
server.on('stream', (stream) => { stream.on('error', () => {}); });
await new Promise(r => server.listen(0, '127.0.0.1', r));
const client = http2.connect('http://127.0.0.1:' + server.address().port);
client.on('goaway', (code, last) => out.events.push('goaway:' + code));
client.on('error', e => out.events.push('error:' + e.code));
await new Promise(r => { client.on('close', () => { out.events.push('close'); r(); }); setTimeout(r, 1500); });
out.destroyed = client.destroyed;
await Promise.race([new Promise(r => server.close(r)), new Promise(r => setTimeout(r, 300))]);
return out;
"#####),
        r#####"{"events":["goaway:1","error:ERR_HTTP2_SESSION_ERROR","close"],"destroyed":true}"#####
    );
}

#[test]
fn p2c_http2_raw_frames() {
    // expected: node22
    assert_eq!(
        p2c_run("http2_raw_frames", r#####"const http2 = require('node:http2'); const net = require('node:net');
const out = {};
const server = http2.createServer();
const done = new Promise(res => { server.on('stream', (stream, headers, flags) => {
  const rec = { path: headers[':path'], endStreamOnHeaders: Boolean(flags & 1), body: '', trailers: null };
  stream.on('data', d => { rec.body += d; });
  stream.on('trailers', t => { rec.trailers = Object.assign({}, t); });
  stream.on('end', () => { out[rec.path] = rec; stream.respond({ ':status': 200 }); stream.end('ok'); if (rec.path === '/last') setTimeout(res, 50); });
  stream.on('error', () => {});
}); });
server.on('session', s => s.on('error', () => {}));
await new Promise(r => server.listen(0, '127.0.0.1', r));
const frame = (type, flags, id, payload) => { const h = Buffer.alloc(9); h[0] = payload.length >>> 16; h[1] = payload.length >>> 8; h[2] = payload.length; h[3] = type; h[4] = flags; h[5] = id >>> 24; h[6] = id >>> 16; h[7] = id >>> 8; h[8] = id; return Buffer.concat([h, payload]); };
const lit = (n, v) => Buffer.concat([Buffer.from([0, n.length]), Buffer.from(n), Buffer.from([v.length]), Buffer.from(v)]);
const hdrs = (list) => Buffer.concat(list.map(([n, v]) => lit(n, v)));
const sock = net.connect(server.address().port, '127.0.0.1');
sock.on('error', () => {}); sock.on('data', () => {});
await new Promise(r => sock.once('connect', r));
const pad = Buffer.from('hello'); const padded = Buffer.concat([Buffer.from([3]), pad, Buffer.from([0, 0, 0])]);
sock.write(Buffer.concat([
  Buffer.from('PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n'), frame(4, 0, 0, Buffer.alloc(0)),
  // stream 1: headers (no END_STREAM), padded DATA, trailers with END_STREAM
  frame(1, 4, 1, hdrs([[':method', 'POST'], [':path', '/padded'], [':scheme', 'http'], [':authority', 'x']])),
  frame(0, 8, 1, padded),
  frame(1, 5, 1, hdrs([['x-sum', '42']])),
  // stream 3: GET with END_STREAM on the HEADERS frame, flags must reach the stream event
  frame(1, 5, 3, hdrs([[':method', 'GET'], [':path', '/last'], [':scheme', 'http'], [':authority', 'x']])),
]));
await done; sock.destroy();
await Promise.race([new Promise(r => server.close(r)), new Promise(r => setTimeout(r, 300))]);
return out;
"#####),
        r#####"{"/padded":{"path":"/padded","endStreamOnHeaders":false,"body":"hello","trailers":{"x-sum":"42"}},"/last":{"path":"/last","endStreamOnHeaders":true,"body":"","trailers":null}}"#####
    );
}

#[test]
fn p2c_http2_server_session_error() {
    // expected: node22
    assert_eq!(
        p2c_run("http2_server_session_error", r#####"const http2 = require('node:http2');
const out = { sessionErrors: [] };
const server = http2.createServer();
server.on('sessionError', (err, session) => { out.sessionErrors.push([err.message, typeof session.close]); });
server.on('session', s => { setTimeout(() => s.emit('error', new Error('boom')), 30); });
await new Promise(r => server.listen(0, '127.0.0.1', r));
const client = http2.connect('http://127.0.0.1:' + server.address().port); client.on('error', () => {});
await new Promise(r => setTimeout(r, 200)); client.close();
await Promise.race([new Promise(r => server.close(r)), new Promise(r => setTimeout(r, 300))]);
return out;
"#####),
        r#####"{"sessionErrors":[["boom","function"]]}"#####
    );
}

#[test]
fn p2c_http2_session_events() {
    // expected: node22
    assert_eq!(
        p2c_run("http2_session_events", r#####"const http2 = require('node:http2');
const out = {};
const c = http2.constants;
const server = http2.createServer();
const seen = { settings: null, pings: [], trailers: null, connectKeys: null, sessionErrors: 0 };
server.on('session', s => {
  s.on('remoteSettings', st => { if (!seen.settings) seen.settings = [st.enablePush, st.initialWindowSize]; });
  s.on('ping', p => seen.pings.push(Buffer.from(p).toString('hex')));
  s.on('error', () => {});
});
server.on('stream', (stream, headers, flags) => {
  const p = headers[':path'], m = headers[':method'];
  if (m === 'CONNECT') { seen.connectKeys = Object.keys(headers).sort(); stream.respond({ ':status': 200 }); stream.end('connected'); return; }
  if (p === '/rst') { stream.on('error', () => {}); stream.close(c.NGHTTP2_INTERNAL_ERROR); return; }
  if (false) { stream.on('trailers', t => { seen.trailers = Object.assign({}, t); }); stream.on('data', () => {}); stream.on('end', () => { stream.respond({ ':status': 200 }); stream.end('done'); }); return; }
  if (p === '/utf8') { const euro = Buffer.from('€€'); stream.respond({ ':status': 200 }); stream.write(euro.subarray(0, 1)); setTimeout(() => { stream.write(euro.subarray(1, 4)); setTimeout(() => stream.end(euro.subarray(4)), 30); }, 30); return; }
  if (p === '/big') { stream.respond({ ':status': 200 }); const big = Buffer.alloc(300000, 97); const ok = stream.write(big); seen.bigWrite = ok; stream.once('drain', () => { seen.drain = true; stream.end(); }); return; }
  stream.respond({ ':status': 200 }); stream.end('ok');
});
await new Promise(r => server.listen(0, '127.0.0.1', r));
const port = server.address().port;
const client = http2.connect('http://127.0.0.1:' + port, { settings: { enablePush: false, initialWindowSize: 123456 } });
client.on('error', () => {});
await new Promise(r => client.once('connect', r));
await new Promise(r => setTimeout(r, 100));
out.initialSettings = seen.settings;
// ping
await new Promise(r => client.ping(Buffer.from('12345678'), r)); await new Promise(r => setTimeout(r, 50)); out.ping = seen.pings;
// CONNECT carries no :path/:scheme
const connectReq = client.request({ ':method': 'CONNECT', ':authority': 'example.com:443' }); connectReq.resume(); connectReq.end(); await new Promise(r => connectReq.on('close', r)); out.connectKeys = seen.connectKeys;
// RST_STREAM with an error code: error before close
{ const req = client.request({ ':path': '/rst' }); req.end(); const ev = []; req.on('response', () => ev.push('response')); req.on('data', () => {}); req.on('error', e => ev.push('error:' + e.code)); await new Promise(r => { req.on('close', () => { ev.push('close:' + req.rstCode); r(); }); }); out.rst = ev; }
// multi-byte characters split across DATA frames
{ const req = client.request({ ':path': '/utf8' }); req.end(); req.setEncoding('utf8'); const parts = []; req.on('data', d => parts.push(d)); await new Promise(r => req.on('close', r)); out.utf8 = [parts.join(''), parts.some(x => x.includes('�'))]; }
// drain after a large write
{ const req = client.request({ ':path': '/big' }); req.end(); req.on('data', () => {}); await new Promise(r => req.on('close', r)); out.big = [seen.bigWrite, seen.drain === true]; }
client.close(); await new Promise(r => setTimeout(r, 80)); await Promise.race([new Promise(r => server.close(r)), new Promise(r => setTimeout(r, 300))]);
return out;
"#####),
        r#####"{"initialSettings":[false,123456],"ping":["3132333435363738"],"connectKeys":[":authority",":method"],"rst":["error:ERR_HTTP2_STREAM_ERROR","close:2"],"utf8":["€€",false],"big":[false,true]}"#####
    );
}

#[test]
fn p2c_net_default_host_events() {
    // expected: node22
    if !p2c_ipv6_available() {
        return;
    }
    assert_eq!(
        p2c_run("net_default_host_events", r#####"const net = require('node:net');
const events = []; let handled = 0; let server; let port;
const closed = new Promise((resolve, reject) => {
  server = net.createServer(socket => { events.push('connection:' + socket.remoteFamily + ':' + (socket.remoteAddress === '::ffff:127.0.0.1')); socket.on('error', reject); socket.on('data', chunk => { events.push('data:' + chunk.toString()); handled++; socket.end('pong' + handled, () => { if (handled === 3) server.close(); }); }); });
  server.on('error', reject);
  server.on('listening', () => { const a = server.address(); events.push('listening:' + a.address + ':' + a.family); port = a.port; });
  server.on('close', () => { events.push('close'); resolve(); });
  server.listen(0, () => {
    (async () => { for (let i = 0; i < 3; i++) { await new Promise((res, rej) => { const c = net.connect(port, '127.0.0.1', () => c.write('ping' + i)); c.on('data', () => {}); c.on('close', res); c.on('error', rej); }); } })();
  });
});
await closed;
return events;
"#####),
        r#####"["listening::::IPv6","connection:IPv6:true","data:ping0","connection:IPv6:true","data:ping1","connection:IPv6:true","data:ping2","close"]"#####
    );
}

#[test]
fn p2c_net_end_finish() {
    // expected: node22
    assert_eq!(
        p2c_run("net_end_finish", r#####"const net = require('node:net');
const out = { order: [] }; let srv;
const s = net.createServer({ allowHalfOpen: true }, sock => { sock.on('error', () => {}); sock.on('data', () => {}); /* keeps connection open, never ends */ srv = sock; });
await new Promise(r => s.listen(0, '127.0.0.1', r));
const c = await new Promise((res, rej) => { const k = net.connect(s.address().port, '127.0.0.1', () => res(k)); k.on('error', rej); });
c.on('finish', () => out.order.push('finish'));
c.on('end', () => out.order.push('end'));
c.on('close', () => out.order.push('close'));
await new Promise(r => c.end('bye', () => { out.order.push('endcb'); r(); }));
await new Promise(r => setTimeout(r, 100));
out.writableFinished = c.writableFinished; out.writableEnded = c.writableEnded; out.destroyedWhileOpen = c.destroyed;
out.finishCount = out.order.filter(x => x === 'finish').length;
const snapshot = JSON.parse(JSON.stringify(out));
c.destroy(); srv.destroy(); await new Promise(r => s.close(r));
return snapshot;
"#####),
        r#####"{"order":["endcb","finish"],"writableFinished":true,"writableEnded":true,"destroyedWhileOpen":false,"finishCount":1}"#####
    );
}

#[test]
fn p2c_net_listen_callbacks() {
    // expected: node22
    assert_eq!(
        p2c_run("net_listen_callbacks", r#####"const net = require('node:net');
const out = {};
const done = (s) => new Promise(r => s.close(r));
const s1 = net.createServer(); out.listen1 = await new Promise(r => s1.listen(0, () => r('cb')));
const p1 = s1.address().port;
out.connect1 = await new Promise((r, j) => { const c = net.connect(p1, () => { c.destroy(); r('cb'); }); c.on('error', j); });
const c2 = await new Promise((r, j) => { const c = net.connect(p1, 'localhost', () => r(c)); c.on('error', j); }); c2.destroy(); out.connect2 = 'cb';
const c3 = await new Promise((r, j) => { const c = net.connect({ port: p1, host: '127.0.0.1' }, () => r(c)); c.on('error', j); }); c3.destroy(); out.connect3 = 'cb';
await done(s1);
const s2 = net.createServer(); out.listen2 = await new Promise(r => s2.listen(0, '127.0.0.1', () => r('cb'))); await done(s2);
const s3 = net.createServer(); out.listen3 = await new Promise(r => s3.listen({ port: 0, host: '127.0.0.1' }, () => r('cb'))); await done(s3);
return out;
"#####),
        r#####"{"listen1":"cb","connect1":"cb","connect2":"cb","connect3":"cb","listen2":"cb","listen3":"cb"}"#####
    );
}

#[test]
fn p2c_net_max_connections() {
    // expected: node22
    assert_eq!(
        p2c_run("net_max_connections", r#####"const net = require('node:net');
const out = { drops: [] };
const s = net.createServer(); s.maxConnections = 1;
s.on('drop', d => out.drops.push([d.remoteAddress, d.remoteFamily, typeof d.remotePort, typeof d.localPort]));
s.on('connection', sock => { sock.on('error', () => {}); });
await new Promise(r => s.listen(0, '127.0.0.1', r));
const port = s.address().port;
const c1 = await new Promise((res, rej) => { const k = net.connect(port, '127.0.0.1', () => res(k)); k.on('error', rej); });
const c2 = net.connect(port, '127.0.0.1'); c2.on('error', () => {});
out.c2closed = await new Promise(r => { c2.on('close', () => r(true)); setTimeout(() => r(false), 1500); });
out.connections = await new Promise(r => s.getConnections((e, n) => r(n)));
c1.destroy(); await new Promise(r => s.close(r));
return out;
"#####),
        r#####"{"drops":[["127.0.0.1","IPv4","number","number"]],"c2closed":true,"connections":1}"#####
    );
}

#[test]
fn p2c_net_remote_peer() {
    // expected: node22
    if !p2c_ipv6_available() {
        return;
    }
    assert_eq!(
        p2c_run("net_remote_peer", r#####"const net = require('node:net');
const out = {};
const s = net.createServer(sock => { out.serverSide4 = [sock.remoteAddress, sock.remoteFamily, typeof sock.remotePort, ]; sock.end('x'); });
await new Promise(r => s.listen(0, '127.0.0.1', r));
const c = await new Promise((res, rej) => { const k = net.connect({ host: 'localhost', port: s.address().port }, () => res(k)); k.on('error', rej); });
await new Promise(r => setTimeout(r, 50));
out.client = [c.remoteAddress, c.remoteFamily, c.remotePort === s.address().port];
c.destroy(); await new Promise(r => s.close(r));
let six, seen6;
try {
  six = net.createServer(sock => { seen6 = [sock.remoteAddress, sock.remoteFamily, typeof sock.remotePort]; sock.end('x'); });
  await new Promise((r, j) => { six.once('error', j); six.listen(0, '::1', r); });
  const c6 = await new Promise((res, rej) => { const k = net.connect(six.address().port, '::1', () => res(k)); k.on('error', rej); k.on('data', () => {}); });
  out.client6 = [c6.remoteAddress, c6.remoteFamily, c6.remotePort === six.address().port];
  await new Promise(r => setTimeout(r, 50));
  out.serverSide6 = seen6; c6.destroy(); await new Promise(r => six.close(r));
} catch (e) { out.ipv6 = 'unavailable:' + e.code; }
return out;
"#####),
        r#####"{"serverSide4":["127.0.0.1","IPv4","number"],"client":["127.0.0.1","IPv4",true],"client6":["::1","IPv6",true],"serverSide6":["::1","IPv6","number"]}"#####
    );
}

#[test]
fn p2c_net_server_listen_state() {
    // expected: node22
    assert_eq!(
        p2c_run("net_server_listen_state", r#####"const net = require('node:net');
const out = {};
const s = net.createServer();
await new Promise(r => s.listen(0, '127.0.0.1', r));
const port = s.address().port;
try { s.listen(0, '127.0.0.1'); out.relisten = 'no throw'; } catch (e) { out.relisten = e.code; }
out.stillSamePort = s.address().port === port;
out.listening = s.listening;
await new Promise(r => s.close(r));
out.afterClose = [s.listening];
// reuse after close is allowed
await new Promise(r => s.listen(0, '127.0.0.1', r)); out.reuse = s.listening; await new Promise(r => s.close(r));
// default host: unspecified address
const d = net.createServer(); await new Promise(r => d.listen(0, r));
const a = d.address(); out.defaultHost = [['::', '0.0.0.0'].includes(a.address), a.family === (a.address === '::' ? 'IPv6' : 'IPv4')];
const c = await new Promise((res, rej) => { const k = net.connect(a.port, '127.0.0.1', () => res(k)); k.on('error', rej); });
out.connectedViaLoopback = true; c.destroy(); await new Promise(r => d.close(r));
return out;
"#####),
        r#####"{"relisten":"ERR_SERVER_ALREADY_LISTEN","stillSamePort":true,"listening":true,"afterClose":[false],"reuse":true,"defaultHost":[true,true],"connectedViaLoopback":true}"#####
    );
}

#[test]
fn p2c_net_set_encoding() {
    // expected: node22
    assert_eq!(
        p2c_run("net_set_encoding", r#####"const net = require('node:net');
const euro = Buffer.from('€€', 'utf8'); // 6 bytes
const out = {};
const s = net.createServer(sock => {
  sock.setEncoding('utf8');
  const chunks = []; const extra = [];
  sock.emit('custom', 1, 2, 3); 
  sock.on('custom', (...a) => extra.push(a)); sock.emit('custom', 1, 2, 3);
  sock.on('data', d => chunks.push(d));
  sock.on('end', () => { out.joined = chunks.join(''); out.types = chunks.map(c => typeof c); out.noBrokenChars = chunks.every(c => !c.includes('�')); out.extra = extra; sock.end(); });
});
await new Promise(r => s.listen(0, '127.0.0.1', r));
const c = await new Promise((res, rej) => { const k = net.connect(s.address().port, '127.0.0.1', () => res(k)); k.on('error', rej); });
c.write(euro.subarray(0, 2)); await new Promise(r => setTimeout(r, 60));
c.write(euro.subarray(2, 4)); await new Promise(r => setTimeout(r, 60));
c.write(euro.subarray(4)); await new Promise(r => setTimeout(r, 60));
c.end();
await new Promise(r => c.on('close', r));
await new Promise(r => s.close(r));
// client side decoding too
const s2 = net.createServer(sock => { sock.write(euro.subarray(0, 1)); setTimeout(() => { sock.write(euro.subarray(1, 4)); setTimeout(() => { sock.end(euro.subarray(4)); }, 60); }, 60); });
await new Promise(r => s2.listen(0, '127.0.0.1', r));
const c2 = net.connect(s2.address().port, '127.0.0.1'); c2.setEncoding('utf8');
const got = []; c2.on('data', d => got.push(d));
await new Promise(r => c2.on('close', r));
out.clientJoined = got.join(''); out.clientBroken = got.some(c => c.includes('�'));
await new Promise(r => s2.close(r));
return out;
"#####),
        r#####"{"joined":"€€","types":["string","string"],"noBrokenChars":true,"extra":[[1,2,3]],"clientJoined":"€€","clientBroken":false}"#####
    );
}

#[test]
fn p2c_net_socketaddress_default() {
    // expected: node22
    assert_eq!(
        p2c_run("net_socketaddress_default", r#####"const net = require('node:net');
const j = (f) => { try { const a = f(); return [a.address, a.family, a.port, a.flowlabel]; } catch (e) { return e.code; } };
return { none: j(() => new net.SocketAddress()), v6: j(() => new net.SocketAddress({ family: 'ipv6' })),
  v4: j(() => new net.SocketAddress({ family: 'ipv4' })), v6addr: j(() => new net.SocketAddress({ address: '::1', family: 'ipv6' })),
  v6noFamily: j(() => new net.SocketAddress({ address: '::1' })), mismatch: j(() => new net.SocketAddress({ address: '::1', family: 'ipv4' })),
  port: j(() => new net.SocketAddress({ family: 'ipv6', port: 8080 })), badFamily: j(() => new net.SocketAddress({ family: 'nope' })) };
"#####),
        r#####"{"none":["127.0.0.1","ipv4",0,0],"v6":["::","ipv6",0,0],"v4":["127.0.0.1","ipv4",0,0],"v6addr":["::1","ipv6",0,0],"v6noFamily":"ERR_INVALID_ADDRESS","mismatch":"ERR_INVALID_ADDRESS","port":["::","ipv6",8080,0],"badFamily":"ERR_INVALID_ARG_VALUE"}"#####
    );
}

#[test]
fn p2c_net_unref_poll_timer() {
    // expected: thaw-only
    assert_eq!(
        p2c_run("net_unref_poll_timer", r#####"// thaw-only: {"refFlagsAfterUnref":[false],"refreffedPumpExists":true}
const net = require('node:net');
const s = net.createServer({ allowHalfOpen: true }, sock => { srv = sock; sock.on('data', () => {}); }); let srv;
await new Promise(r => s.listen(0, '127.0.0.1', r));
const c = await new Promise((res, rej) => { const k = net.connect(s.address().port, '127.0.0.1', () => res(k)); k.on('error', rej); });
await new Promise(r => setTimeout(r, 30));
const orig = globalThis.__thaw_set_timeout_ref; let flags = [];
globalThis.__thaw_set_timeout_ref = function (fn, delay, ref) { if (fn.name === 'pump') flags.push(ref); return orig.apply(this, arguments); };
c.unref(); srv.unref(); s.unref(); await new Promise(r => setTimeout(r, 60));
const afterUnref = Array.from(new Set(flags)); flags = [];
c.ref(); await new Promise(r => setTimeout(r, 60));
const afterRef = flags.includes(true);
globalThis.__thaw_set_timeout_ref = orig; c.destroy(); srv.destroy(); await new Promise(r => s.close(r));
return { refFlagsAfterUnref: afterUnref, refreffedPumpExists: afterRef };
"#####),
        r#####"{"refFlagsAfterUnref":[false],"refreffedPumpExists":true}"#####
    );
}

#[test]
fn p2c_os_dynamic_info() {
    // expected: thaw-only
    assert_eq!(
        p2c_run("os_dynamic_info", r#####"// thaw-only: {"calls":6,"types":["number","number","object","object"]}
const os = require('node:os'); const orig = globalThis.__thaw_os_info; let calls = 0;
globalThis.__thaw_os_info = function () { calls++; return orig.apply(this, arguments); };
const r = [os.freemem(), os.uptime(), os.loadavg(), os.cpus()];
os.freemem(); os.uptime();
globalThis.__thaw_os_info = orig;
return { calls: calls, types: r.map(v => typeof v) };
"#####),
        r#####"{"calls":6,"types":["number","number","object","object"]}"#####
    );
}

#[test]
fn p2c_tls_api_mock() {
    // expected: thaw-only
    assert_eq!(
        p2c_run("tls_api_mock", r#####"// thaw-only: {"calls":[["a.example",443,"a.example","6361","",""],["b.example",8443,"b.example","6361","",""],["c.example",9443,"c.example","6361","",""]],"nullBeforeConnect":[null,null],"proto":"TLSv1.2","cipher":["ECDHE-RSA-AES128-GCM-SHA256","TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256","TLSv1.2"],"copyIsolated":["api.example","api.example",["x","y"]],"afterDestroy":[null,null,null,null]}
const tls = require('node:tls');
const names = ['__thaw_tls_connect_with_options', '__thaw_tls_session_info', '__thaw_tls_peer_certificate', '__thaw_tls_local_certificate', '__thaw_tls_peer_certificate_metadata', '__thaw_tls_local_certificate_metadata', '__thaw_tls_authorization', '__thaw_tls_destroy', '__thaw_tls_poll_read', '__thaw_crypto_hash_hex', 'queueMicrotask', 'setTimeout'];
const originals = names.map(n => globalThis[n]); const queued = []; const calls = [];
try {
  globalThis.__thaw_tls_connect_with_options = function () { calls.push(Array.prototype.slice.call(arguments, 0, 6)); return 'ok:' + (40 + calls.length) + ':'; };
  globalThis.__thaw_tls_session_info = () => JSON.stringify({ protocol: 'TLSv1.2', name: 'ECDHE-RSA-AES128-GCM-SHA256', standardName: 'TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256' });
  globalThis.__thaw_tls_peer_certificate = () => 'aabb'; globalThis.__thaw_tls_local_certificate = () => '';
  globalThis.__thaw_tls_peer_certificate_metadata = () => JSON.stringify({ subject: { CN: ['api.example'] }, issuer: { CN: ['api.example'] }, dnsNames: ['x', 'y'] });
  globalThis.__thaw_tls_local_certificate_metadata = () => '{}';
  globalThis.__thaw_tls_authorization = () => JSON.stringify({ authorized: true, authorizationError: null, chainVerified: true });
  globalThis.__thaw_tls_destroy = () => {}; globalThis.__thaw_tls_poll_read = () => 'pending'; globalThis.__thaw_crypto_hash_hex = () => 'ab'.repeat(32);
  globalThis.queueMicrotask = f => { queued.push(f); }; globalThis.setTimeout = () => 1;
  const out = {};
  const sc = tls.createSecureContext({ ca: 'ca' });
  const s1 = tls.connect({ host: 'a.example', port: 443, secureContext: sc, rejectUnauthorized: false }); s1.on('error', () => {});
  const s2 = tls.connect(8443, 'b.example', { ca: 'ca', rejectUnauthorized: false }, () => {}); s2.on('error', () => {});
  const s3 = tls.connect(9443, 'c.example', { secureContext: sc, rejectUnauthorized: false }); s3.on('error', () => {});
  out.calls = calls;
  const fresh = new tls.TLSSocket(); out.nullBeforeConnect = [fresh.getProtocol(), fresh.getCipher()];
  out.proto = s1.getProtocol(); const c = s1.getCipher(); out.cipher = [c.name, c.standardName, c.version];
  const a = s1.getPeerCertificate(); a.subject.CN = 'tampered'; a.issuer.CN = 'tampered'; a.dnsNames.push('z');
  const b = s1.getPeerCertificate(); out.copyIsolated = [b.subject.CN, b.issuer.CN, b.dnsNames];
  s1.destroy(); out.afterDestroy = [s1.getPeerCertificate(), s1.getCertificate(), s1.getProtocol(), s1.getCipher()];
  return out;
} finally { names.forEach((n, i) => { globalThis[n] = originals[i]; }); }
"#####),
        r#####"{"calls":[["a.example",443,"a.example","6361","",""],["b.example",8443,"b.example","6361","",""],["c.example",9443,"c.example","6361","",""]],"nullBeforeConnect":[null,null],"proto":"TLSv1.2","cipher":["ECDHE-RSA-AES128-GCM-SHA256","TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256","TLSv1.2"],"copyIsolated":["api.example","api.example",["x","y"]],"afterDestroy":[null,null,null,null]}"#####
    );
}

struct P2cPeer(std::process::Child);
impl Drop for P2cPeer {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn p2c_tls_real_peer() {
    // expected: node22 (client run against the same `openssl s_server`)
    use std::process::{Command, Stdio};
    let dir = temp_registry("p2c_tls_peer");
    let openssl = |args: &[&str]| {
        assert!(Command::new("openssl").args(args).current_dir(&dir).stdout(Stdio::null()).stderr(Stdio::null()).status().unwrap().success());
    };
    let ca_args = |name: &str, subject: &str| {
        ["req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", &format!("{name}.key"), "-out", &format!("{name}.pem"), "-days", "2", "-subj", subject, "-addext", "basicConstraints=critical,CA:TRUE", "-addext", "keyUsage=critical,keyCertSign,cRLSign"].map(String::from)
    };
    for (name, subject) in [("ca1", "/CN=P2C CA One"), ("ca2", "/CN=P2C CA Two")] {
        let args = ca_args(name, subject);
        openssl(&args.iter().map(String::as_str).collect::<Vec<_>>());
    }
    openssl(&["req", "-newkey", "rsa:2048", "-nodes", "-keyout", "server.key", "-out", "server.csr", "-subj", "/CN=localhost"]);
    fs::write(dir.join("ext.cnf"), "subjectAltName=DNS:localhost,IP:127.0.0.1\nbasicConstraints=CA:FALSE\nextendedKeyUsage=serverAuth\n").unwrap();
    openssl(&["x509", "-req", "-in", "server.csr", "-CA", "ca2.pem", "-CAkey", "ca2.key", "-CAcreateserial", "-out", "server.pem", "-days", "2", "-extfile", "ext.cnf"]);
    let port = {
        let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        probe.local_addr().unwrap().port()
    };
    let _peer = P2cPeer(
        Command::new("openssl")
            .args(["s_server", "-accept", &port.to_string(), "-cert", "server.pem", "-key", "server.key", "-www", "-quiet"])
            .current_dir(&dir)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let mut ready = false;
    for _ in 0..100 {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            ready = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(ready, "openssl s_server did not start");
    let body = r#####"const tls = require('node:tls'); const fs = require('node:fs');
const d = '__TLS_DIR__'; const ca = fs.readFileSync(d + '/ca2.pem'), ca1 = fs.readFileSync(d + '/ca1.pem');
const out = {};
const c = await new Promise((res, rej) => { const s = tls.connect({ port: __TLS_PORT__, host: 'localhost', ca: [ca1, ca] }, () => res(s)); s.on('error', rej); s.on('data', () => {}); });
out.authorized = c.authorized; out.protocol = c.getProtocol(); const k = c.getCipher(); out.cipher = [k.name, k.standardName, k.version];
const pc = c.getPeerCertificate(); out.peer = [pc.subject.CN, pc.issuer.CN]; pc.subject.CN = 'x'; out.again = c.getPeerCertificate().subject.CN;
c.destroy(); out.after = [c.getPeerCertificate(), c.getProtocol()];
out.wrong = await new Promise(r => { const s = tls.connect({ port: __TLS_PORT__, host: 'localhost', ca: [ca1] }, () => r('connected')); s.on('error', e => r(typeof e.code)); });
const sc = await new Promise((res, rej) => { const s = tls.connect({ port: __TLS_PORT__, host: 'localhost', secureContext: tls.createSecureContext({ ca }) }, () => res(s)); s.on('error', rej); s.on('data', () => {}); }); out.secureContext = sc.authorized; sc.destroy();
const p = await new Promise((res, rej) => { const s = tls.connect(__TLS_PORT__, 'localhost', { ca }, () => res(s)); s.on('error', rej); s.on('data', () => {}); }); out.positional = p.authorized; p.destroy();
await new Promise(r => setTimeout(r, 50));
return out;
"#####.replace("__TLS_DIR__", dir.to_str().unwrap()).replace("__TLS_PORT__", &port.to_string());
    assert_eq!(p2c_run("tls_real_peer", &body), r#####"{"authorized":true,"protocol":"TLSv1.3","cipher":["TLS_AES_256_GCM_SHA384","TLS_AES_256_GCM_SHA384","TLSv1.3"],"peer":["localhost","P2C CA Two"],"again":"localhost","after":[null,null],"wrong":"string","secureContext":true,"positional":true}"#####);
    let _ = fs::remove_dir_all(&dir);
}
