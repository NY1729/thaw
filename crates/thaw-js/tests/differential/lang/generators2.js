function* g() { try { yield 1; yield 2; yield 3; } finally { console.log('closed'); } }
for (const x of g()) { console.log(x); if (x === 2) break; }
const [first] = g(); console.log(first);
function* counter(n) { for (let i = 0; i < n; i++) { const cmd = yield i; if (cmd === 'stop') return 'stopped'; } return 'done'; }
const c = counter(5); console.log(c.next(), c.next('go'), c.next('stop'), c.next());
console.log(JSON.stringify([[...counter(3)], Array.from(counter(2)), Object.fromEntries(new Map([[1, 2]]))]));
function* delegating() { const r = yield* counter(2); console.log('inner returned', r); yield* 'ab'; yield* new Set([7, 8]); }
console.log(JSON.stringify([...delegating()]));
const gen = g(); gen.next(); console.log(gen.return('early'), gen.next());
function* thrower() { try { yield 1; } catch (e) { console.log('caught', e.message); return 'recovered'; } }
const t = thrower(); t.next(); console.log(JSON.stringify([t.throw(new Error('x')), t.next()]));
const lazy = (function* () { console.log('start'); yield 1; })(); console.log('created'); lazy.next();
class Tree { constructor(v, l, r) { this.v = v; this.l = l; this.r = r; } *[Symbol.iterator]() { if (this.l) yield* this.l; yield this.v; if (this.r) yield* this.r; } }
console.log(JSON.stringify([...new Tree(2, new Tree(1), new Tree(3))]));
const it = { from: 1, to: 3, [Symbol.iterator]() { let c = this.from, t = this.to; return { next: () => c <= t ? { value: c++, done: false } : { value: undefined, done: true } }; } };
console.log(JSON.stringify([...it]), Math.max(...it), new Set(it).size);
const entries = Object.entries({ a: 1, b: 2 }); for (const [k, v] of entries) console.log(k, v);
const m = new Map([['x', 1]]); for (const [k, v] of m) console.log(k, v);
console.log(typeof g().next, g()[Symbol.toStringTag], String(g()), g() instanceof g);
try { const gg = (function* () { gg.next(); yield 1; })(); gg.next(); } catch (e) { console.log(e.constructor.name, e.message); }
async function* ticks() { for (let i = 0; i < 3; i++) { await null; yield i; } }
(async () => { const out = []; for await (const t of ticks()) out.push(t); console.log(JSON.stringify(out)); const ag = ticks(); console.log(JSON.stringify([await ag.next(), await ag.return(9), await ag.next()])); })();
