// read-modify-write loop over 3e6 iterations inside a function, called twice; expected checksum: rmw_loop_fn 4499998500000 3000000
function main() {
  let s = 0;
  let t = 0;
  for (let i = 0; i < 3000000; i++) {
    s += i;
    t = t + 2;
    t -= 1;
  }
  return "rmw_loop_fn " + s + " " + t;
}
main();
console.log(main());
