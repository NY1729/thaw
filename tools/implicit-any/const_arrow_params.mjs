// Top-level const arrows take parameter types from call sites like function declarations,
// so a record passed to one is shared (not copied) and an index parameter is a number.
const bump = (c) => { c.n = 5; c.t.push(1); };
const ctx = { n: 0, t: [] };
bump(ctx);
console.log(ctx.n, ctx.t.length);
const items = [10, 20, 30];
const at = (i) => items[i];
const fact = (n) => (n < 2 ? 1 : n * fact(n - 1));
console.log(at(1), at(2), fact(5));
