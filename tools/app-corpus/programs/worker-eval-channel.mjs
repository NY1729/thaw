// worker_threads with an inline worker script, MessageChannel round trips and SharedArrayBuffer counters.
import { Worker } from 'node:worker_threads';

const code = `
const { parentPort, workerData } = require('node:worker_threads');
const counter = new Int32Array(workerData.sab);
parentPort.on('message', (m) => {
  if (m === 'stop') return parentPort.close();
  Atomics.add(counter, 0, m);
  parentPort.postMessage({ echo: m * 2 });
});
`;
const sab = new SharedArrayBuffer(4);
const w = new Worker(code, { eval: true, workerData: { sab } });
const replies = [];
w.on('message', (m) => replies.push(m));
for (const n of [1, 2, 3, 4]) w.postMessage(n);
await new Promise((r) => setTimeout(r, 100));
w.postMessage('stop');
const code2 = await new Promise((r) => w.on('exit', r));
console.log(replies, Atomics.load(new Int32Array(sab), 0), code2);
