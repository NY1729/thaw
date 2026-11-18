// Worker message wire (ported from thaw-quickjs `platform_globals/workers/structured_clone.js`). A value becomes a
// JSON string `{root, nodes}` that crosses worker threads as plain text; `__thaw_worker_decode` rebuilds it.
// Differences from the QuickJS wire: no uncloneable marks, no blob snapshots, and an `Error` node so error events
// keep name, message, stack and cause. A SharedArrayBuffer is an `SAB` node holding an id into the shared-block table
// (worker.rs): the decoding Interp gets the same memory, so writes are seen by every thread holding it. An ArrayBuffer
// in the transfer list is an `ABT` node: its bytes move to the in-flight table (worker.rs) and the buffer is detached.
const cloneError = message => new DOMException(message, 'DataCloneError');
// `transfer` is a list the caller has validated (__thaw_transfer_check); nothing is moved unless the whole value encodes.
globalThis.__thaw_worker_encode = (value, transfer = []) => {
  const seen = new Map(), nodes = [], taken = [];
  for (const buffer of transfer) {
    // ponytail: a port moves between threads once the cross-thread port protocol exists (worker_threads PR3)
    if (typeof MessagePort === 'function' && buffer instanceof MessagePort) throw cloneError('MessagePort transfer across threads is not supported yet');
    seen.set(buffer, nodes.length);
    taken.push([nodes.length, buffer]);
    nodes.push({ t: 'ABT', id: -1 });
  }
  const encode = input => {
    if (input === undefined) return { p: 'undefined' };
    if (typeof input === 'number' && (!Number.isFinite(input) || Object.is(input, -0))) return { p: Object.is(input, -0) ? '-0' : String(input) };
    if (typeof input === 'bigint') return { p: 'bigint', v: String(input) };
    if (typeof input === 'symbol' || typeof input === 'function') throw cloneError('value cannot be cloned');
    if (input === null || typeof input !== 'object') return { v: input };
    if (seen.has(input)) return { r: seen.get(input) };
    if (input instanceof WeakMap || input instanceof WeakSet || input instanceof Promise) throw cloneError('value cannot be cloned');
    if (typeof MessagePort === 'function' && input instanceof MessagePort) throw cloneError('Object that needs transfer was found in message but not listed in transferList');
    const id = nodes.length; seen.set(input, id); nodes.push(null);
    let node;
    if (Array.isArray(input)) {
      const entries = [];
      for (let index = 0; index < input.length; index++) {
        if (Object.prototype.hasOwnProperty.call(input, index)) entries.push([index, encode(input[index])]);
      }
      node = { t: 'Array', l: input.length, v: entries };
    }
    else if (input instanceof Date) node = { t: 'Date', v: encode(input.getTime()) };
    else if (input instanceof RegExp) node = { t: 'RegExp', s: input.source, f: input.flags };
    else if (input instanceof Map) node = { t: 'Map', v: Array.from(input, ([key, item]) => [encode(key), encode(item)]) };
    else if (input instanceof Set) node = { t: 'Set', v: Array.from(input, item => encode(item)) };
    else if (input instanceof SharedArrayBuffer) node = { t: 'SAB', id: __thaw_sab_export(input) };
    else if (input instanceof ArrayBuffer) node = { t: 'ArrayBuffer', v: Array.from(new Uint8Array(input), byte => byte.toString(16).padStart(2, '0')).join(''), m: input.resizable ? input.maxByteLength : undefined };
    else if (ArrayBuffer.isView(input)) {
      // a length-tracking view (over a resizable buffer) travels as tracking: no length (`tr`)
      const tr = input instanceof DataView ? input.__track === true : __thaw_ta_track(input);
      node = { t: input instanceof DataView ? 'DataView' : 'TypedArray', c: input.constructor.name, b: encode(input.buffer), o: input.byteOffset, l: tr ? undefined : input instanceof DataView ? input.byteLength : input.length, tr };
    }
    else if (input instanceof Error) {
      node = { t: 'Error', n: String(input.name), m: String(input.message), s: input.stack === undefined ? undefined : String(input.stack) };
      if (typeof input.code === 'string') node.k = input.code;
      if ('cause' in input) node.c = encode(input.cause);
    }
    else node = { t: 'Object', v: Object.keys(input).map(key => [key, encode(input[key])]) };
    nodes[id] = node;
    return { r: id };
  };
  const root = encode(value);
  // the whole value encoded: the transferred buffers now move (a throw above leaves them attached)
  for (const [index, buffer] of taken) nodes[index].id = __thaw_ab_take(buffer);
  return JSON.stringify({ root, nodes });
};
globalThis.__thaw_worker_decode = source => {
  const graph = JSON.parse(String(source)), values = new Array(graph.nodes.length);
  const primitive = item => {
    if ('r' in item) return values[item.r];
    if (item.p === 'undefined') return undefined;
    if (item.p === '-0') return -0;
    if (item.p === 'NaN') return NaN;
    if (item.p === 'Infinity') return Infinity;
    if (item.p === '-Infinity') return -Infinity;
    if (item.p === 'bigint') return BigInt(item.v);
    return item.v;
  };
  // pass 1: containers that must exist before any reference to them is filled
  graph.nodes.forEach((node, id) => {
    if (node.t === 'Array') values[id] = new Array(node.l);
    else if (node.t === 'Date') values[id] = new Date(primitive(node.v));
    else if (node.t === 'RegExp') values[id] = new RegExp(node.s, node.f);
    else if (node.t === 'Map') values[id] = new Map();
    else if (node.t === 'Set') values[id] = new Set();
    else if (node.t === 'Object') values[id] = {};
    else if (node.t === 'SAB') values[id] = __thaw_sab_import(node.id);
    else if (node.t === 'ABT') values[id] = __thaw_ab_put(node.id);
    else if (node.t === 'ArrayBuffer') {
      const bytes = new Uint8Array(node.v.length / 2);
      for (let index = 0; index < bytes.length; index++) bytes[index] = parseInt(node.v.slice(index * 2, index * 2 + 2), 16);
      values[id] = node.m === undefined ? bytes.buffer : new ArrayBuffer(bytes.length, { maxByteLength: node.m });
      if (node.m !== undefined) new Uint8Array(values[id]).set(bytes);
    }
    else if (node.t === 'Error') {
      const named = globalThis[node.n];
      const Ctor = typeof named === 'function' && (named === Error || named.prototype instanceof Error) ? named : Error;
      values[id] = new Ctor(node.m);
      if (values[id].name !== node.n) values[id].name = node.n;
      if (node.s !== undefined) Object.defineProperty(values[id], 'stack', { value: node.s, writable: true, configurable: true });
      if (node.k !== undefined) values[id].code = node.k;
    }
  });
  // views need their ArrayBuffer (pass 1)
  graph.nodes.forEach((node, id) => {
    if (node.t === 'DataView') values[id] = new DataView(primitive(node.b), node.o, node.tr ? undefined : node.l);
    else if (node.t === 'TypedArray') values[id] = new globalThis[node.c](primitive(node.b), node.o, node.tr ? undefined : node.l);
  });
  // pass 2: fill contents (references to any node now resolve)
  graph.nodes.forEach((node, id) => {
    if (node.t === 'Array') node.v.forEach(([index, item]) => { values[id][index] = primitive(item); });
    else if (node.t === 'Map') node.v.forEach(([key, item]) => values[id].set(primitive(key), primitive(item)));
    else if (node.t === 'Set') node.v.forEach(item => values[id].add(primitive(item)));
    else if (node.t === 'Object') node.v.forEach(([key, item]) => { values[id][key] = primitive(item); });
    else if (node.t === 'Error' && 'c' in node) values[id].cause = primitive(node.c);
  });
  return primitive(graph.root);
};
