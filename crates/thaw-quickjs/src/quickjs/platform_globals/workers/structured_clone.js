  globalThis.__thaw_worker_encode = value => {
    const seen = new Map(), nodes = [];
    const encode = input => {
      if (input === undefined) return { p: 'undefined' };
      if (typeof input === 'number' && !Number.isFinite(input)) return { p: String(input) };
      if (typeof input === 'bigint') return { p: 'bigint', v: String(input) };
      if (input === null || (typeof input !== 'object' && typeof input !== 'function')) return { v: input };
      if (seen.has(input)) return { r: seen.get(input) };
      if (typeof input === 'function') throw new DOMException('value cannot be cloned', 'DataCloneError');
      const id = nodes.length; seen.set(input, id); nodes.push(null);
      let node;
      if (Array.isArray(input)) node = { t: 'Array', v: input.map(encode) };
      else if (input instanceof Date) node = { t: 'Date', v: input.getTime() };
      else if (input instanceof RegExp) node = { t: 'RegExp', s: input.source, f: input.flags, i: input.lastIndex };
      else if (input instanceof Map) node = { t: 'Map', v: Array.from(input, entry => [encode(entry[0]), encode(entry[1])]) };
      else if (input instanceof Set) node = { t: 'Set', v: Array.from(input, encode) };
      else if (typeof globalThis.MessagePort === 'function' && input instanceof globalThis.MessagePort) {
        if (!input.__thawHostPortId) throw new DOMException('MessagePort requires a transfer list', 'DataCloneError');
        node = { t: 'MessagePort', v: String(input.__thawHostPortId) };
      }
      else if (input instanceof ArrayBuffer) node = { t: 'ArrayBuffer', v: Array.from(new Uint8Array(input), byte => byte.toString(16).padStart(2, '0')).join('') };
      else if (ArrayBuffer.isView(input)) node = { t: input instanceof DataView ? 'DataView' : 'TypedArray', c: input.constructor.name, b: encode(input.buffer), o: input.byteOffset, l: input instanceof DataView ? input.byteLength : input.length };
      else node = { t: 'Object', v: Object.keys(input).map(key => [key, encode(input[key])]) };
      nodes[id] = node; return { r: id };
    };
    return JSON.stringify({ root: encode(value), nodes });
  };
  globalThis.__thaw_worker_decode = source => {
    const graph = JSON.parse(String(source)), values = new Array(graph.nodes.length);
    const primitive = item => {
      if ('r' in item) return values[item.r];
      if (item.p === 'undefined') return undefined;
      if (item.p === 'NaN') return NaN;
      if (item.p === 'Infinity') return Infinity;
      if (item.p === '-Infinity') return -Infinity;
      if (item.p === 'bigint') return BigInt(item.v);
      return item.v;
    };
    graph.nodes.forEach((node, id) => {
      if (node.t === 'Array') values[id] = [];
      else if (node.t === 'Date') values[id] = new Date(node.v);
      else if (node.t === 'RegExp') { values[id] = new RegExp(node.s, node.f); values[id].lastIndex = node.i; }
      else if (node.t === 'Map') values[id] = new Map();
      else if (node.t === 'Set') values[id] = new Set();
      else if (node.t === 'MessagePort') {
        if (typeof globalThis.__thaw_create_worker_port !== 'function') throw new DOMException('MessagePort cannot be restored in this context', 'DataCloneError');
        values[id] = globalThis.__thaw_create_worker_port(node.v);
      }
      else if (node.t === 'ArrayBuffer') { const bytes = new Uint8Array(node.v.length / 2); for (let index = 0; index < bytes.length; index++) bytes[index] = parseInt(node.v.slice(index * 2, index * 2 + 2), 16); values[id] = bytes.buffer; }
      else if (node.t === 'Object') values[id] = {};
    });
    const decode = primitive;
    graph.nodes.forEach((node, id) => {
      if (node.t === 'DataView') values[id] = new DataView(decode(node.b), node.o, node.l);
      else if (node.t === 'TypedArray') values[id] = new globalThis[node.c](decode(node.b), node.o, node.l);
    });
    graph.nodes.forEach((node, id) => {
      if (node.t === 'Array') node.v.forEach(item => values[id].push(decode(item)));
      else if (node.t === 'Map') node.v.forEach(entry => values[id].set(decode(entry[0]), decode(entry[1])));
      else if (node.t === 'Set') node.v.forEach(item => values[id].add(decode(item)));
      else if (node.t === 'Object') node.v.forEach(entry => { values[id][entry[0]] = decode(entry[1]); });
    });
    return decode(graph.root);
  };
