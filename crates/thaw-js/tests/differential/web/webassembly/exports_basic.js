const ADD = new Uint8Array([0,97,115,109,1,0,0,0,1,7,1,96,2,127,127,1,127,3,2,1,0,7,7,1,3,97,100,100,0,0,10,9,1,7,0,32,0,32,1,106,11]);
console.log(WebAssembly.validate(ADD), WebAssembly.validate(new Uint8Array([0,97,115,109,2,0,0,0])));
const inst = new WebAssembly.Instance(new WebAssembly.Module(ADD));
const add = inst.exports.add;
console.log(add(2, 3), add('7', 1), add(2147483648, 1), add(1.9, 2.9), add());
console.log(add.name, add.length, typeof add, String(add));
console.log(Object.getOwnPropertyNames(inst).length, Object.isFrozen(inst.exports), Object.getPrototypeOf(inst.exports) === null);
console.log(inst.exports.add === add, Object.keys(inst.exports).join(','));
console.log(Object.prototype.toString.call(inst), Object.prototype.toString.call(WebAssembly.Module), Object.prototype.toString.call(WebAssembly));
console.log(Object.getOwnPropertyDescriptor(inst.exports, 'add').writable, Object.getOwnPropertyDescriptor(inst.exports, 'add').enumerable);
try { inst.exports.add = 1; } catch (e) { console.log(e.name); }
console.log(inst.exports.add, 'set ok');
try { WebAssembly.Instance.prototype.exports; } catch (e) { console.log(e.name, e.message); }
