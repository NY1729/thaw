// CustomEvent.prototype.detail is a prototype getter; the value lives on the instance's state.
const d = Object.getOwnPropertyDescriptor(CustomEvent.prototype, 'detail');
console.log(typeof d.get, d.set, d.enumerable, d.configurable, d.get.name, d.get.length);
console.log(new CustomEvent('c').detail, new CustomEvent('c', { detail: undefined }).detail);
console.log(new CustomEvent('c', { detail: { a: 1 } }).detail.a, new CustomEvent('c', { detail: 0 }).detail);
const ce = new CustomEvent('c', { detail: 'x', bubbles: true });
console.log(Object.keys(ce).length, Object.getOwnPropertyNames(ce).length, ce.bubbles, ce.type);
try { ce.detail = 3; console.log('no throw'); } catch (err) { console.log('set detail', err instanceof TypeError); }
console.log(ce.detail);
try { d.get.call(new Event('x')); } catch (err) { console.log('brand', err instanceof TypeError, err.code); }
const t = new EventTarget();
t.addEventListener('c', ev => console.log('listener sees', ev.detail));
t.dispatchEvent(new CustomEvent('c', { detail: [1, 2] }));
