// WebSocket (RFC 6455 client, Node 22 / undici surface) over the `__thaw_net_*` natives: ws:// only.
// Node-observed behaviour (node v22.23.1): a failure before the upgrade fires `error` only (readyState stays
// CONNECTING, no `close`); an abrupt drop after open fires `close` 1006; the close handshake reports the peer's
// code and reason; bufferedAmount counts unflushed payload bytes. CloseEvent/ErrorEvent are not globals.
// ponytail: wss:// is not wired yet (needs the __thaw_tls_* transport); extensions are never negotiated.
const CONNECTING = 0, OPEN = 1, CLOSING = 2, CLOSED = 3;
const GUID = '258EAFA5-E914-47DA-95CA-C5AB0DC85B11';
const CLOSE_TIMEOUT_MS = 30000;
const states = new WeakMap();
const eventFields = new WeakMap();
const reportFailure = 'Received network error or non-101 status code.';

function domError(message, name) { return new DOMException(message, name); }
function utf8(text) { return new TextEncoder().encode(text); }
function randomBytes(n) { const out = new Uint8Array(n); __thaw_random_fill(out); return out; }
function sha1Base64(bytes) { const out = new Uint8Array(20); __thaw_hash('sha1', bytes, out); return Buffer.from(out).toString('base64'); }
function isToken(text) { return /^[!#$%&'*+\-.^_`|~0-9A-Za-z]+$/.test(text); }

// Internal, non-global event classes (Node: `event.constructor.name` is CloseEvent / ErrorEvent).
class CloseEvent extends Event {
  constructor(type, init = {}) {
    super(type);
    eventFields.set(this, { wasClean: Boolean(init.wasClean), code: init.code | 0, reason: String(init.reason || '') });
  }
  get wasClean() { return eventFields.get(this).wasClean; }
  get code() { return eventFields.get(this).code; }
  get reason() { return eventFields.get(this).reason; }
}
class ErrorEvent extends Event {
  constructor(type, init = {}) {
    super(type);
    eventFields.set(this, { message: String(init.message || ''), error: init.error });
  }
  get message() { return eventFields.get(this).message; }
  get error() { return eventFields.get(this).error; }
  get filename() { return ''; }
  get lineno() { return 0; }
  get colno() { return 0; }
}

function stateOf(self) {
  const st = states.get(self);
  if (!st) throw new TypeError('Illegal invocation');
  return st;
}

// Byte queue: chunks of Uint8Array, `total` bytes in all.
function addChunk(st, bytes) { if (bytes.length) { st.chunks.push(bytes); st.total += bytes.length; } }
function peek(st, n) {
  const out = new Uint8Array(n);
  let off = 0;
  for (const c of st.chunks) { if (off >= n) break; const k = Math.min(c.length, n - off); out.set(c.subarray(0, k), off); off += k; }
  return out;
}
function take(st, n) {
  const out = new Uint8Array(n);
  let off = 0;
  while (off < n) {
    const c = st.chunks[0];
    const k = Math.min(c.length, n - off);
    out.set(c.subarray(0, k), off);
    off += k;
    if (k === c.length) st.chunks.shift(); else st.chunks[0] = c.subarray(k);
  }
  st.total -= n;
  return out;
}

function frameBytes(opcode, payload) {
  const n = payload.length;
  const mask = randomBytes(4);
  let head;
  if (n < 126) head = [0x80 | opcode, 0x80 | n];
  else if (n < 65536) head = [0x80 | opcode, 0x80 | 126, n >>> 8, n & 255];
  else head = [0x80 | opcode, 0x80 | 127, 0, 0, 0, 0, (n >>> 24) & 255, (n >>> 16) & 255, (n >>> 8) & 255, n & 255];
  const out = new Uint8Array(head.length + 4 + n);
  out.set(head, 0);
  out.set(mask, head.length);
  const base = head.length + 4;
  for (let i = 0; i < n; i++) out[base + i] = payload[i] ^ mask[i & 3];
  return out;
}

function writeBytes(st, bytes) {
  if (st.handle === null) return;
  const result = __thaw_net_write(st.handle, Buffer.from(bytes).toString('hex'));
  if (result !== 'ok') socketEnded(st, result);
}

function scheduleFlush(st) {
  if (st.flushTimer !== null) return;
  st.flushTimer = setTimeout(function flush() {
    st.flushTimer = null;
    if (st.handle === null) { st.buffered = 0; return; }
    if (__thaw_net_pending(st.handle) === 0) st.buffered = 0;
    else st.flushTimer = setTimeout(flush, 1);
  }, 0);
}

function sendFrame(st, opcode, payload) {
  st.buffered += payload.length;
  writeBytes(st, frameBytes(opcode, payload));
  scheduleFlush(st);
}

function closePayload(code, reason) {
  if (code === undefined) return new Uint8Array(0);
  const text = utf8(reason);
  const out = new Uint8Array(2 + text.length);
  out[0] = code >>> 8;
  out[1] = code & 255;
  out.set(text, 2);
  return out;
}

// Tear the connection down and dispatch `close` (the established-connection path).
function closeConnection(st, code, reason, wasClean) {
  if (st.readyState === CLOSED) return;
  st.readyState = CLOSED;
  stopSocket(st);
  st.self.dispatchEvent(new CloseEvent('close', { wasClean, code, reason }));
}

function stopSocket(st) {
  if (st.timer !== null) clearTimeout(st.timer);
  if (st.closeTimer !== null) clearTimeout(st.closeTimer);
  st.timer = null;
  st.closeTimer = null;
  if (st.handle !== null) {
    const h = st.handle;
    st.handle = null;
    __thaw_net_shutdown(h);
    __thaw_net_destroy(h);
  }
}

// Failure before the upgrade completed: `error` only, as Node does.
function failConnecting(st, message) {
  st.failed = true;
  stopSocket(st);
  st.self.dispatchEvent(new ErrorEvent('error', { message, error: new Error(message) }));
}

// The socket ended or errored (eof, read or write failure).
function socketEnded(st, reason) {
  if (st.handle === null) return;
  if (st.readyState === CONNECTING) return failConnecting(st, reason.indexOf('err:') === 0 ? reason.slice(4) : reason);
  closeConnection(st, 1006, '', false);
}

// Fail an established connection with `code` (RFC 6455 section 7.1.7).
function failOpen(st, code, reason) {
  if (st.handle !== null && (st.readyState === OPEN || st.readyState === CLOSING)) sendFrame(st, 0x8, closePayload(code, reason));
  closeConnection(st, code, reason, false);
}

// Send (or, when not OPEN, only count) one message. Jobs run in order; a Blob job is async.
function emit(st, opcode, bytes) {
  if (st.readyState === OPEN) sendFrame(st, opcode, bytes);
  else st.buffered += bytes.length;
}
function enqueue(st, job) {
  st.jobs.push(job);
  if (!st.running) drain(st);
}
function drain(st) {
  st.running = true;
  while (st.jobs.length) {
    const pending = st.jobs[0]();
    if (pending instanceof Promise) {
      const next = () => { st.jobs.shift(); drain(st); };
      pending.then(next, next);
      return;
    }
    st.jobs.shift();
  }
  st.running = false;
}

function deliver(st, opcode, bytes) {
  if (st.readyState !== OPEN) return;
  if (opcode === 0x1) {
    let text;
    try { text = new TextDecoder('utf-8', { fatal: true }).decode(bytes); } catch (e) { return failOpen(st, 1007, 'invalid UTF-8'); }
    st.self.dispatchEvent(new MessageEvent('message', { data: text, origin: st.origin }));
    return;
  }
  const data = st.binaryType === 'arraybuffer' ? bytes.buffer : new Blob([bytes]);
  st.self.dispatchEvent(new MessageEvent('message', { data, origin: st.origin }));
}

function handleFrame(st, fin, rsv, opcode, payload) {
  if (rsv) return failOpen(st, 1002, 'reserved bits set');
  if (opcode === 0x8) {
    if (payload.length === 1) return failOpen(st, 1002, 'invalid close frame');
    const code = payload.length >= 2 ? (payload[0] << 8) | payload[1] : 1005;
    let reason = '';
    if (payload.length > 2) {
      try { reason = new TextDecoder('utf-8', { fatal: true }).decode(payload.subarray(2)); } catch (e) { return failOpen(st, 1007, 'invalid UTF-8'); }
    }
    if (st.readyState === OPEN) {
      st.readyState = CLOSING;
      if (st.handle !== null) writeBytes(st, frameBytes(0x8, payload.length >= 2 ? payload.subarray(0, 2) : new Uint8Array(0)));
    }
    closeConnection(st, code, reason, true);
    return;
  }
  if (opcode === 0x9) { if (st.readyState === OPEN) sendFrame(st, 0xA, payload); return; }
  if (opcode === 0xA) return;
  if (opcode !== 0x0 && opcode !== 0x1 && opcode !== 0x2) return failOpen(st, 1002, 'unknown opcode');
  if (opcode === 0x0) {
    if (!st.fragments) return failOpen(st, 1002, 'unexpected continuation');
    st.fragments.push(payload);
    if (!fin) return;
    const parts = st.fragments, kind = st.fragmentOpcode;
    st.fragments = null;
    let size = 0;
    for (const p of parts) size += p.length;
    const whole = new Uint8Array(size);
    let off = 0;
    for (const p of parts) { whole.set(p, off); off += p.length; }
    return deliver(st, kind, whole);
  }
  if (st.fragments) return failOpen(st, 1002, 'expected continuation');
  if (!fin) { st.fragments = [payload]; st.fragmentOpcode = opcode; return; }
  deliver(st, opcode, payload);
}

function parseFrames(st) {
  for (;;) {
    if (st.total < 2 || st.handle === null) return;
    const h = peek(st, Math.min(st.total, 14));
    const fin = (h[0] & 0x80) !== 0, rsv = h[0] & 0x70, opcode = h[0] & 0x0f, masked = (h[1] & 0x80) !== 0;
    let len = h[1] & 0x7f, off = 2;
    if (len === 126) { if (h.length < 4) return; len = (h[2] << 8) | h[3]; off = 4; }
    else if (len === 127) { if (h.length < 10) return; len = 0; for (let i = 2; i < 10; i++) len = len * 256 + h[i]; off = 10; }
    if (masked) return failOpen(st, 1002, 'masked frame from server');
    if (st.total < off + len) return;
    const payload = take(st, off + len).slice(off);
    handleFrame(st, fin, rsv, opcode, payload);
    if (st.readyState === CLOSED) return;
  }
}

// Reads the 101 response head, checks it, and then opens the connection.
function checkHandshake(st) {
  const bytes = peek(st, st.total);
  let end = -1;
  for (let i = 0; i + 3 < bytes.length; i++) {
    if (bytes[i] === 13 && bytes[i + 1] === 10 && bytes[i + 2] === 13 && bytes[i + 3] === 10) { end = i; break; }
  }
  if (end < 0) return false;
  const lines = new TextDecoder('latin1').decode(bytes.subarray(0, end)).split('\r\n');
  take(st, end + 4);
  if (!/^HTTP\/1\.1 101/.test(lines[0])) { failConnecting(st, reportFailure); return true; }
  const headers = new Map();
  for (const line of lines.slice(1)) {
    const i = line.indexOf(':');
    if (i > 0) headers.set(line.slice(0, i).trim().toLowerCase(), line.slice(i + 1).trim());
  }
  const upgrade = (headers.get('upgrade') || '').toLowerCase();
  const connection = (headers.get('connection') || '').toLowerCase().split(',').map(s => s.trim());
  if (upgrade !== 'websocket' || !connection.includes('upgrade')) { failConnecting(st, reportFailure); return true; }
  if (headers.get('sec-websocket-accept') !== st.expectedAccept) { failConnecting(st, 'Incorrect hash received in Sec-WebSocket-Accept header.'); return true; }
  const selected = headers.get('sec-websocket-protocol');
  if (selected !== undefined && !st.protocols.includes(selected)) { failConnecting(st, 'Sec-WebSocket-Protocol mismatch'); return true; }
  if (headers.get('sec-websocket-extensions')) { failConnecting(st, 'Extensions are not supported'); return true; }
  st.readyState = OPEN;
  st.protocol = selected === undefined ? '' : selected;
  st.self.dispatchEvent(new Event('open'));
  return true;
}

function pump(st) {
  st.timer = null;
  if (st.handle === null) return;
  // Read everything available; an eof/error ends the stream only after the bytes before it are framed.
  let ended = null;
  for (;;) {
    const r = __thaw_net_poll_read(st.handle);
    if (r === 'pending') break;
    if (r.indexOf('ok:') === 0) { addChunk(st, Buffer.from(r.slice(3), 'hex')); continue; }
    ended = r;
    break;
  }
  if (st.readyState === CONNECTING) {
    checkHandshake(st);
    if (st.readyState === OPEN) parseFrames(st);
  } else {
    parseFrames(st);
  }
  if (ended !== null) { if (st.handle !== null) socketEnded(st, ended); return; }
  if (st.handle !== null && !st.failed) st.timer = setTimeout(() => pump(st), 1);
}

// Connect, then send the upgrade request; reads and frames are pumped by `pump`.
function connectSocket(st) {
  const op = __thaw_net_connect_start(st.host, st.port);
  (function waitConnect() {
    const r = __thaw_async_poll(op);
    if (r === 'pending') return setTimeout(waitConnect, 1);
    if (r.indexOf('ok:') !== 0) { if (!st.failed) failConnecting(st, reportFailure); return; }
    const h = Number(r.split(':')[1]);
    if (st.failed) { __thaw_net_destroy(h); return; }
    st.handle = h;
    const request = 'GET ' + st.path + ' HTTP/1.1\r\nHost: ' + st.hostHeader + '\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n' +
      'Sec-WebSocket-Key: ' + st.key + '\r\nSec-WebSocket-Version: 13\r\n' +
      (st.protocols.length ? 'Sec-WebSocket-Protocol: ' + st.protocols.join(', ') + '\r\n' : '') + '\r\n';
    writeBytes(st, utf8(request));
    pump(st);
  })();
}

function parseProtocols(protocols) {
  if (protocols === undefined) return [];
  const list = Array.isArray(protocols) ? protocols.map(String) : [String(protocols)];
  const seen = new Set();
  for (const p of list) {
    if (!isToken(p) || seen.has(p)) throw domError('Invalid Sec-WebSocket-Protocol value', 'SyntaxError');
    seen.add(p);
  }
  return list;
}

// Handler attributes: the listener is registered at the first assignment (so it keeps that order among listeners).
function handlerGet(self, type) { return stateOf(self).handlers[type] || null; }
function handlerSet(self, type, value) {
  const record = stateOf(self);
  record.handlers[type] = typeof value === 'function' ? value : null;
  if (record.wrapped[type]) return;
  record.wrapped[type] = true;
  self.addEventListener(type, event => { const h = record.handlers[type]; if (h) h.call(self, event); });
}

class _WebSocket extends EventTarget {
  // ponytail: rest parameters, not `arguments` (thaw-js has no `arguments` inside class constructors); length is set below.
  constructor(...args) {
    super();
    if (args.length < 1) throw new TypeError('WebSocket constructor: 1 argument required, but 0 found.');
    const [url, protocols] = args;
    let u;
    try { u = new URL(String(url)); } catch (e) { throw domError('TypeError: Invalid URL', 'SyntaxError'); }
    if (u.protocol === 'http:') u.protocol = 'ws:';
    else if (u.protocol === 'https:') u.protocol = 'wss:';
    if (u.protocol !== 'ws:' && u.protocol !== 'wss:') throw domError('Expected a ws: or wss: protocol, got ' + u.protocol, 'SyntaxError');
    if (u.hash !== '') throw domError('Got fragment', 'SyntaxError');
    const list = parseProtocols(protocols);
    const secure = u.protocol === 'wss:';
    const host = u.hostname.replace(/^\[|\]$/g, '');
    const st = {
      self: this, readyState: CONNECTING, protocol: '', url: u.href, origin: u.origin, binaryType: 'blob',
      buffered: 0, handle: null, timer: null, closeTimer: null, flushTimer: null, chunks: [], total: 0,
      fragments: null, fragmentOpcode: 0, handlers: {}, wrapped: {}, protocols: list, failed: false, jobs: [], running: false,
      host, port: Number(u.port || (secure ? 443 : 80)), path: (u.pathname || '/') + u.search,
      hostHeader: u.host, key: '', expectedAccept: '', secure,
    };
    const keyBytes = randomBytes(16);
    st.key = Buffer.from(keyBytes).toString('base64');
    st.expectedAccept = sha1Base64(utf8(st.key + GUID));
    states.set(this, st);
    // ponytail: wss:// needs the __thaw_tls_* transport; until then the attempt fails asynchronously so handlers see it.
    if (secure) { setTimeout(() => failConnecting(st, 'wss:// is not supported yet'), 0); return; }
    connectSocket(st);
  }

  // Rest parameters so that `close.length` is 0, as in Node.
  close(...args) {
    const st = stateOf(this);
    const [code, reason] = args;
    if (code !== undefined && code !== 1000 && (code < 3000 || code > 4999)) throw domError('invalid code', 'InvalidAccessError');
    const text = reason === undefined ? '' : String(reason);
    if (utf8(text).length > 123) throw domError('Reason must be less than 123 bytes; received ' + utf8(text).length, 'SyntaxError');
    if (st.readyState === CLOSING || st.readyState === CLOSED) return;
    if (st.readyState === CONNECTING) {
      st.readyState = CLOSING;
      if (!st.failed) failConnecting(st, reportFailure);
      return;
    }
    st.readyState = CLOSING;
    if (st.handle !== null) {
      writeBytes(st, frameBytes(0x8, closePayload(code, text)));
      st.closeTimer = setTimeout(() => { st.closeTimer = null; closeConnection(st, 1006, '', false); }, CLOSE_TIMEOUT_MS);
    }
  }

  send(data) {
    const st = stateOf(this);
    if (st.readyState === CONNECTING) throw domError('Sent before connected.', 'InvalidStateError');
    if (data instanceof Blob) { enqueue(st, () => data.arrayBuffer().then(buf => emit(st, 0x2, new Uint8Array(buf)))); return; }
    if (typeof data === 'string') return enqueue(st, () => emit(st, 0x1, utf8(data)));
    if (data instanceof ArrayBuffer) return enqueue(st, () => emit(st, 0x2, new Uint8Array(data.slice(0))));
    if (ArrayBuffer.isView(data)) return enqueue(st, () => emit(st, 0x2, new Uint8Array(data.buffer.slice(data.byteOffset, data.byteOffset + data.byteLength))));
    enqueue(st, () => emit(st, 0x1, utf8(String(data))));
  }

  get readyState() { return stateOf(this).readyState; }
  get bufferedAmount() { return stateOf(this).buffered; }
  get url() { return stateOf(this).url; }
  get extensions() { stateOf(this); return ''; }
  get protocol() { return stateOf(this).protocol; }
  get onopen() { return handlerGet(this, 'open'); }
  set onopen(v) { handlerSet(this, 'open', v); }
  get onerror() { return handlerGet(this, 'error'); }
  set onerror(v) { handlerSet(this, 'error', v); }
  get onclose() { return handlerGet(this, 'close'); }
  set onclose(v) { handlerSet(this, 'close', v); }
  get onmessage() { return handlerGet(this, 'message'); }
  set onmessage(v) { handlerSet(this, 'message', v); }
  get binaryType() { return stateOf(this).binaryType; }
  // Node (v22.23.1): any value other than 'arraybuffer' leaves 'blob'.
  set binaryType(v) { stateOf(this).binaryType = v === 'arraybuffer' ? 'arraybuffer' : 'blob'; }
}

// Node names the class `_WebSocket` internally (visible in its "Class constructor" TypeError); the global is `WebSocket`.
Object.defineProperty(_WebSocket, 'name', { value: 'WebSocket', configurable: true });
Object.defineProperty(_WebSocket, 'length', { value: 1, configurable: true });
for (const [name, value] of [['CONNECTING', CONNECTING], ['OPEN', OPEN], ['CLOSING', CLOSING], ['CLOSED', CLOSED]]) {
  Object.defineProperty(_WebSocket, name, { value, enumerable: true, writable: false, configurable: false });
  Object.defineProperty(_WebSocket.prototype, name, { value, enumerable: true, writable: false, configurable: false });
}
Object.defineProperty(_WebSocket.prototype, Symbol.toStringTag, { value: 'WebSocket', configurable: true });
// Node: the methods and the IDL attributes are enumerable (the constructor and the statics are not).
for (const name of ['close', 'send', 'readyState', 'bufferedAmount', 'url', 'extensions', 'protocol', 'onopen', 'onerror', 'onclose', 'onmessage', 'binaryType']) {
  Object.defineProperty(_WebSocket.prototype, name, { enumerable: true });
}
Object.defineProperty(globalThis, 'WebSocket', { value: _WebSocket, writable: true, enumerable: false, configurable: true });
