console.log(Object.getOwnPropertyNames(WebAssembly).join(','));
console.log(Object.keys(WebAssembly).join(','));
console.log(typeof WebAssembly, Object.prototype.toString.call(WebAssembly), WebAssembly[Symbol.toStringTag]);
for (const n of ['Module', 'Instance', 'Memory', 'Table', 'Global', 'Tag', 'Exception']) {
  const C = WebAssembly[n];
  console.log(n, typeof C, C.length, C.name, Object.getOwnPropertyNames(C.prototype).join(','));
  console.log(n, Object.prototype.toString.call(C.prototype), Object.getOwnPropertyDescriptor(C.prototype, 'constructor').enumerable);
}
for (const n of ['CompileError', 'LinkError', 'RuntimeError']) {
  const C = WebAssembly[n];
  const e = new C('boom');
  console.log(n, C.length, C.name, e.name, e.message, e instanceof Error, e instanceof C, String(e), Object.getPrototypeOf(C) === Error);
  console.log(n, Object.prototype.toString.call(e), C('x') instanceof C, Object.getOwnPropertyDescriptor(C.prototype, 'name').enumerable);
}
console.log(WebAssembly.Module.imports.length, WebAssembly.Module.exports.name, WebAssembly.validate.length, WebAssembly.instantiate.length, WebAssembly.compile.length);
console.log(Object.getOwnPropertyDescriptor(WebAssembly, 'Module').enumerable, Object.getOwnPropertyDescriptor(WebAssembly, 'JSTag').writable);
console.log(WebAssembly.JSTag instanceof WebAssembly.Tag, Object.prototype.toString.call(WebAssembly.JSTag));
try { WebAssembly.Module(new Uint8Array(8)); } catch (e) { console.log(e.name, e.message); }
try { new WebAssembly.Module(new Uint8Array([0,97,115,109,2,0,0,0])); } catch (e) { console.log(e.name, e.message); }
try { new WebAssembly.Module(new Uint8Array([1,2])); } catch (e) { console.log(e.name, e.message); }
try { new WebAssembly.Module(); } catch (e) { console.log(e.name, e.message); }
