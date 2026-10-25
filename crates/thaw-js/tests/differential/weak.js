const wm = new WeakMap();
const k = {}, k2 = function () {};
wm.set(k, 1).set(k2, 'f');
console.log(wm.get(k), wm.get(k2), wm.has(k), wm.has({}), wm.delete(k), wm.has(k));
try { wm.set(1, 2); } catch (e) { console.log(e.name + ': ' + e.message); }
try { WeakMap(); } catch (e) { console.log(e.name + ': ' + e.message); }
const ws = new WeakSet([k, k2]);
console.log(ws.has(k), ws.has({}), ws.delete(k), ws.has(k), ws instanceof WeakSet);
try { ws.add('x'); } catch (e) { console.log(e.name + ': ' + e.message); }
const ref = new WeakRef(k2);
console.log(ref.deref() === k2, Object.prototype.toString.call(ref), Object.prototype.toString.call(wm));
const reg = new FinalizationRegistry(() => {});
console.log(reg.register(k, 'x'), reg.unregister(k));
class Cache extends WeakMap { size() { return 'n/a'; } }
const c = new Cache([[k, 5]]);
console.log(c.get(k), c.size(), c instanceof WeakMap, typeof WeakMap, WeakMap.name, new WeakMap([[k, 1]]).get(k));
