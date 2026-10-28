const e = new DOMException('boom', 'AbortError');
console.log(e.name, e.message, e.code, e instanceof Error, e instanceof DOMException);
console.log(Object.prototype.toString.call(e), String(e));
const d = new DOMException();
console.log(JSON.stringify([d.name, d.message, d.code]));
console.log(new DOMException('x', 'DataCloneError').code, new DOMException('x', 'Weird').code);
console.log(DOMException.ABORT_ERR, DOMException.prototype.TIMEOUT_ERR, DOMException.DATA_CLONE_ERR);
const c = new DOMException('m', { name: 'TimeoutError', cause: 5 });
console.log(c.name, c.code, c.cause, typeof e.stack);
