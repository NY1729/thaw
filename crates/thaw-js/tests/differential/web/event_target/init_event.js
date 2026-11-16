// Event.prototype.initEvent: sets type/bubbles/cancelable, but not while dispatching.
const e = new Event('a', { bubbles: true, cancelable: true });
console.log(Event.prototype.initEvent.length, Event.prototype.initEvent.name);
e.initEvent('b', false, false);
console.log(e.type, e.bubbles, e.cancelable);
e.initEvent('c');
console.log(e.type, e.bubbles, e.cancelable);
try { e.initEvent(); } catch (err) { console.log('no args', err instanceof TypeError, err.code); }
const t = new EventTarget();
t.addEventListener('z', ev => {
  ev.initEvent('changed', true, true);
  console.log('during dispatch', ev.type, ev.bubbles, ev.cancelable);
});
const z = new Event('z', { cancelable: true });
t.dispatchEvent(z);
console.log('after dispatch', z.type, z.bubbles, z.cancelable);
const ce = new CustomEvent('ce', { detail: 5 });
ce.initEvent('cc', true, true);
console.log(ce.type, ce.detail, ce.bubbles, ce.cancelable);
