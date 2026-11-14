const IMP = new Uint8Array([0,97,115,109,1,0,0,0,1,6,1,96,1,127,1,127,2,9,1,3,101,110,118,1,103,0,0,3,2,1,0,7,7,1,3,114,117,110,0,1,10,11,1,9,0,32,0,16,0,65,1,106,11]);
const run = new WebAssembly.Instance(new WebAssembly.Module(IMP), { env: { g: (x) => x * 10 } }).exports.run;
console.log(run(4), run('2'), run());
const err = new TypeError('boom');
const thrower = new WebAssembly.Instance(new WebAssembly.Module(IMP), { env: { g: () => { throw err; } } }).exports.run;
try { thrower(1); } catch (e) { console.log(e === err, e.name, e.message); }
const undef = new WebAssembly.Instance(new WebAssembly.Module(IMP), { env: { g: () => {} } }).exports.run;
console.log(undef(1));
