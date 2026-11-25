// Operators on untyped (any) parameters must match Node.
const add = (a, b) => a + b;
const cmp = (a, b) => [a < b, a > b, a <= b, a >= b, a == b, a === b];
const truthy = (x) => (x ? 'T' : 'F');
const prop = (o) => o.a + o.b.length;
const callIt = (f, x) => f(x);
const sum = (xs) => { let s = 0; for (const x of xs) s += x; return s; };
const spread = (xs, ys) => [...xs, ...ys];
const destr = ({ a, b }, [c, d]) => a + b + c + d;
const tpl = (n, s) => `n=${n} s=${s}`;
function method(o, k) { return o[k]; }
const inc = (x) => x + 1;
console.log(add(1, 2), add('a', 'b'), add('a', 1), add(1, '2'), add([1], [2]), add(null, 1));
console.log(cmp(1, 2), cmp('a', 'b'), cmp('2', 10), cmp(null, 0));
console.log(truthy(0), truthy(''), truthy('x'), truthy(null), truthy({}), truthy(NaN));
console.log(prop({ a: 1, b: 'xyz' }), prop({ a: 'q', b: [1, 2] }));
console.log(callIt((n) => n * 2, 21), callIt(String, 5));
console.log(sum([1, 2, 3]), sum(new Set([4, 5])));
console.log(spread([1], [2, 3]));
console.log(destr({ a: 1, b: 2 }, [3, 4]));
console.log(tpl(1, 'x'), tpl(null, undefined), tpl({ x: 1 }, [1, 2]));
console.log(method({ k: 5 }, 'k'), method([7, 8], 1), method('abc', 'length'));
console.log(inc(1), inc(1.5), inc('1'));
class Box { constructor(v) { this.v = v; } get() { return this.v; } set(v) { this.v = v; return this; } }
const b = new Box(1);
console.log(b.get(), b.set('s').get(), new Box([1, 2]).get().length);
class Counter { #n; constructor() { this.#n = 0; } inc() { this.#n++; return this.#n; } }
const c = new Counter(); c.inc(); console.log(c.inc());
