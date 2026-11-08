// xorshift-style int32 mixing, 3e6 iterations
let x = 123456789, acc = 0;
for (let i = 0; i < 3000000; i++) {
  x ^= x << 13; x ^= x >>> 17; x ^= x << 5;
  acc = (acc + (x & 0xff)) | 0;
}
console.log("bitops_loop " + x + " " + acc);
