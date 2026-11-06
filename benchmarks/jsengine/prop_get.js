// 1e6 plain data property reads (3 own props, no accessors) on one object; expected checksum: prop_get 999985
const o = { a: 1, b: 2, c: 3 };
let s = 0;
for (let i = 0; i < 1000000; i++) {
  s = (s + o.a + o.b + o.c) % 1000003;
}
console.log("prop_get " + s);
