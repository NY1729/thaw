// tight numeric loop over 1e7 iterations inside a function, called twice; expected checksum: loop_sum_fn 49999995000000
function main() {
  let s = 0;
  for (let i = 0; i < 10000000; i++) {
    s += i;
  }
  return "loop_sum_fn " + s;
}
main();
console.log(main());
