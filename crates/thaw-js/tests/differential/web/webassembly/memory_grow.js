const mem = new WebAssembly.Memory({ initial: 1, maximum: 3 });
const first = mem.buffer;
console.log(first.byteLength, mem.buffer === first, first instanceof ArrayBuffer);
new Uint8Array(first)[5] = 9;
console.log(mem.grow(1), first.byteLength, first.detached, mem.buffer.byteLength, mem.buffer === first);
console.log(new Uint8Array(mem.buffer)[5]);
try { mem.grow(5); } catch (e) { console.log(e.name, e.message); }
try { new WebAssembly.Memory({}); } catch (e) { console.log(e.name, e.message); }
try { new WebAssembly.Memory({ initial: 2, maximum: 1 }); } catch (e) { console.log(e.name, e.message); }
try { WebAssembly.Memory({ initial: 1 }); } catch (e) { console.log(e.name, e.message); }
try { WebAssembly.Memory.prototype.grow.call({}, 1); } catch (e) { console.log(e.name, e.message); }
const ZERO = new WebAssembly.Memory({ initial: 0 });
console.log(ZERO.buffer.byteLength, ZERO.grow(0), Object.prototype.toString.call(ZERO));
const MEMMOD = new Uint8Array([0,97,115,109,1,0,0,0, 5,4,1,1,1,4, 7,10,1,6,109,101,109,111,114,121,2,0]);
const exp = new WebAssembly.Instance(new WebAssembly.Module(MEMMOD)).exports.memory;
console.log(exp instanceof WebAssembly.Memory, exp.buffer.byteLength, exp.grow(2), exp.buffer.byteLength);
