// Build script: gzip assets with zlib (sync, promisified and streaming) and verify round trip.
import { gzipSync, gunzipSync, gzip, gunzip, createGzip, createGunzip } from 'node:zlib';
import { promisify } from 'node:util';
import { createReadStream, createWriteStream } from 'node:fs';
import { writeFile, readFile, mkdir } from 'node:fs/promises';
import { pipeline } from 'node:stream/promises';

const text = 'the quick brown fox jumps over the lazy dog\n'.repeat(200);
await mkdir('out', { recursive: true });
await writeFile('out/big.txt', text);

const sync = gzipSync(text);
console.log('sync roundtrip', gunzipSync(sync).toString() === text, sync.length < text.length);

const gz = await promisify(gzip)(Buffer.from(text));
const back = await promisify(gunzip)(gz);
console.log('async roundtrip', back.equals(Buffer.from(text)));

await pipeline(createReadStream('out/big.txt'), createGzip(), createWriteStream('out/big.txt.gz'));
await pipeline(createReadStream('out/big.txt.gz'), createGunzip(), createWriteStream('out/big.copy.txt'));
console.log('stream roundtrip', (await readFile('out/big.copy.txt', 'utf8')) === text);
console.log('magic', (await readFile('out/big.txt.gz')).subarray(0, 2).toString('hex'));
