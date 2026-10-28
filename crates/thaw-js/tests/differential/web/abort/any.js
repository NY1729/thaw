const a = new AbortController(), b = new AbortController();
const any = AbortSignal.any([a.signal, b.signal]);
any.addEventListener('abort', () => console.log('any fired', any.reason));
console.log(any.aborted);
b.abort('B');
console.log(any.aborted, any.reason, a.signal.aborted);
a.abort('A');
console.log(any.reason);
const done = AbortSignal.abort('done');
console.log(AbortSignal.any([new AbortController().signal, done]).reason);
try { AbortSignal.any([{}]); } catch (e) { console.log(e instanceof TypeError); }
