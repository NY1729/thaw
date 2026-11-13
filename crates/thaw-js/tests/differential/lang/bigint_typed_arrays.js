// BigInt64Array / BigUint64Array: construction, element stores (wrap-around, ToBigInt errors), methods, inspect.
const err = (f) => { try { f(); console.log('no error'); } catch (e) { console.log(e.constructor.name, e.message); } };

const a = new BigInt64Array([1n, -2n, -(2n ** 63n)]);
console.log(a);
console.log(a.length, a.byteLength, a.BYTES_PER_ELEMENT, BigInt64Array.BYTES_PER_ELEMENT, BigUint64Array.BYTES_PER_ELEMENT);
console.log(BigInt64Array.name, BigInt64Array.length, BigUint64Array.name, BigUint64Array.length);
console.log(Object.prototype.toString.call(a), String(new BigUint64Array([1n, 2n])), a.join('|'));
console.log(new BigInt64Array(2), new BigUint64Array(0));
console.log(new BigInt64Array(new ArrayBuffer(16), 8, 1));

const u = new BigUint64Array(1);
u[0] = -1n;
console.log(u[0], u[5]);
const w = new BigInt64Array(4);
w[0] = 2n ** 63n;
w[1] = 2n ** 64n + 5n;
w[2] = -(2n ** 63n) - 1n;
w[3] = 7n;
console.log(w.join());
const x = new BigUint64Array(1);
x[0] = -2n;
console.log(x[0], x.join());
const ch = new BigInt64Array(3);
ch[1] = '42';
ch[2] = true;
ch[9] = 5n;
console.log(ch, ch[9], Object.keys(ch));
const shared = new ArrayBuffer(16);
const view = new BigInt64Array(shared);
view[1] = 5n;
console.log(new Uint8Array(shared)[8], new BigUint64Array(shared)[1]);

err(() => { new BigInt64Array(2)[0] = 1; });
err(() => { new BigUint64Array(1)[0] = 1.5; });
err(() => { const t = new BigInt64Array(1); t[0] = undefined; });
err(() => { const t = new BigInt64Array(1); t[0] = null; });
err(() => { const t = new BigInt64Array(1); t[0] = 'x'; });
err(() => new BigInt64Array([1, 2]));
err(() => new BigInt64Array(new Int8Array([1])));
err(() => new Int8Array(new BigInt64Array([1n])));
err(() => new BigInt64Array(2).set([1]));
err(() => new BigInt64Array(2).set(new Int8Array(1)));
err(() => new BigInt64Array(2).set([1n, 2n, 3n]));
err(() => new BigInt64Array(new ArrayBuffer(4)));
err(() => new BigInt64Array(new ArrayBuffer(16), 4));
err(() => JSON.stringify(new BigInt64Array(2)));
err(() => BigInt64Array.from([1, 2]));

const s = new BigInt64Array([5n, 6n, 7n, 8n]);
s.set([1n, 2n], 2);
console.log(s.join());
s.set(new BigInt64Array([9n]), 0);
console.log(s.join());
const sub = s.subarray(1, 3);
sub[0] = -1n;
console.log(s.join(), sub.join(), sub.byteOffset);
console.log(s.slice(-2).join(), s.slice(1, 3) instanceof BigInt64Array);

console.log(BigInt64Array.from([1n, 2n, 3n], (v) => v * -1n).join());
console.log(BigUint64Array.of(1n, 2n ** 64n - 1n).join('|'));
for (const v of new BigUint64Array([10n, 20n])) console.log('it', v);
console.log([...new BigInt64Array([4n]).entries()]);

const m = new BigInt64Array([3n, 1n, 2n]);
console.log(m.map((v) => v * 2n), m.filter((v) => v > 1n));
console.log(m.indexOf(2n), m.indexOf(9n), m.includes(1n), m.includes(1), m.lastIndexOf(3n), m.at(-1));
console.log(m.sort().join(), m.reverse().join(), new BigInt64Array(3).fill(-7n).join());

console.log(String(new BigInt64Array([1n, 2n])), `${new BigUint64Array([3n])}`, new BigInt64Array([1n]).toString());
console.log(new BigInt64Array([-1n]).toString(), String(new BigUint64Array([7n])));

const c = structuredClone(new BigInt64Array([-3n, 4n]));
console.log(c, c.constructor.name);
console.log(structuredClone(new BigUint64Array([2n ** 64n - 1n])));

const dv = new DataView(new ArrayBuffer(8));
dv.setBigInt64(0, -2n, true);
console.log(dv.getBigUint64(0, true), dv.getBigInt64(0, true));
const be = new DataView(new ArrayBuffer(8));
be.setBigUint64(0, 0x0102n);
console.log(be.getUint8(7), be.getUint8(0), be.getBigUint64(0), be.getBigInt64(0, true));
