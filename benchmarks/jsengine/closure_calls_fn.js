// 1e6 calls to a closure that mutates captured state, loop inside a function, called twice; expected checksum: closure_calls_fn 1000000
function main() {
  function makeCounter() {
    let n = 0;
    return () => ++n;
  }
  const next = makeCounter();
  let last = 0;
  for (let i = 0; i < 1000000; i++) {
    last = next();
  }
  return "closure_calls_fn " + last;
}
main();
console.log(main());
