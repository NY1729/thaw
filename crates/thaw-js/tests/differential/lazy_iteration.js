const it = { [Symbol.iterator]() { let i = 0; return { next() { return { value: i++, done: false }; }, return() { console.log('closed'); return {}; } }; } };
for (const x of it) { if (x > 2) break; console.log(x); }
const [a, b] = it; console.log(a, b);
const [c, ...none] = { [Symbol.iterator]() { let n = 0; return { next() { return n < 3 ? { value: n++, done: false } : { value: undefined, done: true }; } }; } };
console.log(c, none);
try { for (const x of it) { throw new Error('inside'); } } catch (e) { console.log(e.message); }
function f() { for (const x of it) { return 'returned'; } }
console.log(f());
outer: for (const i of [1, 2]) { for (const x of it) { continue outer; } }
