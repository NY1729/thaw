function F() { return new.target === F ? 'new' : new.target === undefined ? 'call' : 'other'; }
console.log(F(), new F() instanceof F, Reflect.construct(F, []) instanceof F);
class Base { constructor() { this.nt = new.target.name; } }
class Derived extends Base { constructor() { super(); this.d = 1; } }
class Derived2 extends Base { }
console.log(new Base().nt, new Derived().nt, new Derived2().nt);
class Abstract { constructor() { if (new.target === Abstract) throw new TypeError('abstract'); } }
try { new Abstract(); } catch (e) { console.log(e.message); }
console.log(new (class extends Abstract { })() instanceof Abstract);
const arrow = function () { return (() => new.target === undefined)(); };
console.log(arrow(), new arrow() instanceof arrow);
class P {
  #a = 1; static #s = 'st'; #m() { return this.#a * 10; } get #g() { return this.#a + 1; } set #g(v) { this.#a = v; }
  static #sm() { return P.#s; }
  run() { this.#g = 5; return [this.#m(), this.#g, P.#sm(), #a in this, this.#a++, this.#a, ++this.#a]; }
  static is(o) { return #a in o; }
  static mk() { return new P(); }
}
console.log(JSON.stringify(new P().run()), P.is(new P()), P.is({}), Object.keys(new P()), JSON.stringify(new P()), Object.getOwnPropertyNames(new P()));
class Counter { static #n = 0; static inc() { return ++Counter.#n; } #items = []; add(x) { this.#items.push(x); return this; } get size() { return this.#items.length; } }
console.log(Counter.inc(), Counter.inc(), new Counter().add(1).add(2).size);
class Sub extends P { constructor() { super(); this.z = 2; } }
console.log(new Sub().run().length, Sub.is(new Sub()));
