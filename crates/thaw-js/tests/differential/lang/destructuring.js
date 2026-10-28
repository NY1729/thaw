const { a = 1, b: { c = 2, ...r } = {}, ...others } = { b: { d: 4, e: 5 }, x: 9, y: 8 }; console.log(a, c, r, others);
const [x = 10, , y = 20, ...z] = [undefined, 1, null, 4, 5]; console.log(x, y, z);
function f({ p = 1, q } = {}, [r1, r2 = 'd'] = [], ...rest) { return [p, q, r1, r2, rest]; } console.log(f(), f({q:2}, [1], 3, 4));
let m, n; [m, n] = [1, 2]; [m, n] = [n, m]; console.log(m, n);
({ m, n = 5 } = { m: 'M' }); console.log(m, n);
const key = 'dyn'; const { [key]: dv, [`${key}2`]: dv2 = 'def' } = { dyn: 'D' }; console.log(dv, dv2);
const [[a1, [a2]], { b1: [b2] }] = [[1, [2]], { b1: [3] }]; console.log(a1, a2, b2);
for (const [k, v] of Object.entries({ u: 1, w: 2 })) console.log(k, v);
for (const { id, tags: [t0] } of [{ id: 1, tags: ['a'] }]) console.log(id, t0);
const { length } = 'hello'; const [c0, c1] = 'hi'; console.log(length, c0, c1);
try { const { q } = null; } catch (e) { console.log(e instanceof TypeError); }
const o = {}; ({ a: o.x, b: o['y'] } = { a: 1, b: 2 }); console.log(o);
const fn = ({ a, b } = { a: 1, b: 2 }, [c] = [3]) => a + b + c; console.log(fn(), fn.length);
const { 0: first, [1]: second } = ['f', 's']; console.log(first, second);
