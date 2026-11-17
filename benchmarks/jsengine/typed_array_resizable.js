// Length-tracking Uint32Array over a resizable ArrayBuffer (max 256 KiB), resized between 64 passes.
// expected checksum: typed_array_resizable 50272075776 65536 262144
const rab = new ArrayBuffer(16384, { maxByteLength: 262144 });
const view = new Uint32Array(rab);
let sum = 0;
for (let pass = 0; pass < 64; pass++) {
  rab.resize(16384 * (1 + (pass % 16)));
  for (let i = 0; i < view.length; i++) view[i] = i + pass;
  for (let i = 0; i < view.length; i++) sum += view[i];
}
console.log("typed_array_resizable " + sum + " " + view.length + " " + rab.byteLength);
