// Uint8Array / Int32Array / Float64Array indexed fill and sum loops, 2 passes over 300k elements each.
// expected checksum: typed_array_sum 76493056 465162888927744 45000150000
const N = 300000;
const u8 = new Uint8Array(N);
const i32 = new Int32Array(N);
const f64 = new Float64Array(N);
let u8sum = 0;
let i32sum = 0;
let f64sum = 0;
for (let pass = 0; pass < 2; pass++) {
  for (let i = 0; i < N; i++) u8[i] = (i + pass) & 255;
  for (let i = 0; i < N; i++) i32[i] = (i * 7919 - pass) | 0;
  for (let i = 0; i < N; i++) f64[i] = i * 0.5 + pass;
  for (let i = 0; i < N; i++) u8sum += u8[i];
  for (let i = 0; i < N; i++) i32sum += i32[i];
  for (let i = 0; i < N; i++) f64sum += f64[i];
}
console.log("typed_array_sum " + u8sum + " " + i32sum + " " + f64sum);
