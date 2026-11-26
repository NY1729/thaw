// Unresolved types in JS are `any`: recursive returns, async generators, Promise callbacks/resolve values.

function isPrime(n) {
  if (n < 2) return false;
  for (let i = 2; i * i <= n; i++) if (n % i === 0) return false;
  return true;
}
function fact(n) {
  return n <= 1 ? 1 : n * fact(n - 1);
}
function pick(x) {
  if (x) return pick(0);
  return x;
}

async function* gen(n) {
  for (let i = 0; i < n; i++) yield i * 2;
}

function later(v) {
  return new Promise((r) => setTimeout(() => r(v), 1));
}

console.log([1, 2, 3, 4, 5, 7, 11].filter((n) => isPrime(n)).join(','));
console.log(fact(5), pick(3));
const seen = [];
for await (const v of gen(4)) seen.push(v);
console.log(JSON.stringify(seen));
await later({ a: 1, b: [2] }).then(({ a, b }) => console.log(a, b));
