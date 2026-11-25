// timers/promises scheduling: delays, intervals as async iterators, AbortSignal cancellation.
import { setTimeout as sleep, setInterval as every, setImmediate as immediate } from 'node:timers/promises';

const log = [];
const t0 = Date.now();
const tasks = [
  sleep(30, 'slow').then((v) => log.push(v)),
  sleep(5, 'fast').then((v) => log.push(v)),
  sleep(15, 'mid').then((v) => log.push(v)),
];
await Promise.all(tasks);
console.log(log, Date.now() - t0 >= 25);

let ticks = 0;
for await (const v of every(5, 'tick')) {
  if (++ticks === 4) break;
}
console.log('ticks', ticks);

const ac = new AbortController();
const p = sleep(10_000, 'never', { signal: ac.signal });
setTimeout(() => ac.abort(), 5);
try {
  await p;
} catch (e) {
  console.log(e.name, e.code);
}

const order = [];
setImmediate(() => order.push('immediate'));
process.nextTick(() => order.push('tick'));
Promise.resolve().then(() => order.push('micro'));
await sleep(20);
console.log(order, await immediate('imm'));

const timeout = setTimeout(() => console.log('should not run'), 50);
clearTimeout(timeout);
const iv = setInterval(() => {}, 1000);
iv.unref();
console.log('done');
