// bitops_loop with the loop inside a function, called twice (the second call is the one that can compile); expected checksum: bitops_loop_fn -694039125 382245794
function main() {
  // xorshift-style int32 mixing, 3e6 iterations
  let x = 123456789, acc = 0;
  for (let i = 0; i < 3000000; i++) {
    x ^= x << 13; x ^= x >>> 17; x ^= x << 5;
    acc = (acc + (x & 0xff)) | 0;
  }
  return "bitops_loop_fn " + x + " " + acc;
}
main();
console.log(main());
