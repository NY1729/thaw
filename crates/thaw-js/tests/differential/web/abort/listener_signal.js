const c = new AbortController();
const t = new EventTarget();
let n = 0;
t.addEventListener('x', () => n++, { signal: c.signal });
t.dispatchEvent(new Event('x'));
c.abort();
t.dispatchEvent(new Event('x'));
console.log(n);
t.addEventListener('x', () => n += 100, { signal: AbortSignal.abort() });
t.dispatchEvent(new Event('x'));
console.log(n);
try { t.addEventListener('x', () => {}, { signal: {} }); } catch (e) { console.log(e instanceof TypeError); }
