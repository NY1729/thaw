// subarray / set / copyWithin / slice over a 1 MiB Uint8Array, 30 rounds.
// expected checksum: typed_array_view_ops 9831270 6243840
const N = 1 << 20;
const buf = new Uint8Array(N);
for (let i = 0; i < N; i++) buf[i] = (i * 31) & 255;
const src = new Uint8Array(8192);
let acc = 0;
for (let r = 0; r < 30; r++) {
  const base = (r * 8192) % (N >> 1);
  src.fill(r & 255);
  buf.set(src, base);
  buf.copyWithin(base + 4096, base, base + 4096);
  const v = buf.subarray(base, base + 65536);
  acc = (acc + v[r & 1023] + v.length) | 0;
  const c = buf.slice(base, base + 262144);
  acc = (acc + c[4095] + c.length) | 0;
}
let sum = 0;
for (let i = 0; i < N; i += 16) sum += buf[i];
console.log("typed_array_view_ops " + acc + " " + sum);
