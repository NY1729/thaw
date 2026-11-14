// Typed arrays and Buffers: CanonicalNumericIndexString keys ('-1', '1.5', '-0', 'Infinity', 'NaN',
// out-of-range integers) are integer-indexed exotic keys: writes are ignored, reads are undefined,
// `in`/hasOwnProperty are false, delete is true, defineProperty throws. '01' and 'length' stay ordinary.
const ctors = [Uint8Array, Float64Array, BigInt64Array, Buffer];
const names = [-1, '1.5', '-0', '01', 'Infinity', 'NaN', 4294967296, 'length', 'foo'];
for (const C of ctors) {
  const isBig = C === BigInt64Array;
  const s = C === Buffer ? Buffer.from([1, 2, 3]) : new C(3);
  const v = isBig ? 5n : 5;
  const out = [];
  for (const k of names) {
    const r = { k: String(k) };
    s[k] = v;
    r.get = s[k] === undefined ? 'undef' : String(s[k]);
    r.in = k in s;
    r.own = s.hasOwnProperty(k);
    r.del = delete s[k];
    out.push(JSON.stringify(r));
  }
  console.log(C.name, out.join(' ; '));
  console.log(C.name, 'keys', Object.keys(s).join(','), 'names', Object.getOwnPropertyNames(s).join(','));
  console.log(C.name, 'json', JSON.stringify(s, (k, x) => (typeof x === 'bigint' ? 'n' + x : x)));
  for (const k of ['-1', '1.5', '-0', 'Infinity', 'NaN', 4294967296, '01']) {
    try {
      Object.defineProperty(s, k, { value: v });
      console.log(C.name, 'define', String(k), 'ok', String(s[k]));
    } catch (e) {
      console.log(C.name, 'define', String(k), e.constructor.name, e.message);
    }
  }
  console.log(C.name, 'after', Object.keys(s).join(','), JSON.stringify(s, (k, x) => (typeof x === 'bigint' ? 'n' + x : x)));
}
const u = new Uint8Array(3);
u.foo = 1;
u['-1'] = 7;
console.log(u.foo, u['-1'], Object.keys(u).join(','), 1 in u, 3 in u, -1 in u);
console.log(Reflect.defineProperty(u, '-1', { value: 1 }), Reflect.has(u, '-1'), Reflect.set(u, '-1', 3), Reflect.get(u, '-1'));
console.log(Reflect.defineProperty(u, '0', { value: 9 }), u[0], Reflect.defineProperty(u, '0', { value: 9, writable: false }));
