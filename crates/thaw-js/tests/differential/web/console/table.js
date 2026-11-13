// console.table renders Node's box table; cells are inspected.
const out = [];
const sink = { write(s) { out.push(s); return true; }, on() { return this; }, once() { return this; },
  removeListener() { return this; }, emit() { return false; }, listenerCount() { return 1; } };
const c = new console.Console(sink);
c.table([{ a: 1, b: 'x' }, { a: 2 }]);
c.table(5);
c.table('str');
c.table(null);
c.table([1, 'two', { k: true }]);
c.table({ r1: { c: 1 }, r2: { c: 22 } });
c.table([{ a: 1, b: 2, c: 3 }], ['b', 'zz']);
c.table([[1, 2], [3]]);
c.table([]);
c.table({});
c.table([{ name: 'long name here', v: [1, 2, 3] }]);
console.log(JSON.stringify(out));
