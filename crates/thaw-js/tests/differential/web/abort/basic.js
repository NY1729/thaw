const c = new AbortController();
const s = c.signal;
console.log(s.aborted, s.reason, Object.prototype.toString.call(s), Object.prototype.toString.call(c));
console.log(s instanceof EventTarget, s instanceof AbortSignal);
s.addEventListener('abort', (e) => console.log('listener', e.type, e.target === s));
s.onabort = () => console.log('onabort');
c.abort();
console.log(s.aborted, s.reason.name, s.reason.message, s.reason.code, s.reason instanceof DOMException);
c.abort(new Error('again'));
console.log(s.reason.name);
try { s.throwIfAborted(); } catch (e) { console.log('thrown', e.name); }
const c2 = new AbortController();
c2.abort('why');
console.log(c2.signal.reason);
try { new AbortSignal(); } catch (e) { console.log(e instanceof TypeError, e.message); }
const pre = AbortSignal.abort();
console.log(pre.aborted, pre.reason.name);
console.log(AbortSignal.abort(7).reason);
