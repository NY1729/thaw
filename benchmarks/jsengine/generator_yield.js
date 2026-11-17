// 500k yields from a generator consumed by for-of; expected checksum: generator_yield 500000 124999750000
function* gen(n) {
  for (let i = 0; i < n; i++) yield i;
}
let sum = 0;
let count = 0;
for (const v of gen(500000)) {
  sum += v;
  count++;
}
console.log("generator_yield " + count + " " + sum);
