// A record literal handed to a closure / array-held middleware keeps reference semantics.
const mw = [
  (ctx, next) => { ctx.trace.push('a'); const r = next(); ctx.trace.push('a-after'); return r; },
  (ctx, next) => { ctx.trace.push('b'); ctx.n = 5; return next(); },
];
const ctx = { trace: [], n: 0 };
const run = (i) => (i < mw.length ? mw[i](ctx, () => run(i + 1)) : 'done');
console.log(run(0), ctx.trace.join('>'), ctx.n);
const bump = (c) => { c.count += 1; };
const counter = { count: 0 };
bump(counter); bump(counter);
console.log(counter.count);
// An array field in a record given to an untyped (polymorphic) function declaration.
function len(o) { return o.b.length; }
console.log(len({ b: [1, 2] }), len({ a: 'q', b: [1, 2, 3] }), len({ b: 'xyz' }));
// A RegExp inside an untyped record.
const routes = [];
const route = (m, p) => routes.push({ m, re: new RegExp('^' + p.replace(/:(\w+)/g, '(?<$1>[^/]+)') + '$') });
route('GET', '/u/:id');
const m = routes[0].re.exec('/u/42');
console.log(routes[0].re.test('/u/1'), m && m.groups.id);
