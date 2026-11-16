// Event's state is prototype accessors (Node), not own data properties.
const fn = f => (f ? `${f.name}/${f.length}` : '-');
const show = (o, k) => {
  const d = Object.getOwnPropertyDescriptor(o, k);
  if (!d) return `${k}: absent`;
  const flags = `e${d.enumerable ? 1 : 0}c${d.configurable ? 1 : 0}w${d.writable ? 1 : 0}`;
  if (d.get || d.set) return `${k} accessor ${flags} get=${fn(d.get)} set=${fn(d.set)}`;
  return `${k} data ${flags} ${typeof d.value}=${fn(d.value)}`;
};
for (const k of ['type', 'target', 'currentTarget', 'srcElement', 'bubbles', 'cancelable', 'composed',
  'defaultPrevented', 'eventPhase', 'timeStamp', 'isTrusted', 'cancelBubble', 'returnValue']) {
  console.log(show(Event.prototype, k));
}
console.log(show(Event.prototype, 'composedPath'));
console.log(show(Event.prototype, 'initEvent'));
const e = new Event('a', { bubbles: true, cancelable: true, composed: true });
console.log(Object.keys(e).length, Object.getOwnPropertyNames(e).length, e.hasOwnProperty('type'));
console.log(e.type, e.bubbles, e.cancelable, e.composed, e.defaultPrevented, e.eventPhase);
console.log(e.target, e.currentTarget, e.srcElement, e.isTrusted, e.cancelBubble, e.returnValue);
console.log(e.composedPath().length, typeof e.timeStamp, e.timeStamp > 0);
try { e.type = 'x'; console.log('no throw'); } catch (err) { console.log('set type', err instanceof TypeError); }
console.log(e.type);
e.cancelBubble = true;
console.log('cancelBubble', e.cancelBubble);
e.returnValue = false;
console.log('returnValue', e.returnValue, e.defaultPrevented);
const plain = new Event('p');
plain.returnValue = false;
console.log('non-cancelable', plain.returnValue, plain.defaultPrevented);
try { Object.getOwnPropertyDescriptor(Event.prototype, 'type').get.call({}); }
catch (err) { console.log('brand', err instanceof TypeError, err.code); }
const t = new EventTarget();
t.addEventListener('p', ev => {
  console.log('in', ev.eventPhase, ev.target === t, ev.currentTarget === t, ev.srcElement === t, ev.composedPath().length, ev.composedPath()[0] === t);
});
const d = new Event('p');
t.dispatchEvent(d);
console.log('after', d.eventPhase, d.currentTarget, d.target === t, d.composedPath().length);
const s = new Event('s', { cancelable: true });
s.stopPropagation();
console.log('stopPropagation sets cancelBubble', s.cancelBubble);
