function* g(){ const x = yield 1; console.log('got', x); try { yield 2; } finally { console.log('cleanup'); } return 3; }
const it = g();
console.log(it.next('a'), it.next('b'), it.return(9), it.next());
function* inner(){ yield 'a'; yield 'b'; return 'r'; }
function* outer(){ const r = yield* inner(); yield r; yield* [1,2]; }
console.log([...outer()]);
const iterable = { *[Symbol.iterator](){ yield 1; yield 2; yield 3; } };
console.log(Array.from(iterable), Math.max(...iterable));
const [a, ...rest] = iterable; console.log(a, rest);
const custom = { [Symbol.iterator](){ let i=0; return { next(){ return i<3 ? {value:i++, done:false} : {value:undefined, done:true}; }, return(){ console.log('ret'); return {}; } }; } };
for (const v of custom) { if (v === 1) break; console.log(v); }
const [p] = custom; console.log(p);
class C { *items(){ yield* [10,20]; } static *s(){ yield 's'; } }
console.log([...new C().items()], [...C.s()]);
const o = { *gen(){ yield 1; } }; console.log([...o.gen()]);
function* thrower(){ try { yield 1; } catch(e){ console.log('caught', e); yield 'c'; } }
const t = thrower(); t.next(); console.log(t.throw('boom'), t.next());
console.log(typeof g, Object.prototype.toString.call(it), it[Symbol.iterator]() === it);
function* fib(){ let [a,b]=[0,1]; for(;;){ yield a; [a,b]=[b,a+b]; } }
const out=[]; for (const f of fib()) { if (f>50) break; out.push(f); } console.log(out.join());
console.log(new Map([[1,2]]).entries().next(), [...new Set([1,2]).values()]);
