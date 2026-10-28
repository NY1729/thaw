const wm = new WeakMap(), k = {}; wm.set(k, 1); console.log(wm.get(k), wm.has(k), wm.has({}), wm.delete(k), wm.has(k));
const ws = new WeakSet([k]); console.log(ws.has(k), ws.delete(k), ws.has(k));
const wr = new WeakRef(k); console.log(wr.deref() === k);
try { wm.set(1, 1); } catch (e) { console.log(e.constructor.name); }
console.log(Symbol.iterator.toString(), Symbol('x').description, Symbol.for('a') === Symbol.for('a'), Symbol.keyFor(Symbol.for('a')));
class T { get [Symbol.toStringTag](){ return 'TT'; } static [Symbol.hasInstance](x){ return x === 1; } [Symbol.toPrimitive](h){ return h === 'number' ? 42 : 'str'; } }
console.log(String(new T()), Object.prototype.toString.call(new T()), 1 instanceof T, +new T(), `${new T()}`, new T() + '');
const sp = { [Symbol.isConcatSpreadable]: true, length: 2, 0: 'a', 1: 'b' }; console.log([1].concat(sp));
const s = Symbol('d'); const o = { [s]: 1, x: 2 }; console.log(Object.keys(o), Object.getOwnPropertySymbols(o).length, s in o, JSON.stringify(o));
console.log(typeof Symbol(), Symbol.asyncIterator.toString(), Symbol.species.toString());
class MyArr extends Array { static get [Symbol.species]() { return Array; } } console.log(new MyArr(1,2,3).map(x=>x) instanceof MyArr);
