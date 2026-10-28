const log = [];
const p = new Proxy({a:1}, { get(t,k,r){ log.push('get '+String(k)); return Reflect.get(t,k,r); }, set(t,k,v,r){ log.push('set '+k); return Reflect.set(t,k,v,r); }, has(t,k){ log.push('has '+k); return k in t; }, deleteProperty(t,k){ log.push('del '+k); return delete t[k]; }, ownKeys(t){ log.push('keys'); return Reflect.ownKeys(t); } });
p.b = 2; p.a; 'a' in p; delete p.a; Object.keys(p);
console.log(log.join('|'), p.b);
const fp = new Proxy(function(x){ return x*2; }, { apply(t, th, args){ return t(...args) + 1; }, construct(t, args){ return {made: args}; } });
console.log(fp(2), new fp(1,2));
console.log(Reflect.ownKeys({a:1,[Symbol.iterator]:2}).length, Reflect.has({x:1},'x'), Reflect.getPrototypeOf([]) === Array.prototype);
console.log(Reflect.apply(Math.max, null, [1,3,2]), Reflect.construct(Date, [0]).getTime(), Reflect.defineProperty({}, 'x', {value:1}));
const o = {}; Reflect.set(o, 'q', 1); console.log(o, Reflect.deleteProperty(o,'q'), o);
const dflt = new Proxy({}, { get: (t,k) => k in t ? t[k] : 'dflt:'+String(k) }); console.log(dflt.foo);
const { proxy, revoke } = Proxy.revocable({}, {}); revoke(); try { proxy.x; } catch (e) { console.log(e instanceof TypeError); }
