// DOMException's name/code/message are prototype getters, and instances have no own name/code/message.
const fn = f => (f ? `${f.name}/${f.length}` : '-');
for (const k of ['code', 'message', 'name']) {
  const d = Object.getOwnPropertyDescriptor(DOMException.prototype, k);
  console.log(k, fn(d.get), d.set, d.enumerable, d.configurable);
}
const ex = new DOMException('msg', 'AbortError');
console.log(ex.name, ex.code, ex.message, String(ex), ex instanceof Error, Object.prototype.toString.call(ex));
console.log(Object.getOwnPropertyNames(ex).filter(k => k !== 'stack').length, Object.keys(ex).length);
console.log(new DOMException().name, new DOMException().code, JSON.stringify(new DOMException().message));
console.log(new DOMException('x', { name: 'DataCloneError' }).code, new DOMException('x', 'NotFoundError').code);
console.log(new DOMException('x', 'Custom').code, new DOMException('x', { name: 'Y', cause: 3 }).cause);
try { ex.name = 'other'; console.log('no throw', ex.name); } catch (err) { console.log('set name', err instanceof TypeError); }
try { ex.code = 3; console.log('no throw', ex.code); } catch (err) { console.log('set code', err instanceof TypeError); }
try { DOMException.prototype.name; } catch (err) { console.log('proto name', err instanceof TypeError, err.code); }
try { Object.getOwnPropertyDescriptor(DOMException.prototype, 'code').get.call({}); } catch (err) { console.log('brand', err instanceof TypeError); }
console.log(String(new DOMException('gone', 'NotFoundError')));
