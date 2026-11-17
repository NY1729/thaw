//! Resizable ArrayBuffer (`new ArrayBuffer(n, {maxByteLength})`, `resize`, length-tracking and out-of-bounds views,
//! DataView, transfer, structuredClone). The expectations are Node 22.23.1 output: `PROBE` below is the whole
//! generator, `EXPECTED` its stdout, both inlined so the test needs no Node at run time.
//! Regenerate: `node probe.js > expected.txt` in a scratch dir, then paste both (the probe prints one line per case).

use super::*;
use crate::heap_limit;

/// Node 22.23.1 probe source (one console line per case, `t(label, f)` prints `n label: result`).
const PROBE: &str = r####"// Resizable ArrayBuffer probe: each case prints one line. Expectations are Node 22 output.
function show(r) {
  if (r === undefined) return 'undefined';
  if (r === null) return 'null';
  if (typeof r === 'bigint') return r.toString() + 'n';
  if (typeof r === 'object') {
    if (Array.isArray(r)) return '[' + r.map(show).join(' ') + ']';
    if (ArrayBuffer.isView(r)) return Object.prototype.toString.call(r) + '[' + Array.from(new Uint8Array(r.buffer, r.byteOffset, r.byteLength)).join(',') + ']';
    if (r instanceof ArrayBuffer) return Object.prototype.toString.call(r) + ' len=' + r.byteLength;
    return Object.prototype.toString.call(r);
  }
  return String(r);
}
let n = 0;
function t(label, f) {
  n++;
  try {
    console.log(n + ' ' + label + ': ' + show(f()));
  } catch (e) {
    console.log(n + ' ' + label + ': threw ' + e.name + ': ' + e.message);
  }
}
function fixedTA(T, b, o, l) { return new T(b, o, l); }

// 1-14: constructor option validation and getters
t('ctor max', () => new ArrayBuffer(4, { maxByteLength: 8 }).byteLength);
t('resizable of rab', () => new ArrayBuffer(4, { maxByteLength: 8 }).resizable);
t('maxByteLength of rab', () => new ArrayBuffer(4, { maxByteLength: 8 }).maxByteLength);
t('resizable of fixed', () => new ArrayBuffer(4).resizable);
t('maxByteLength of fixed', () => new ArrayBuffer(4).maxByteLength);
t('max undefined is fixed', () => new ArrayBuffer(4, { maxByteLength: undefined }).resizable);
t('options number is ignored', () => new ArrayBuffer(4, 5).resizable);
t('options null is ignored', () => new ArrayBuffer(4, null).resizable);
t('len over max', () => new ArrayBuffer(9, { maxByteLength: 8 }));
t('zero max', () => { const b = new ArrayBuffer(0, { maxByteLength: 0 }); return [b.resizable, b.maxByteLength]; });
t('negative max', () => new ArrayBuffer(2, { maxByteLength: -1 }));
t('NaN max is zero', () => new ArrayBuffer(2, { maxByteLength: NaN }));
t('string max is NaN is zero', () => new ArrayBuffer(0, { maxByteLength: 'abc' }).maxByteLength);
t('huge max', () => new ArrayBuffer(2, { maxByteLength: 2 ** 40 }));
t('max over 2^32', () => new ArrayBuffer(2, { maxByteLength: 2 ** 33 }).maxByteLength);

// 15-24: resize
t('resize up', () => { const b = new ArrayBuffer(2, { maxByteLength: 6 }); b.resize(6); return b.byteLength; });
t('resize down', () => { const b = new ArrayBuffer(4, { maxByteLength: 6 }); b.resize(1); return b.byteLength; });
t('resize to max', () => { const b = new ArrayBuffer(2, { maxByteLength: 6 }); return [b.resize(6), b.byteLength]; });
t('resize same', () => { const b = new ArrayBuffer(2, { maxByteLength: 6 }); b.resize(2); return b.byteLength; });
t('resize beyond max', () => new ArrayBuffer(2, { maxByteLength: 6 }).resize(7));
t('resize on fixed', () => new ArrayBuffer(2).resize(1));
t('resize no args', () => { const b = new ArrayBuffer(2, { maxByteLength: 6 }); b.resize(); return b.byteLength; });
t('resize negative', () => new ArrayBuffer(2, { maxByteLength: 6 }).resize(-1));
t('resize keeps and zeroes', () => {
  const b = new ArrayBuffer(2, { maxByteLength: 6 }); const u = new Uint8Array(b); u[0] = 1; u[1] = 2;
  b.resize(4); return [u[0], u[1], u[2], u[3]].join(',');
});
t('shrink then grow zeroes', () => {
  const b = new ArrayBuffer(4, { maxByteLength: 6 }); const u = new Uint8Array(b); u.set([1, 2, 3, 4]);
  b.resize(2); b.resize(4); return Array.from(u).join(',');
});
t('resize valueOf coerces', () => { const b = new ArrayBuffer(2, { maxByteLength: 6 }); b.resize({ valueOf() { return 3; } }); return b.byteLength; });

// 25-30: detached
t('resizable after transfer', () => { const b = new ArrayBuffer(2, { maxByteLength: 6 }); b.transfer(); return b.resizable; });
t('max after transfer', () => { const b = new ArrayBuffer(2, { maxByteLength: 6 }); b.transfer(); return b.maxByteLength; });
t('resize detached', () => { const b = new ArrayBuffer(2, { maxByteLength: 6 }); b.transfer(); return b.resize(1); });
t('detached byteLength', () => { const b = new ArrayBuffer(2, { maxByteLength: 6 }); b.transfer(); return b.byteLength; });
t('tracking view on detached', () => { const b = new ArrayBuffer(2, { maxByteLength: 6 }); const u = new Uint8Array(b); b.transfer(); return [u.length, u.byteLength, u.byteOffset, u[0]].join(','); });
t('ctor on detached', () => { const b = new ArrayBuffer(2, { maxByteLength: 6 }); b.transfer(); return new Uint8Array(b); });

// 31-39: length-tracking views
t('tracking u8 follows resize', () => { const b = new ArrayBuffer(2, { maxByteLength: 6 }); const u = new Uint8Array(b); const a = [u.length]; b.resize(5); a.push(u.length, u.byteLength); b.resize(1); a.push(u.length, u.byteLength, u.byteOffset); return a.join(','); });
t('tracking i16 tail truncated', () => { const b = new ArrayBuffer(4, { maxByteLength: 8 }); const u = new Int16Array(b); const a = [u.length]; b.resize(5); a.push(u.length, u.byteLength); b.resize(3); a.push(u.length, u.byteLength); return a.join(','); });
t('tracking with offset', () => { const b = new ArrayBuffer(4, { maxByteLength: 8 }); const u = new Uint8Array(b, 2); const a = [u.length, u.byteOffset]; b.resize(6); a.push(u.length, u.byteOffset); b.resize(1); a.push(u.length, u.byteOffset); return a.join(','); });
t('tracking view offset past end', () => new Uint8Array(new ArrayBuffer(4, { maxByteLength: 8 }), 5));
t('tracking i16 odd offset', () => new Int16Array(new ArrayBuffer(4, { maxByteLength: 8 }), 1));
t('tracking i16 ok offset', () => { const b = new ArrayBuffer(4, { maxByteLength: 8 }); const u = new Int16Array(b, 2); b.resize(7); return u.length; });
t('tracking view writes after grow', () => { const b = new ArrayBuffer(2, { maxByteLength: 6 }); const u = new Uint8Array(b); b.resize(4); u[3] = 9; return u[3]; });
t('tracking view keys', () => { const b = new ArrayBuffer(2, { maxByteLength: 6 }); const u = new Uint8Array(b); b.resize(3); return Object.keys(u).join(','); });
t('tracking view element past length', () => { const b = new ArrayBuffer(2, { maxByteLength: 6 }); const u = new Uint8Array(b); b.resize(1); return [u[1], u.at(-1)].join(','); });

// 40-50: fixed-length views going out of bounds
t('fixed OOB length', () => { const b = new ArrayBuffer(4, { maxByteLength: 8 }); const u = new Uint8Array(b, 0, 4); b.resize(2); return [u.length, u.byteLength, u.byteOffset].join(','); });
t('fixed OOB reads', () => { const b = new ArrayBuffer(4, { maxByteLength: 8 }); const u = new Uint8Array(b, 0, 4); b.resize(2); return [u[0], u[3]].join(','); });
t('fixed OOB keys', () => { const b = new ArrayBuffer(4, { maxByteLength: 8 }); const u = new Uint8Array(b, 0, 4); b.resize(2); return Object.keys(u).length; });
t('fixed OOB back in bounds', () => { const b = new ArrayBuffer(4, { maxByteLength: 8 }); const u = new Uint8Array(b, 0, 4); u[1] = 7; b.resize(2); u[1] = 9; b.resize(4); return [u.length, u.byteLength, u[1]].join(','); });
t('fixed OOB offset view', () => { const b = new ArrayBuffer(4, { maxByteLength: 8 }); const u = new Uint8Array(b, 2, 2); b.resize(3); return [u.length, u.byteOffset, u[0]].join(','); });
t('fixed OOB write ignored', () => { const b = new ArrayBuffer(4, { maxByteLength: 8 }); const u = new Uint8Array(b, 0, 4); b.resize(2); u[3] = 5; b.resize(4); return u[3]; });
t('fixed OOB fill', () => { const b = new ArrayBuffer(4, { maxByteLength: 8 }); const u = new Uint8Array(b, 0, 4); b.resize(2); u.fill(1); return 'no throw'; });
t('fixed OOB set', () => { const b = new ArrayBuffer(4, { maxByteLength: 8 }); const u = new Uint8Array(b, 0, 4); b.resize(2); u.set([1]); return 'no throw'; });
t('fixed OOB join', () => { const b = new ArrayBuffer(4, { maxByteLength: 8 }); const u = new Uint8Array(b, 0, 4); b.resize(2); return u.join(','); });
t('fixed OOB iteration', () => { const b = new ArrayBuffer(4, { maxByteLength: 8 }); const u = new Uint8Array(b, 0, 4); b.resize(2); return [...u].join(','); });
t('fixed OOB map', () => { const b = new ArrayBuffer(4, { maxByteLength: 8 }); const u = new Uint8Array(b, 0, 4); b.resize(2); return u.map(x => x).length; });
t('fixed OOB slice', () => { const b = new ArrayBuffer(4, { maxByteLength: 8 }); const u = new Uint8Array(b, 0, 4); b.resize(2); return u.slice().length; });
t('fixed OOB subarray', () => { const b = new ArrayBuffer(4, { maxByteLength: 8 }); const u = new Uint8Array(b, 0, 4); b.resize(2); return u.subarray(0).length; });
t('fixed OOB sort', () => { const b = new ArrayBuffer(4, { maxByteLength: 8 }); const u = new Uint8Array(b, 0, 4); b.resize(2); u.sort(); return 'no throw'; });
t('fixed OOB reverse', () => { const b = new ArrayBuffer(4, { maxByteLength: 8 }); const u = new Uint8Array(b, 0, 4); b.resize(2); u.reverse(); return 'no throw'; });
t('fixed OOB includes', () => { const b = new ArrayBuffer(4, { maxByteLength: 8 }); const u = new Uint8Array(b, 0, 4); b.resize(2); return u.includes(0); });
t('fixed OOB forEach', () => { const b = new ArrayBuffer(4, { maxByteLength: 8 }); const u = new Uint8Array(b, 0, 4); b.resize(2); let c = 0; u.forEach(() => c++); return c; });
t('fixed OOB at', () => { const b = new ArrayBuffer(4, { maxByteLength: 8 }); const u = new Uint8Array(b, 0, 4); b.resize(2); return u.at(0); });
t('fixed OOB copyWithin', () => { const b = new ArrayBuffer(4, { maxByteLength: 8 }); const u = new Uint8Array(b, 0, 4); b.resize(2); u.copyWithin(0, 1); return 'no throw'; });
t('fixed OOB buffer getter', () => { const b = new ArrayBuffer(4, { maxByteLength: 8 }); const u = new Uint8Array(b, 0, 4); b.resize(2); return u.buffer === b; });

// 51-54: subarray / slice / set / iteration on resized buffers
t('subarray of tracking follows', () => { const b = new ArrayBuffer(4, { maxByteLength: 8 }); const u = new Uint8Array(b); const s = u.subarray(1); b.resize(6); return [s.length, s.byteOffset].join(','); });
t('subarray with end is fixed', () => { const b = new ArrayBuffer(4, { maxByteLength: 8 }); const u = new Uint8Array(b); const s = u.subarray(1, 3); b.resize(6); return s.length; });
t('slice result fixed', () => { const b = new ArrayBuffer(4, { maxByteLength: 8 }); const s = new Uint8Array(b).slice(1); return [s.length, s.buffer.resizable].join(','); });
t('ArrayBuffer slice fixed', () => { const b = new ArrayBuffer(4, { maxByteLength: 8 }); const s = b.slice(0, 2); return [s.byteLength, s.resizable, s.maxByteLength].join(','); });
t('set between views after grow', () => { const b = new ArrayBuffer(2, { maxByteLength: 6 }); const u = new Uint8Array(b); b.resize(4); const w = new Uint8Array(4); w.set([1, 2, 3, 4]); u.set(w.subarray(0, 4)); return Array.from(u).join(','); });
t('set overflow after shrink', () => { const b = new ArrayBuffer(4, { maxByteLength: 8 }); const u = new Uint8Array(b); b.resize(2); u.set([1, 2, 3]); return 'no throw'; });
t('iteration sees growth', () => { const b = new ArrayBuffer(2, { maxByteLength: 6 }); const u = new Uint8Array(b); const seen = []; for (const x of u) { seen.push(x); if (seen.length === 1) b.resize(4); } return seen.length; });
t('iteration ends on shrink', () => { const b = new ArrayBuffer(4, { maxByteLength: 8 }); const u = new Uint8Array(b); const seen = []; for (const x of u) { seen.push(x); if (seen.length === 1) b.resize(2); } return seen.length; });
t('forEach mid resize', () => { const b = new ArrayBuffer(4, { maxByteLength: 8 }); const u = new Uint8Array(b); let c = 0; u.forEach(() => { c++; if (c === 1) b.resize(2); }); return c; });
t('fill value coerce resizes', () => { const b = new ArrayBuffer(4, { maxByteLength: 8 }); const u = new Uint8Array(b); u.fill({ valueOf() { b.resize(1); return 7; } }); return Array.from(new Uint8Array(b)).join(','); });
t('set source getter resizes', () => { const b = new ArrayBuffer(4, { maxByteLength: 8 }); const u = new Uint8Array(b); const src = { length: 2, get 0() { b.resize(1); return 5; }, 1: 6 }; u.set(src); return Array.from(new Uint8Array(b)).join(','); });

// 55-66: DataView on resizable buffers
t('dv tracking byteLength', () => { const b = new ArrayBuffer(4, { maxByteLength: 8 }); const d = new DataView(b, 1); const a = [d.byteLength]; b.resize(7); a.push(d.byteLength, d.byteOffset); return a.join(','); });
t('dv fixed shrink OOB byteLength', () => { const b = new ArrayBuffer(4, { maxByteLength: 8 }); const d = new DataView(b, 0, 4); b.resize(2); return d.byteLength; });
t('dv fixed shrink OOB byteOffset', () => { const b = new ArrayBuffer(4, { maxByteLength: 8 }); const d = new DataView(b, 2, 2); b.resize(3); return d.byteOffset; });
t('dv OOB getInt8', () => { const b = new ArrayBuffer(4, { maxByteLength: 8 }); const d = new DataView(b, 0, 4); b.resize(2); return d.getInt8(0); });
t('dv OOB setInt8', () => { const b = new ArrayBuffer(4, { maxByteLength: 8 }); const d = new DataView(b, 0, 4); b.resize(2); d.setInt8(0, 1); return 'no throw'; });
t('dv OOB back in bounds', () => { const b = new ArrayBuffer(4, { maxByteLength: 8 }); const d = new DataView(b, 0, 4); b.resize(2); b.resize(4); return [d.byteLength, d.getUint8(3)].join(','); });
t('dv tracking shrink past offset', () => { const b = new ArrayBuffer(4, { maxByteLength: 8 }); const d = new DataView(b, 3); b.resize(1); return d.byteLength; });
t('dv tracking getInt8 after shrink', () => { const b = new ArrayBuffer(4, { maxByteLength: 8 }); const d = new DataView(b, 1); b.resize(1); return d.getInt8(0); });
t('dv tracking getInt8 after grow', () => { const b = new ArrayBuffer(1, { maxByteLength: 8 }); const d = new DataView(b); b.resize(4); d.setUint16(2, 258); return d.getUint8(3); });
t('dv detached byteLength', () => { const b = new ArrayBuffer(4, { maxByteLength: 8 }); const d = new DataView(b); b.transfer(); return d.byteLength; });
t('dv ctor offset past end', () => { const b = new ArrayBuffer(4, { maxByteLength: 8 }); return new DataView(b, 5); });
t('dv ctor fixed past end', () => { const b = new ArrayBuffer(4, { maxByteLength: 8 }); return new DataView(b, 2, 3); });
t('dv of resized getFloat64 out of range', () => { const b = new ArrayBuffer(8, { maxByteLength: 16 }); const d = new DataView(b); b.resize(4); return d.getFloat64(0); });

// 67-72: transfer / transferToFixedLength / structuredClone / brand
t('transfer keeps resizable', () => { const b = new ArrayBuffer(2, { maxByteLength: 6 }); const c = b.transfer(); return [c.resizable, c.maxByteLength, c.byteLength].join(','); });
t('transfer with length', () => { const b = new ArrayBuffer(2, { maxByteLength: 6 }); const c = b.transfer(4); return [c.resizable, c.maxByteLength, c.byteLength].join(','); });
t('transfer over max', () => new ArrayBuffer(2, { maxByteLength: 6 }).transfer(7));
t('transferToFixedLength drops resizable', () => { const b = new ArrayBuffer(2, { maxByteLength: 6 }); const c = b.transferToFixedLength(); return [c.resizable, c.maxByteLength, c.byteLength].join(','); });
t('transferToFixedLength with length', () => { const b = new ArrayBuffer(2, { maxByteLength: 6 }); const c = b.transferToFixedLength(5); return [c.resizable, c.byteLength].join(','); });
t('structuredClone keeps resizable', () => { const b = new ArrayBuffer(2, { maxByteLength: 6 }); const c = structuredClone(b); c.resize(5); return [c.resizable, c.maxByteLength, c.byteLength].join(','); });
t('structuredClone tracking view', () => { const b = new ArrayBuffer(2, { maxByteLength: 6 }); const u = new Uint8Array(b); const c = structuredClone(u); return [c.length, c.buffer.resizable].join(','); });
t('toString tag', () => Object.prototype.toString.call(new ArrayBuffer(2, { maxByteLength: 6 })));
t('isView', () => ArrayBuffer.isView(new ArrayBuffer(1, { maxByteLength: 2 })));
t('resize name and length', () => [ArrayBuffer.prototype.resize.name, ArrayBuffer.prototype.resize.length, Object.getOwnPropertyDescriptor(ArrayBuffer.prototype, 'maxByteLength').get.name].join(','));
t('resize on non-buffer', () => ArrayBuffer.prototype.resize.call({}, 1));
t('maxByteLength on non-buffer', () => Object.getOwnPropertyDescriptor(ArrayBuffer.prototype, 'maxByteLength').get.call({}));
t('resizable on SAB', () => Object.getOwnPropertyDescriptor(ArrayBuffer.prototype, 'resizable').get.call(new SharedArrayBuffer(1)));
t('typed ctor offset past buffer', () => new Uint8Array(new ArrayBuffer(4, { maxByteLength: 8 }), 5));
t('typed ctor length too long', () => new Uint8Array(new ArrayBuffer(4, { maxByteLength: 8 }), 1, 10));
t('typed ctor odd offset', () => new Int16Array(new ArrayBuffer(4, { maxByteLength: 8 }), 1, 1));
t('Buffer.from resized', () => { const b = new ArrayBuffer(2, { maxByteLength: 6 }); const buf = Buffer.from(b); b.resize(5); return buf.length; });
t('TextDecoder tracking', () => { const b = new ArrayBuffer(2, { maxByteLength: 6 }); const u = new Uint8Array(b); u[0] = 104; u[1] = 105; b.resize(3); u[2] = 33; return new TextDecoder().decode(u); });
t('TextDecoder OOB view', () => { const b = new ArrayBuffer(4, { maxByteLength: 8 }); const u = new Uint8Array(b, 0, 4); b.resize(2); return JSON.stringify(new TextDecoder().decode(u)); });
t('getRandomValues tracking', () => { const b = new ArrayBuffer(2, { maxByteLength: 6 }); const u = new Uint8Array(b); b.resize(4); crypto.getRandomValues(u); return u.length; });
t('getRandomValues OOB', () => { const b = new ArrayBuffer(4, { maxByteLength: 8 }); const u = new Uint8Array(b, 0, 4); b.resize(2); return crypto.getRandomValues(u).length; });

// 73-75: inspect
console.log(new ArrayBuffer(2, { maxByteLength: 6 }));
console.log(new Uint8Array(new ArrayBuffer(2, { maxByteLength: 6 })));
console.log(new ArrayBuffer(3, { maxByteLength: 6 }));

// 107-111: more inspect and clone cases
console.log(new ArrayBuffer(2));
const rz = new ArrayBuffer(2, { maxByteLength: 6 }); const rzu = new Uint8Array(rz); rz.resize(3);
console.log(rzu);
const oob = new ArrayBuffer(4, { maxByteLength: 8 }); const oobu = new Uint8Array(oob, 0, 4); oob.resize(2);
console.log(oobu);
t('structuredClone tracking view follows resize', () => { const b = new ArrayBuffer(2, { maxByteLength: 6 }); const u = new Uint8Array(b); const c = structuredClone(u); c.buffer.resize(4); return [c.length, c.byteLength].join(','); });
"####;

/// `node probe.js` stdout under Node 22.23.1.
const EXPECTED: &str = r####"1 ctor max: 4
2 resizable of rab: true
3 maxByteLength of rab: 8
4 resizable of fixed: false
5 maxByteLength of fixed: 4
6 max undefined is fixed: false
7 options number is ignored: false
8 options null is ignored: false
9 len over max: threw RangeError: Invalid array buffer max length
10 zero max: [true 0]
11 negative max: threw RangeError: Invalid array buffer max length
12 NaN max is zero: threw RangeError: Invalid array buffer max length
13 string max is NaN is zero: 0
14 huge max: [object ArrayBuffer] len=2
15 max over 2^32: 8589934592
16 resize up: 6
17 resize down: 1
18 resize to max: [undefined 6]
19 resize same: 2
20 resize beyond max: threw RangeError: ArrayBuffer.prototype.resize: Invalid length parameter
21 resize on fixed: threw TypeError: Method ArrayBuffer.prototype.resize called on incompatible receiver #<ArrayBuffer>
22 resize no args: 0
23 resize negative: threw RangeError: ArrayBuffer.prototype.resize: Invalid length parameter
24 resize keeps and zeroes: 1,2,0,0
25 shrink then grow zeroes: 1,2,0,0
26 resize valueOf coerces: 3
27 resizable after transfer: true
28 max after transfer: 0
29 resize detached: threw TypeError: Cannot perform ArrayBuffer.prototype.resize on a detached ArrayBuffer
30 detached byteLength: 0
31 tracking view on detached: 0,0,0,
32 ctor on detached: threw TypeError: Cannot perform Construct on a detached ArrayBuffer
33 tracking u8 follows resize: 2,5,5,1,1,0
34 tracking i16 tail truncated: 2,2,4,1,2
35 tracking with offset: 2,2,4,2,0,0
36 tracking view offset past end: threw RangeError: Start offset 5 is outside the bounds of the buffer
37 tracking i16 odd offset: threw RangeError: start offset of Int16Array should be a multiple of 2
38 tracking i16 ok offset: 2
39 tracking view writes after grow: 9
40 tracking view keys: 0,1,2
41 tracking view element past length: ,0
42 fixed OOB length: 0,0,0
43 fixed OOB reads: ,
44 fixed OOB keys: 0
45 fixed OOB back in bounds: 4,4,7
46 fixed OOB offset view: 0,0,
47 fixed OOB write ignored: 0
48 fixed OOB fill: threw TypeError: Cannot perform %TypedArray%.prototype.fill on a detached ArrayBuffer
49 fixed OOB set: threw TypeError: Cannot perform %TypedArray%.prototype.set on a detached ArrayBuffer
50 fixed OOB join: threw TypeError: Cannot perform %TypedArray%.prototype.join on a detached ArrayBuffer
51 fixed OOB iteration: threw TypeError: Cannot perform %TypedArray%.prototype.values on a detached ArrayBuffer
52 fixed OOB map: threw TypeError: Cannot perform %TypedArray%.prototype.map on a detached ArrayBuffer
53 fixed OOB slice: threw TypeError: Cannot perform %TypedArray%.prototype.slice on a detached ArrayBuffer
54 fixed OOB subarray: 0
55 fixed OOB sort: threw TypeError: Cannot perform %TypedArray%.prototype.sort on a detached ArrayBuffer
56 fixed OOB reverse: threw TypeError: Cannot perform %TypedArray%.prototype.reverse on a detached ArrayBuffer
57 fixed OOB includes: threw TypeError: Cannot perform %TypedArray%.prototype.includes on a detached ArrayBuffer
58 fixed OOB forEach: threw TypeError: Cannot perform %TypedArray%.prototype.forEach on a detached ArrayBuffer
59 fixed OOB at: threw TypeError: Cannot perform %TypedArray%.prototype.at on a detached ArrayBuffer
60 fixed OOB copyWithin: threw TypeError: Cannot perform %TypedArray%.prototype.copyWithin on a detached ArrayBuffer
61 fixed OOB buffer getter: true
62 subarray of tracking follows: 5,1
63 subarray with end is fixed: 2
64 slice result fixed: 3,false
65 ArrayBuffer slice fixed: 2,false,2
66 set between views after grow: 1,2,3,4
67 set overflow after shrink: threw RangeError: offset is out of bounds
68 iteration sees growth: 4
69 iteration ends on shrink: 2
70 forEach mid resize: 4
71 fill value coerce resizes: 7
72 set source getter resizes: 5
73 dv tracking byteLength: 3,6,1
74 dv fixed shrink OOB byteLength: threw TypeError: Cannot perform get DataView.prototype.byteLength on a detached ArrayBuffer
75 dv fixed shrink OOB byteOffset: threw TypeError: Cannot perform get DataView.prototype.byteOffset on a detached ArrayBuffer
76 dv OOB getInt8: threw TypeError: Cannot perform DataView.prototype.getInt8 on a detached ArrayBuffer
77 dv OOB setInt8: threw TypeError: Cannot perform DataView.prototype.getInt8 on a detached ArrayBuffer
78 dv OOB back in bounds: 4,0
79 dv tracking shrink past offset: threw TypeError: Cannot perform get DataView.prototype.byteLength on a detached ArrayBuffer
80 dv tracking getInt8 after shrink: threw RangeError: Offset is outside the bounds of the DataView
81 dv tracking getInt8 after grow: 2
82 dv detached byteLength: threw TypeError: Cannot perform get DataView.prototype.byteLength on a detached ArrayBuffer
83 dv ctor offset past end: threw RangeError: Start offset 5 is outside the bounds of the buffer
84 dv ctor fixed past end: threw RangeError: Invalid DataView length 3
85 dv of resized getFloat64 out of range: threw RangeError: Offset is outside the bounds of the DataView
86 transfer keeps resizable: true,6,2
87 transfer with length: true,6,4
88 transfer over max: threw RangeError: Invalid array buffer length
89 transferToFixedLength drops resizable: false,2,2
90 transferToFixedLength with length: false,5
91 structuredClone keeps resizable: true,6,5
92 structuredClone tracking view: 2,true
93 toString tag: [object ArrayBuffer]
94 isView: false
95 resize name and length: resize,1,get maxByteLength
96 resize on non-buffer: threw TypeError: Method ArrayBuffer.prototype.resize called on incompatible receiver #<Object>
97 maxByteLength on non-buffer: threw TypeError: Method get ArrayBuffer.prototype.maxByteLength called on incompatible receiver #<Object>
98 resizable on SAB: threw TypeError: Method get ArrayBuffer.prototype.resizable called on incompatible receiver #<SharedArrayBuffer>
99 typed ctor offset past buffer: threw RangeError: Start offset 5 is outside the bounds of the buffer
100 typed ctor length too long: threw RangeError: Invalid typed array length: 10
101 typed ctor odd offset: threw RangeError: start offset of Int16Array should be a multiple of 2
102 Buffer.from resized: 5
103 TextDecoder tracking: hi!
104 TextDecoder OOB view: ""
105 getRandomValues tracking: 4
106 getRandomValues OOB: 0
ArrayBuffer { [Uint8Contents]: <00 00>, [byteLength]: 2 }
Uint8Array(2) [ 0, 0 ]
ArrayBuffer { [Uint8Contents]: <00 00 00>, [byteLength]: 3 }
ArrayBuffer { [Uint8Contents]: <00 00>, [byteLength]: 2 }
Uint8Array(3) [ 0, 0, 0 ]
Uint8Array(0) []
107 structuredClone tracking view follows resize: 4,4
"####;

/// Runs `src` under one engine: the AST interpreter (`Off`), the closure compiler (`On`) and `check`, all with the
/// compiler taking over after the first call so the compiled element paths see these views too.
/// Runs `src` on `it` for its effect, panicking with the source when it throws.
fn must(it: &mut Interp, src: &str) {
    if it.run_sloppy(src).is_err() {
        panic!("script must run: {src}");
    }
}

fn run_lines(src: &str, mode: crate::compile::CompileMode) -> String {
    let mut it = Interp::with_compile_mode(mode);
    it.compile_after = 1;
    match it.run_sloppy(src) {
        Ok(_) => {}
        Err(Ctl::Throw(v)) => panic!("uncaught {}", it.to_str(&v)),
        Err(_) => panic!("stray completion"),
    }
    it.run_event_loop().expect("event loop runs");
    std::mem::take(&mut it.out).join("\n")
}

#[test]
fn resizable_buffer_probe_matches_node_22() {
    use crate::compile::CompileMode;
    for mode in [CompileMode::Off, CompileMode::On, CompileMode::Check] {
        let got = run_lines(PROBE, mode);
        let got: Vec<&str> = got.lines().collect();
        let want: Vec<&str> = EXPECTED.lines().collect();
        assert_eq!(got.len(), want.len(), "{mode:?}: line count");
        for (g, w) in got.iter().zip(want.iter()) {
            assert_eq!(g, w, "{mode:?}");
        }
    }
}

/// Hot functions that index resizable and out-of-bounds views (the compiled element path must agree).
const PROBE2: &str = r####"// Element access inside hot functions (the compiled fast path must agree with the interpreter on rz views).
function readAll(u) { let s = 0; for (let i = 0; i < 6; i++) { const v = u[i]; s += v === undefined ? 100 : v; } return s; }
function writeAll(u, k) { for (let i = 0; i < 6; i++) u[i] = k + i; }
const b = new ArrayBuffer(4, { maxByteLength: 8 });
const u = new Uint8Array(b);
const f = new Uint8Array(b, 0, 4);
const out = [];
for (let r = 0; r < 4; r++) { writeAll(u, r); out.push(readAll(u), readAll(f)); }
b.resize(6); writeAll(u, 10); out.push(readAll(u), readAll(f));
b.resize(3); out.push(readAll(u), readAll(f)); writeAll(f, 50);
out.push(Array.from(new Uint8Array(b)).join(','));
b.resize(4); out.push(readAll(f), f[3]);
const w = new Int16Array(new ArrayBuffer(4, { maxByteLength: 8 }));
for (let r = 0; r < 3; r++) { w[r] = r * 1000; w.buffer.resize(4 + 2 * r); out.push(w.length, w[r]); }
console.log(out.join(' '));
"####;

/// `node probe2.js` stdout under Node 22.23.1.
const EXPECTED2: &str = r####"206 206 210 210 214 214 218 218 75 246 333 600 10,11,12 233 0 2 0 3 1000 4 2000
"####;

#[test]
fn resizable_buffer_hot_element_paths_match_node_22() {
    use crate::compile::CompileMode;
    for mode in [CompileMode::Off, CompileMode::On, CompileMode::Check] {
        let got = run_lines(PROBE2, mode);
        assert_eq!(got, EXPECTED2.trim_end_matches('\n'), "{mode:?}");
    }
}

/// The postMessage wire (worker_wire.js) carries a resizable buffer and its views like structuredClone does.
const PROBE3: &str = r####"// The postMessage wire (worker_wire.js) and structuredClone must agree on resizable buffers and views. Under Node the
// wire is absent, so `copy` is structuredClone (the spec algorithm both sides implement).
const copy = typeof __thaw_worker_decode === 'function' ? (x) => __thaw_worker_decode(__thaw_worker_encode(x)) : (x) => structuredClone(x);
const b = new ArrayBuffer(2, { maxByteLength: 6 });
const u = new Uint8Array(b); u[0] = 7; u[1] = 8;
const f = new Uint8Array(b, 0, 2);
const d = new DataView(b, 1);
const c = copy({ b, u, f, d });
console.log([c.b.resizable, c.b.maxByteLength, c.b.byteLength, c.u.length, c.u[0], c.u[1], c.f.length, c.d.byteLength].join(','));
c.b.resize(5);
console.log([c.u.length, c.f.length, c.d.byteLength, c.b.byteLength, c.u.buffer === c.b].join(','));
b.resize(1);
console.log([copy(new ArrayBuffer(3)).resizable, copy(u).length].join(','));
"####;

/// `node probe3.js` stdout under Node 22.23.1 (structuredClone; the wire is thaw's own and must print the same).
const EXPECTED3: &str = r####"true,6,2,2,7,8,2,1
5,2,4,5,true
false,1
"####;

#[test]
fn resizable_buffer_wire_and_structured_clone_match_node_22() {
    use crate::compile::CompileMode;
    let mut probe_it = Interp::with_compile_mode(CompileMode::Off);
    must(&mut probe_it, "console.log(typeof __thaw_worker_decode)");
    assert_eq!(
        std::mem::take(&mut probe_it.out),
        ["function"],
        "the thaw wire is installed"
    );
    for mode in [CompileMode::Off, CompileMode::On, CompileMode::Check] {
        assert_eq!(
            run_lines(PROBE3, mode),
            EXPECTED3.trim_end_matches('\n'),
            "{mode:?}"
        );
    }
}

/// Accounting: the bytes a resizable buffer holds are charged on resize and credited on shrink, transfer and drop.
#[test]
fn resizable_buffer_resize_charges_and_credits_the_heap_limit() {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            heap_limit::reset();
        }
    }
    let _reset = Reset;
    heap_limit::enable(1 << 30);
    let mut it = Interp::with_compile_mode(crate::compile::CompileMode::Off);
    must(&mut it, "0;");
    let base = heap_limit::used();
    must(
        &mut it,
        "var b = new ArrayBuffer(16, { maxByteLength: 64 });",
    );
    let at16 = heap_limit::used();
    assert!(at16 >= base, "the constructor charges its bytes");
    must(&mut it, "b.resize(64);");
    assert_eq!(heap_limit::used() - at16, 48);
    must(&mut it, "b.resize(8);");
    assert_eq!(
        heap_limit::used() + 8,
        at16,
        "shrink credits the 56 bytes it gave back"
    );
    must(&mut it, "b = b.transfer();");
    assert_eq!(
        heap_limit::used(),
        at16 - 8,
        "transfer moves the 8 bytes: the source is credited, the copy charged"
    );
    must(&mut it, "b = null;");
    assert_eq!(
        heap_limit::used(),
        base,
        "releasing the buffer credits everything it charged"
    );
}
