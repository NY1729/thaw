const src = { a: 1, d: new Date(5), m: new Map([[1, { z: 2 }]]), s: new Set([1, 2]), r: /x/gi, arr: [1, 2, 3], u8: new Uint8Array([1, 2]) };
src.self = src;
const c = structuredClone(src);
console.log(c !== src, c.self === c, c.d instanceof Date, c.d.getTime(), c.m.get(1).z, c.s.size, c.r.source, c.r.flags, c.arr.length, Array.from(c.u8).join());
try { structuredClone(() => {}); } catch (e) { console.log(e.name, e instanceof DOMException); }
try { structuredClone(Symbol('s')); } catch (e) { console.log(e.name); }
const err = structuredClone(new RangeError('bad'));
console.log(err instanceof RangeError, err.message);
console.log(structuredClone(5n), structuredClone(null), structuredClone('s'));
