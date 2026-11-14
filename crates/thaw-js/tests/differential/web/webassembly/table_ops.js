const ADD = new Uint8Array([0,97,115,109,1,0,0,0,1,7,1,96,2,127,127,1,127,3,2,1,0,7,7,1,3,97,100,100,0,0,10,9,1,7,0,32,0,32,1,106,11]);
const add = new WebAssembly.Instance(new WebAssembly.Module(ADD)).exports.add;
const t = new WebAssembly.Table({ element: 'anyfunc', initial: 2 });
console.log(t.length, t.get(0), t.get(1), t.grow(1), t.length);
t.set(0, add);
console.log(t.get(0) === add, t.get(0)(20, 22), t.get(1));
try { t.set(0, () => 1); } catch (e) { console.log(e.name); }
try { t.get(9); } catch (e) { console.log(e.name, e.message); }
try { t.set(9, null); } catch (e) { console.log(e.name, e.message); }
try { t.grow(10); } catch (e) { console.log(e.name, e.message); }
try { new WebAssembly.Table({ element: 'i32', initial: 1 }); } catch (e) { console.log(e.name, e.message); }
try { new WebAssembly.Table({ element: 'anyfunc' }); } catch (e) { console.log(e.name, e.message); }
const x = new WebAssembly.Table({ element: 'externref', initial: 1 }, 'hello');
console.log(x.get(0), x.set(0, 42), x.get(0), x.grow(1, 'z'), x.get(1), x.length);
const ft = new WebAssembly.Table({ element: 'anyfunc', initial: 1, maximum: 1 });
try { ft.grow(1); } catch (e) { console.log(e.name, e.message); }
console.log(Object.prototype.toString.call(t), Object.getOwnPropertyNames(WebAssembly.Table.prototype).join(','));
const TBL = new Uint8Array([0,97,115,109,1,0,0,0, 4,4,1,112,0,2, 7,7,1,3,116,98,108,1,0]);
const tbl = new WebAssembly.Instance(new WebAssembly.Module(TBL)).exports.tbl;
console.log(tbl.length, tbl.get(0), tbl instanceof WebAssembly.Table);
