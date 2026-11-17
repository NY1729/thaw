// DataView getInt32/setInt32 (little-endian) and getFloat64/setFloat64 (big-endian) loops, 2 passes over 32k slots.
// expected checksum: dataview_rw -6389497856 268460032
const N = 32768;
const dv = new DataView(new ArrayBuffer(N * 8));
let i32sum = 0;
let f64sum = 0;
for (let pass = 0; pass < 2; pass++) {
  for (let i = 0; i < N; i++) dv.setInt32(i * 8, (i * 2654435761 + pass) | 0, true);
  for (let i = 0; i < N; i++) i32sum += dv.getInt32(i * 8, true);
  for (let i = 0; i < N; i++) dv.setFloat64(i * 8, i * 0.25 + pass, false);
  for (let i = 0; i < N; i++) f64sum += dv.getFloat64(i * 8, false);
}
console.log("dataview_rw " + i32sum + " " + f64sum);
