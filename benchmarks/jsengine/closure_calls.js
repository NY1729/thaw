// 1e6 calls to a closure that mutates captured state; expected checksum: closure_calls 1000000
function makeCounter() {
  let n = 0;
  return () => ++n;
}
const next = makeCounter();
let last = 0;
for (let i = 0; i < 1000000; i++) {
  last = next();
}
console.log("closure_calls " + last);
