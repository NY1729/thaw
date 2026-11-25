// Async generators, Readable.from, batching and backpressure-free fan-in.
import { Readable } from 'node:stream';
import { setTimeout as sleep } from 'node:timers/promises';

async function* numbers(n) {
  for (let i = 1; i <= n; i++) {
    if (i % 4 === 0) await sleep(1);
    yield i;
  }
}
async function* map(src, fn) {
  for await (const x of src) yield await fn(x);
}
async function* batch(src, size) {
  let buf = [];
  for await (const x of src) {
    buf.push(x);
    if (buf.length === size) {
      yield buf;
      buf = [];
    }
  }
  if (buf.length) yield buf;
}

const squares = map(numbers(10), async (x) => x * x);
for await (const b of batch(squares, 3)) console.log(b);

const r = Readable.from(numbers(5));
const got = [];
for await (const x of r) got.push(x);
console.log(got);

const it = numbers(3)[Symbol.asyncIterator]();
console.log(await it.next(), await it.next(), await it.return(), await it.next());
console.log(await Array.fromAsync(map(numbers(4), (x) => x + 0.5)));
