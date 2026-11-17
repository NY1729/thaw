// 50k for-await iterations over an async generator; expected checksum: async_generator 50000 1249975000
async function* agen(n) {
  for (let i = 0; i < n; i++) yield i;
}
(async () => {
  let sum = 0;
  let count = 0;
  for await (const v of agen(50000)) {
    sum += v;
    count++;
  }
  console.log("async_generator " + count + " " + sum);
})();
