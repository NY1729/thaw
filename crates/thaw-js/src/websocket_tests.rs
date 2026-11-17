//! Host tests for the WebSocket prelude (`web/websocket.js`, ws:// only). Each scenario runs against a tiny
//! RFC 6455 server written over the `__thaw_net_*` natives (FIXTURE, the same API as the node:net fixture the
//! expectations were recorded with on node v22.23.1: `ws.serve(onMessage, start)`).
use crate::Interp;

/// Thaw side of the shared fixture: a raw-socket server that upgrades, echoes frames to `onMessage`, and answers
/// the client's close frame through `conn.end()`. `start(port)` runs asynchronously, as in node.
const FIXTURE: &str = r##"var ws = {};
ws.frame = function (op, payload) {
  var n = payload.length, head;
  if (n < 126) head = Buffer.from([0x80 | op, n]);
  else if (n < 65536) { head = Buffer.alloc(4); head[0] = 0x80 | op; head[1] = 126; head.writeUInt16BE(n, 2); }
  else { head = Buffer.alloc(10); head[0] = 0x80 | op; head[1] = 127; head.writeUInt32BE(0, 2); head.writeUInt32BE(n, 6); }
  return Buffer.concat([head, payload]);
};
ws.serve = function (onMessage, start) {
  var l = __thaw_net_listen('127.0.0.1', 0).split(':');
  var lh = Number(l[1]);
  var srv = { port: Number(l[2]), stopped: false };
  srv.stop = function () { srv.stopped = true; __thaw_net_close_listener(lh); };
  (function accept() {
    if (srv.stopped) return;
    var acc = __thaw_net_poll_accept(lh);
    if (acc === 'err:pending') return setTimeout(accept, 1);
    if (acc.indexOf('ok:') !== 0) return;
    conn(Number(acc.split(':')[1]));
    accept();
  })();
  function conn(h) {
    var c = { up: false, buf: Buffer.alloc(0) };
    c.send = function (op, payload) { __thaw_net_write(h, ws.frame(op, Buffer.from(payload)).toString('hex')); };
    c.raw = function (hex) { __thaw_net_write(h, hex); };
    c.end = function () { __thaw_net_shutdown(h); __thaw_net_destroy(h); };
    (function read() {
      var d = __thaw_net_poll_read(h);
      if (d === 'pending') return setTimeout(read, 1);
      if (d.indexOf('ok:') !== 0) return;
      c.buf = Buffer.concat([c.buf, Buffer.from(d.slice(3), 'hex')]);
      if (!c.up) {
        var end = c.buf.indexOf('\r\n\r\n');
        if (end < 0) return read();
        var head = c.buf.slice(0, end).toString();
        var key = /Sec-WebSocket-Key: (.*)/i.exec(head)[1].trim();
        var digest = new Uint8Array(20);
        __thaw_hash('sha1', new TextEncoder().encode(key + '258EAFA5-E914-47DA-95CA-C5AB0DC85B11'), digest);
        var proto = /Sec-WebSocket-Protocol: ([^,\r\n]*)/i.exec(head);
        __thaw_net_write(h, Buffer.from('HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n' + (proto ? 'Sec-WebSocket-Protocol: ' + proto[1].trim() + '\r\n' : '') + 'Sec-WebSocket-Accept: ' + Buffer.from(digest).toString('base64') + '\r\n\r\n').toString('hex'));
        c.buf = c.buf.slice(end + 4); c.up = true;
      }
      while (c.buf.length >= 2) {
        var op = c.buf[0] & 15, masked = c.buf[1] & 128, len = c.buf[1] & 127, off = 2;
        if (len === 126) { if (c.buf.length < 4) break; len = c.buf.readUInt16BE(2); off = 4; }
        else if (len === 127) { if (c.buf.length < 10) break; len = c.buf.readUInt32BE(6); off = 10; }
        var need = off + (masked ? 4 : 0) + len;
        if (c.buf.length < need) break;
        var payload = Buffer.from(c.buf.slice(off + (masked ? 4 : 0), need));
        if (masked) for (var i = 0; i < len; i++) payload[i] ^= c.buf[off + (i & 3)];
        c.buf = c.buf.slice(need);
        onMessage(c, op, payload);
      }
      read();
    })();
  }
  setTimeout(function () { start(srv.port); }, 0);
  return srv;
};
"##;

/// Runs `scenario` after the fixture and returns the console lines joined by newlines.
fn run(scenario: &str) -> String {
    let mut it = Interp::new();
    it.run(&format!("{FIXTURE}\n{scenario}"))
        .unwrap_or_else(|e| panic!("script must run: {e:?}"));
    it.run_event_loop()
        .unwrap_or_else(|e| panic!("event loop: {e:?}"));
    // Interp has Drop: move the lines out with mem::take, not by moving the field.
    std::mem::take(&mut it.out).join("\n")
}

const SCENARIO_A: &str = r##"var seen = [];
var srv = ws.serve(function (c, op, p) {
  if (op === 1 || op === 2) { seen.push('op' + op + ':' + p.length); c.send(op, p); }
  else if (op === 8) { seen.push('close:' + (p.length >= 2 ? p.readUInt16BE(0) : 'none') + ':' + p.slice(2).toString()); c.send(8, p); c.end(); }
}, function (port) {
  var w = new WebSocket('ws://127.0.0.1:' + port + '/p?q=1');
  console.log('init', w.url.replace(/:\d+/, ':P'), w.readyState, w.binaryType, JSON.stringify(w.protocol), JSON.stringify(w.extensions), w.bufferedAmount);
  w.addEventListener('open', function (e) { console.log('listener open', e.type, e.constructor.name, w.readyState); });
  w.onopen = function (e) {
    console.log('onopen', e.constructor.name, w.readyState);
    w.send('hi');
    w.send(new Uint8Array([1, 2, 3]));
    console.log('buffered', w.bufferedAmount);
  };
  var n = 0;
  w.onmessage = function (e) {
    n++;
    var d = e.data;
    if (typeof d === 'string') console.log('message text', d, e.constructor.name, e.origin.replace(/\d+$/, 'P'), JSON.stringify(e.lastEventId), e.source, JSON.stringify(e.ports), e.type, e.bubbles, e.cancelable);
    else if (d instanceof Blob) console.log('message blob', d.size, JSON.stringify(d.type), e.constructor.name);
    else console.log('message', d.constructor.name, d.byteLength);
    if (n === 1) { w.binaryType = 'arraybuffer'; w.binaryType = 'bogus'; console.log('binaryType', w.binaryType); w.send(new Uint8Array([4, 5])); }
    if (n === 2) { console.log('ab', e.data.constructor.name); w.close(1000, 'bye'); console.log('closing', w.readyState); }
  };
  w.onclose = function (e) {
    console.log('close', e.code, JSON.stringify(e.reason), e.wasClean, e.constructor.name, e.type, w.readyState);
    srv.stop();
    (function done() { if (seen.some(function (s) { return s.indexOf('close:') === 0; })) console.log('seen', JSON.stringify(seen)); else setTimeout(done, 1); })(); // the server records the client's close frame after the client's own `close` event
  };
});
"##;
const EXPECTED_A: &str = r##"init ws://127.0.0.1:P/p?q=1 0 blob "" "" 0
listener open open Event 1
onopen Event 1
buffered 5
message text hi MessageEvent ws://127.0.0.1:P "" null [] message false false
binaryType blob
message blob 3 "" MessageEvent
ab Blob
closing 2
close 1000 "bye" true CloseEvent close 3
seen ["op1:2","op2:3","op2:2","close:1000:bye"]"##;

#[test]
fn websocket_events_text_binary_and_close_handshake() {
    assert_eq!(run(SCENARIO_A), EXPECTED_A);
}

const SCENARIO_B: &str = r##"var seen = [];
var srv = ws.serve(function (c, op, p) {
  if (op === 1) {
    var t = p.toString();
    seen.push('text:' + t);
    if (t === 'ping') c.send(9, 'p1');
    else if (t === 'frag') { c.raw('0102' + '6162'); c.raw('8003' + '636465'); }
    else if (t === 'srvclose') c.send(8, Buffer.concat([Buffer.from([0x03, 0xe9]), Buffer.from('going away')]));
    else c.send(1, p);
  } else if (op === 10) seen.push('pong:' + p.toString());
  else if (op === 8) { seen.push('close:' + p.readUInt16BE(0)); c.send(8, p); c.end(); }
}, function (port) {
  var w = new WebSocket('ws://127.0.0.1:' + port + '/');
  w.binaryType = 'arraybuffer';
  w.onopen = function () { w.send('ping'); w.send('frag'); };
  w.onmessage = function (e) {
    console.log('message', e.data, e.data.constructor.name);
    if (e.data === 'abcde') w.send('srvclose');
  };
  w.onclose = function (e) {
    console.log('close', e.code, JSON.stringify(e.reason), e.wasClean, w.readyState);
    srv.stop();
    (function done() { if (seen.some(function (s) { return s.indexOf('close:') === 0; })) console.log('seen', JSON.stringify(seen)); else setTimeout(done, 1); })(); // the server records the client's close frame after the client's own `close` event
  };
});
"##;
const EXPECTED_B: &str = r##"message abcde String
close 1001 "going away" true 3
seen ["text:ping","text:frag","pong:p1","text:srvclose","close:1001"]"##;

#[test]
fn websocket_ping_pong_fragments_and_server_close() {
    assert_eq!(run(SCENARIO_B), EXPECTED_B);
}

const SCENARIO_C: &str = r##"function t(label, fn) {
  try { var r = fn(); console.log(label, 'ok', r === undefined ? '' : r); }
  catch (e) { console.log(label, e.constructor.name, e.name, JSON.stringify(e.message), e.code, e instanceof DOMException, e instanceof Error); }
}
t('http', function () { return new WebSocket('http://127.0.0.1:1/').url; });
t('https', function () { return new WebSocket('https://127.0.0.1:1/a?b=2').url; });
t('ftp', function () { return new WebSocket('ftp://127.0.0.1:1/'); });
t('bad url', function () { return new WebSocket('not a url'); });
t('fragment', function () { return new WebSocket('ws://127.0.0.1:1/#f'); });
t('dup protocol', function () { return new WebSocket('ws://127.0.0.1:1/', ['a', 'a']); });
t('bad protocol', function () { return new WebSocket('ws://127.0.0.1:1/', 'a b'); });
t('no args', function () { return new WebSocket(); });
t('no new', function () { return WebSocket('ws://127.0.0.1:1/'); });
t('static', function () { return [WebSocket.CONNECTING, WebSocket.OPEN, WebSocket.CLOSING, WebSocket.CLOSED, WebSocket.length, WebSocket.name].join(','); });
var w = new WebSocket('ws://127.0.0.1:1/');
t('close 1', function () { return w.close(1); });
t('close 999', function () { return w.close(999); });
t('close 1001', function () { return w.close(1001); });
t('close long', function () { return w.close(1000, 'x'.repeat(124)); });
t('close emoji', function () { return w.close(1000, '\u{1F600}'.repeat(31)); });
t('close ok', function () { return w.close(1000, 'x'.repeat(123)); });
console.log('after', w.readyState, Object.prototype.toString.call(w), typeof CloseEvent, typeof ErrorEvent, typeof EventSource);
"##;
const EXPECTED_C: &str = r##"http ok ws://127.0.0.1:1/
https ok wss://127.0.0.1:1/a?b=2
ftp DOMException SyntaxError "Expected a ws: or wss: protocol, got ftp:" 12 true true
bad url DOMException SyntaxError "TypeError: Invalid URL" 12 true true
fragment DOMException SyntaxError "Got fragment" 12 true true
dup protocol DOMException SyntaxError "Invalid Sec-WebSocket-Protocol value" 12 true true
bad protocol DOMException SyntaxError "Invalid Sec-WebSocket-Protocol value" 12 true true
no args TypeError TypeError "WebSocket constructor: 1 argument required, but 0 found." undefined false true
no new TypeError TypeError "Class constructor _WebSocket cannot be invoked without 'new'" undefined false true
static ok 0,1,2,3,1,WebSocket
close 1 DOMException InvalidAccessError "invalid code" 15 true true
close 999 DOMException InvalidAccessError "invalid code" 15 true true
close 1001 DOMException InvalidAccessError "invalid code" 15 true true
close long DOMException SyntaxError "Reason must be less than 123 bytes; received 124" 12 true true
close emoji DOMException SyntaxError "Reason must be less than 123 bytes; received 124" 12 true true
close ok ok 
after 2 [object WebSocket] undefined undefined undefined"##;

#[test]
fn websocket_constructor_and_close_validation_errors() {
    assert_eq!(run(SCENARIO_C), EXPECTED_C);
}

const SCENARIO_D: &str = r##"var srv = ws.serve(function () {}, function (port) {
  srv.stop();
  // a reserved port nothing listens on: reusing the just-freed `port` raced other tests binding it (the connect then
  // succeeded and no error event ever fired)
  var w = new WebSocket('ws://127.0.0.1:1/');
  w.onerror = function (e) {
    console.log('error', e.type, e.constructor.name, e.message, e.error.constructor.name, e.error.message);
    // the "no close event follows" check starts from the error itself: a fixed delay from the connect
    // raced the refusal on a saturated CI runner (the error arrived after the timer)
    setTimeout(function () { console.log('later', w.readyState); }, 100);
  };
  w.onclose = function (e) { console.log('close', e.code, e.wasClean); };
});
"##;
const EXPECTED_D: &str = r##"error error ErrorEvent Received network error or non-101 status code. Error Received network error or non-101 status code.
later 0"##;

#[test]
fn websocket_refused_connection_fires_error_only() {
    assert_eq!(run(SCENARIO_D), EXPECTED_D);
}

const SCENARIO_E: &str = r##"var srv = ws.serve(function (c, op, p) { if (op === 1) c.send(1, p); else if (op === 8) { c.send(8, p); c.end(); } }, function (port) {
  var w = new WebSocket('ws://127.0.0.1:' + port + '/');
  try { w.send('early'); } catch (e) { console.log('send', e.constructor.name, e.name, e.message, e.code); }
  console.log('state', w.readyState, w.bufferedAmount);
  w.onopen = function () {
    console.log('open', w.readyState);
    w.send('hi');
    console.log('buffered', w.bufferedAmount);
    setTimeout(function () {
      console.log('flushed', w.bufferedAmount);
      w.close();
    }, 50);
  };
  w.onclose = function (e) { console.log('close', e.code, JSON.stringify(e.reason), e.wasClean, w.readyState); srv.stop(); };
});
"##;
const EXPECTED_E: &str = r##"send DOMException InvalidStateError Sent before connected. 11
state 0 0
open 1
buffered 2
flushed 0
close 1005 "" true 3"##;

#[test]
fn websocket_send_before_open_and_buffered_amount() {
    assert_eq!(run(SCENARIO_E), EXPECTED_E);
}

const SCENARIO_F: &str = r##"var srv = ws.serve(function (c, op, p) {
  if (op === 8) { c.send(8, p); c.end(); }
}, function (port) {
  var w = new WebSocket('ws://127.0.0.1:' + port + '/', ['chat', 'other']);
  console.log('protocol before open', JSON.stringify(w.protocol));
  w.onopen = function () { console.log('open protocol', JSON.stringify(w.protocol)); w.close(); };
  w.onclose = function (e) { console.log('close', e.code, JSON.stringify(e.reason), e.wasClean); srv.stop(); };
});
"##;
const EXPECTED_F: &str = r##"protocol before open ""
open protocol "chat"
close 1005 "" true"##;

#[test]
fn websocket_subprotocol_negotiation_and_close_without_code() {
    assert_eq!(run(SCENARIO_F), EXPECTED_F);
}
