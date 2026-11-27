// Mixed-type array literals: computed indexes, for...of and read-only Array methods keep each element's JS type.
const a = [undefined, '', 1, null, true];
for (let i = 0; i < a.length; i++) console.log(i, typeof a[i], String(a[i]));
const b = [0n, 'a', 2, undefined];
for (let i = 0; i < b.length; i++) console.log(i, typeof b[i], String(b[i]));
const c = [1, 'x', null, 0n, undefined, true];
for (const v of c) console.log(typeof v);
console.log(typeof c[0], typeof c[1], c.length);
let n = 0;
for (let i = 0; i < c.length; i++) { if (typeof c[i] === 'bigint') n++; }
console.log(n, c.indexOf('x'), c.includes(null));
console.log(c);
function f(arr) { return arr.map(v => typeof v); }
console.log(f([undefined, '', 1, null]).join());
