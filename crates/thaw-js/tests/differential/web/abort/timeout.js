const s = AbortSignal.timeout(10);
console.log(s.aborted);
s.addEventListener('abort', () => console.log('timeout', s.reason.name, s.reason.code, s.reason.message));
setTimeout(() => console.log('after', s.aborted), 40);
try { AbortSignal.timeout(-1); } catch (e) { console.log(e instanceof RangeError); }
