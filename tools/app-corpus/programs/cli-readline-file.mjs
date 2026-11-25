// CLI tool: readline over a file written by the program, tally levels.
import { createInterface } from 'node:readline';
import { createReadStream } from 'node:fs';
import { writeFile } from 'node:fs/promises';

await writeFile('app.log', ['INFO start', 'WARN slow 120ms', 'INFO ok', 'ERROR boom', 'INFO done', 'WARN slow 340ms', ''].join('\n'));
const rl = createInterface({ input: createReadStream('app.log'), crlfDelay: Infinity });
const tally = {};
let n = 0;
for await (const line of rl) {
  n++;
  const level = line.split(' ', 1)[0];
  tally[level] = (tally[level] ?? 0) + 1;
}
console.log(n, tally);
const ms = (await import('node:fs')).readFileSync('app.log', 'utf8').match(/\d+(?=ms)/g).map(Number);
console.log('slow total', ms.reduce((a, b) => a + b));
