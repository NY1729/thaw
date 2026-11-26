// `[]` starts an any[] and `let x = null` an any: later pushes/assignments of any type are fine.
const tests = [];
tests.push({ name: 'a', fn: () => 1 });
tests.push({ name: 'b', fn: () => 2 });
let total = 0;
for (const t of tests) total += t.fn();
console.log(tests.length, total, tests[1].name);

const log = [];
log.push(42, 'x', { k: 1 });
console.log(JSON.stringify(log));

const routes = [];
routes.push({ method: 'GET', prefix: '/a/', h: (p) => p.slice(3) });
for (const r of routes) console.log(r.method, r.h('/a/zz'));

const items = [{ g: { rate: 2 }, c: 4 }, { g: { rate: 9 }, c: 3 }, { g: { rate: 1 }, c: 5 }];
let best = null;
for (const { g, c } of items) if (!best || g.rate / c > best.g.rate / best.c) best = { g, c };
console.log(best.c, best.g.rate);
let none = null;
console.log(none === null, none ?? 'dflt');
var v = undefined;
v = 'now';
console.log(v);
const alias = tests;
alias.push(1);
console.log(tests.length);
