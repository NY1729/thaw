// tight numeric loop over 1e7 iterations; expected checksum: loop_sum 49999995000000
let s = 0;
for (let i = 0; i < 10000000; i++) {
  s += i;
}
console.log("loop_sum " + s);
