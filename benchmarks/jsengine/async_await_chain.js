// 200k sequential awaits of already-resolved promises in one async function; expected checksum: async_await_chain 19999900000
async function chain(n) {
  let acc = 0;
  for (let i = 0; i < n; i++) {
    acc += await Promise.resolve(i);
  }
  return acc;
}
chain(200000).then((acc) => console.log("async_await_chain " + acc));
