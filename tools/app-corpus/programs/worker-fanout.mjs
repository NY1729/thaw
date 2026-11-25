// worker_threads fan-out: the same file is main and worker, results gathered over messages.
import { Worker, isMainThread, parentPort, workerData } from 'node:worker_threads';

function isPrime(n) {
  if (n < 2) return false;
  for (let i = 2; i * i <= n; i++) if (n % i === 0) return false;
  return true;
}

if (isMainThread) {
  const ranges = [[0, 2500], [2500, 5000], [5000, 7500], [7500, 10000]];
  const results = await Promise.all(
    ranges.map(
      ([lo, hi], idx) =>
        new Promise((resolve, reject) => {
          const w = new Worker(new URL(import.meta.url), { workerData: { lo, hi, idx } });
          w.once('message', resolve);
          w.once('error', reject);
        }),
    ),
  );
  results.sort((a, b) => a.idx - b.idx);
  for (const r of results) console.log(r);
  console.log('total primes', results.reduce((s, r) => s + r.count, 0));
} else {
  const { lo, hi, idx } = workerData;
  let count = 0;
  for (let n = lo; n < hi; n++) if (isPrime(n)) count++;
  parentPort.postMessage({ idx, lo, hi, count });
}
