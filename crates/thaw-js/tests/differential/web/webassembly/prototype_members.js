// Tiny module: export f (() => 42), memory (min 1, max 2), tbl (funcref, min 2), g (mutable i32 = 5), and custom section "note".
const BYTES = new Uint8Array([
  0,97,115,109,1,0,0,0,
  1,5,1,96,0,1,127,
  3,2,1,0,
  4,4,1,112,0,2,
  5,4,1,1,1,2,
  6,6,1,127,1,65,5,11,
  7,24,4, 1,102,0,0, 6,109,101,109,111,114,121,2,0, 3,116,98,108,1,0, 1,103,3,0,
  10,6,1,4,0,65,42,11,
  0,8,4,110,111,116,101,1,2,3,
]);
const desc = (o, k) => {
  const x = Object.getOwnPropertyDescriptor(o, k);
  if (!x) return 'NONE';
  const f = ['value' in x ? 'data' : 'accessor', x.writable ? 'w' : '-', x.enumerable ? 'e' : '-', x.configurable ? 'c' : '-'];
  if (typeof x.value === 'function') f.push('fn:' + x.value.name + '/' + x.value.length + (x.value.prototype !== undefined ? '/proto' : ''));
  if (x.get) f.push('get:' + x.get.name + '/' + x.get.length);
  if (x.set) f.push('set:' + x.set.name + '/' + x.set.length);
  if (typeof x.value === 'string') f.push(JSON.stringify(x.value));
  return f.join(' ');
};
const W = WebAssembly;
console.log(desc(W.Module, 'customSections'), desc(W.Module, 'exports'), desc(W.Module, 'imports'));
console.log(desc(W.Memory.prototype, 'buffer'), desc(W.Memory.prototype, 'grow'));
console.log(desc(W.Table.prototype, 'get'), desc(W.Table.prototype, 'grow'), desc(W.Table.prototype, 'set'), desc(W.Table.prototype, 'length'));
console.log(desc(W.Global.prototype, 'value'), desc(W.Global.prototype, 'valueOf'));
console.log(desc(W.Instance.prototype, 'exports'));
for (const C of [W.CompileError, W.LinkError, W.RuntimeError]) console.log(C.name, desc(C.prototype, 'message'), JSON.stringify(new C().message));
const m = new W.Module(BYTES);
console.log(W.Module.customSections(m, 'note').length, W.Module.customSections(m, 'note')[0].byteLength, W.Module.customSections(m, 'nope').length);
console.log(W.Module.exports(m).map((e) => e.name + ':' + e.kind).join(','));
const ex = new W.Instance(m, {}).exports;
console.log(ex.f(), ex.f.name, ex.f.length, ex.memory.buffer.byteLength);
const buf = ex.memory.buffer;
console.log(ex.memory.grow(1), buf.byteLength, ex.memory.buffer.byteLength, buf === ex.memory.buffer);
try { ex.memory.grow(5); } catch (e) { console.log(e.name, e.message); }
const t = ex.tbl;
console.log(t.length, t.get(0), t.grow(1), t.length, t.get(2));
t.set(0, ex.f);
console.log(t.get(0) === ex.f, t.get(0)(), t.get(1));
try { t.get(9); } catch (e) { console.log(e.name, e.message); }
const g = ex.g;
console.log(g.value, g.valueOf());
g.value = 9;
console.log(g.value, g.valueOf());
console.log(Object.prototype.toString.call(ex), Object.keys(ex).join(','));
