const ADD = new Uint8Array([0,97,115,109,1,0,0,0,1,7,1,96,2,127,127,1,127,3,2,1,0,7,7,1,3,97,100,100,0,0,10,9,1,7,0,32,0,32,1,106,11]);
const IMP = new Uint8Array([0,97,115,109,1,0,0,0,1,6,1,96,1,127,1,127,2,9,1,3,101,110,118,1,103,0,0,3,2,1,0,7,7,1,3,114,117,110,0,1,10,11,1,9,0,32,0,16,0,65,1,106,11]);
const m = new WebAssembly.Module(ADD);
console.log(WebAssembly.Module.exports(m).map((e) => e.name + ':' + e.kind).join(','));
console.log(WebAssembly.Module.imports(new WebAssembly.Module(IMP)).map((e) => [e.module, e.name, e.kind].join('.')).join(','));
console.log(WebAssembly.Module.imports(m).length, WebAssembly.Module.customSections(m, 'nothing').length);
try { WebAssembly.Module.imports({}); } catch (e) { console.log(e.name, e.message); }
try { WebAssembly.Module.customSections(m); } catch (e) { console.log(e.name, e.message); }
console.log(Object.keys(WebAssembly.Module.exports(m)[0]).join(','));
try { new WebAssembly.Module(new ArrayBuffer(8)); } catch (e) { console.log(e.name, e.message); }
try { new WebAssembly.Module('abc'); } catch (e) { console.log(e.name, e.message); }
const bytes = new Uint8Array(ADD.length + 12);
bytes.set(ADD, 0);
const WITH = new Uint8Array([...ADD, 0, 5, 2, 110, 97, 9, 9]);
console.log(WebAssembly.Module.customSections(new WebAssembly.Module(WITH), 'na').length, WebAssembly.Module.customSections(new WebAssembly.Module(WITH), 'na')[0].byteLength);
console.log(WebAssembly.validate(WITH), WebAssembly.validate(5 === 5 ? ADD : null));
try { WebAssembly.validate(5); } catch (e) { console.log(e.name, e.message); }
