// A Buffer nested in an `any` array or object prints as `<Buffer ..>` (up to 50 bytes, then `... N more bytes`).
const a = [Buffer.from('a')];
console.log(a);
const o = { b: Buffer.from('hi'), n: 1, nested: { c: Buffer.alloc(3) } };
console.log(o);
console.log([Buffer.alloc(52, 1)]);
console.log(Buffer.from('xyz'), [Buffer.from('')]);
