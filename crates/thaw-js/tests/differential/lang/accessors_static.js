class A { static #count = 0; static x = 5; static { A.y = A.x * 2; } #p = 1; get p(){ return this.#p; } set p(v){ this.#p = v*2; } static get count(){ return A.#count; } static inc(){ return ++A.#count; } #priv(){ return 'pm'; } callPriv(){ return this.#priv(); } static has(o){ return #p in o; } }
const a = new A(); a.p = 4; console.log(a.p, A.y, A.inc(), A.inc(), A.count, a.callPriv(), A.has(a), A.has({}));
const o = { _v: 1, get v(){ return this._v; }, set v(x){ this._v = x + 1; } }; o.v = 5; console.log(o.v, Object.getOwnPropertyDescriptor(o,'v').get !== undefined);
Object.defineProperty(o, 'k', { get(){ return 'k'; }, enumerable: false }); console.log(o.k, Object.keys(o));
class B extends A { static { B.z = super.x; } get p(){ return super.p + 100; } }
console.log(B.z, new B().p);
console.log(Object.getOwnPropertyNames(A.prototype));
label1: for (let i=0;i<3;i++){ for (let j=0;j<3;j++){ if (j===1) continue label1; if (i===2) break label1; console.log(i,j); } }
blk: { console.log('in'); if (true) break blk; console.log('never'); }
sw: switch (1) { case 1: for(;;){ break sw; } }
console.log('done');
