const t = new EventTarget();
try { t.addEventListener('e', 42); console.log('no throw'); }
catch (err) { console.log(err instanceof TypeError); }
try { t.dispatchEvent({ type: 'e' }); console.log('no throw'); }
catch (err) { console.log(err instanceof TypeError); }
const ev = new Event('re');
t.addEventListener('re', () => { try { t.dispatchEvent(ev); } catch (err) { console.log('reentrant', err instanceof Error); } });
t.dispatchEvent(ev);
console.log(ev.type, new Event(7).type);
class Store extends EventTarget { fire() { return this.dispatchEvent(new Event('fire')); } }
const s = new Store();
s.addEventListener('fire', () => console.log('subclass fired'));
console.log(s.fire(), s instanceof EventTarget, Object.prototype.toString.call(new Event('q')));
