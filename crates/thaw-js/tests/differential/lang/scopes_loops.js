const fns = [];
for (let i = 0; i < 3; i++) fns.push(() => i);
for (let i = 0, j = 10; i < 2; i++, j--) fns.push(() => i + ':' + j);
console.log(fns.map(f => f()).join());
const g2 = [];
for (const k in { a: 1, b: 2 }) g2.push(() => k);
for (const v of [7, 8]) g2.push(() => v);
console.log(g2.map(f => f()).join());
let total = 0;
for (let i = 0; i < 5; i++) { let sq = i * i; { let sq = -1; total += sq; } total += sq; }
console.log(total);
var x = 'outer';
{ let x = 'inner'; { var y = x; } }
console.log(x, y);
function hoist() { console.log(typeof v1, typeof f1); var v1 = 1; function f1() {} if (true) { var v2 = 2; function f2() { return 'f2'; } } return [v2, typeof f2]; }
console.log(hoist().join());
switch (1) { case 1: { let z = 'case'; console.log(z); } }
let counter = 0; while (counter < 3) { let c = counter++; setTimeout(() => console.log('t', c)); }
for (var vi = 0; vi < 2; vi++) setTimeout(() => console.log('v', vi));
const objs = []; for (let i = 0; i < 3; i++) { const o = { i }; objs.push(() => o.i * 2); }
console.log(objs.map(f => f()).join());
let a = 1; { a = 2; let b = a; { a = b + 1; } } console.log(a);
function counterFactory() { let n = 0; return { inc: () => ++n, get: () => n }; }
const c1 = counterFactory(), c2 = counterFactory(); c1.inc(); c1.inc(); c2.inc(); console.log(c1.get(), c2.get());
const out = []; outer: for (let i = 0; i < 3; i++) { for (let j = 0; j < 3; j++) { if (j == 1) continue outer; out.push(() => [i, j].join('')); } }
console.log(out.map(f => f()).join());
try { console.log(tdz); let tdz = 1; } catch (e) { console.log(e.constructor.name); }
console.log((function () { return typeof arguments; })(), (() => { try { return typeof undeclared; } catch (e) { return 'threw'; } })());
