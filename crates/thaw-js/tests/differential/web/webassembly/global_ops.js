const g = new WebAssembly.Global({ value: 'i32', mutable: true }, 7);
g.value = 9;
console.log(g.value, g.valueOf(), g.value + 1, Object.keys(g).length);
const c = new WebAssembly.Global({ value: 'i64' }, 5n);
console.log(c.value, typeof c.value);
try { c.value = 3n; } catch (e) { console.log(e.name, e.message); }
try { new WebAssembly.Global({ value: 'i64', mutable: true }, 5); } catch (e) { console.log(e.name, e.message); }
try { new WebAssembly.Global({ value: 'x' }); } catch (e) { console.log(e.name, e.message); }
try { new WebAssembly.Global(); } catch (e) { console.log(e.name, e.message); }
const f = new WebAssembly.Global({ value: 'f32', mutable: true }, 1.1);
console.log(f.value, new WebAssembly.Global({ value: 'i32' }).value);
const e = new WebAssembly.Global({ value: 'externref', mutable: true }, { k: 1 });
console.log(e.value.k, Object.prototype.toString.call(e), Object.prototype.toString.call(WebAssembly.Global.prototype));
const GLB = new Uint8Array([0,97,115,109,1,0,0,0, 6,6,1,127,1,65,42,11, 7,5,1,1,103,3,0]);
const eg = new WebAssembly.Instance(new WebAssembly.Module(GLB)).exports.g;
console.log(eg instanceof WebAssembly.Global, eg.value, eg.valueOf());
try { eg.value = 1; } catch (er) { console.log(er.name, er.message); }
