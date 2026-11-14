const DIV = new Uint8Array([0,97,115,109,1,0,0,0,1,7,1,96,2,127,127,1,127,3,2,1,0,7,7,1,3,100,105,118,0,0,10,9,1,7,0,32,0,32,1,109,11]);
const div = new WebAssembly.Instance(new WebAssembly.Module(DIV)).exports.div;
console.log(div(7, 2), div(-7, 2));
try { div(7, 0); } catch (e) { console.log(e instanceof WebAssembly.RuntimeError, e.name, e.message, e instanceof Error); }
try { div(-2147483648, -1); } catch (e) { console.log(e.name, e.message); }
const r = new WebAssembly.RuntimeError('custom');
console.log(r.name, r.message, String(r), Object.getPrototypeOf(WebAssembly.RuntimeError.prototype) === Error.prototype);
console.log(WebAssembly.LinkError.prototype.name, WebAssembly.CompileError.prototype.message === '', new WebAssembly.LinkError('m', { cause: 1 }).cause);
try { throw new WebAssembly.CompileError('bad'); } catch (e) { console.log(e.name, e.message, e instanceof WebAssembly.CompileError, e instanceof WebAssembly.LinkError); }
