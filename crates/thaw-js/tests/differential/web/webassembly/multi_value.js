const SWAP = new Uint8Array([0,97,115,109,1,0,0,0, 1,8,1,96,2,127,127,2,127,127, 3,2,1,0, 7,8,1,4,115,119,97,112,0,0, 10,8,1,6,0,32,1,32,0,11]);
const swap = new WebAssembly.Instance(new WebAssembly.Module(SWAP)).exports.swap;
const r = swap(1, 2);
console.log(Array.isArray(r), r.join(','), swap.length);
