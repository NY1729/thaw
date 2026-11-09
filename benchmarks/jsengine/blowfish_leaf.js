// Blowfish-shaped leaf kernel: the 16 Feistel rounds of bcryptjs _encipher run inside one leaf function's loops,
// so the whole hot loop compiles (no call inside it). Int32Array and plain-array tables must print the same checksum.
// expected checksum: blowfish_leaf 1459210893 1459210893 (the same value as blowfish_fn; checked in compile/num_tests.rs)
function fill(a, seed) {
  let x = seed;
  for (let i = 0; i < a.length; i++) {
    x ^= x << 13; x ^= x >>> 17; x ^= x << 5;
    a[i] = x | 0;
  }
  return a;
}

function runLeaf(P, S, rounds) {
  let acc = 0;
  for (let n = 0; n < rounds; n++) {
    let l = n, r = acc;
    for (let i = 0; i < 16; i += 2) {
      let t = S[l >>> 24];
      t = (t + S[0x100 | ((l >> 16) & 0xff)]) | 0;
      t ^= S[0x200 | ((l >> 8) & 0xff)];
      t = (t + S[0x300 | (l & 0xff)]) | 0;
      r ^= t ^ P[i];
      t = S[r >>> 24];
      t = (t + S[0x100 | ((r >> 16) & 0xff)]) | 0;
      t ^= S[0x200 | ((r >> 8) & 0xff)];
      t = (t + S[0x300 | (r & 0xff)]) | 0;
      l ^= t ^ P[i + 1];
    }
    acc = (acc ^ (r ^ P[17]) ^ (l ^ P[16])) | 0;
  }
  return acc;
}

const typedP = fill(new Int32Array(18), 0x2545f491);
const typedS = fill(new Int32Array(1024), 0x9e3779b9);
const plainP = fill(new Array(18).fill(0), 0x2545f491);
const plainS = fill(new Array(1024).fill(0), 0x9e3779b9);

console.log("blowfish_leaf " + runLeaf(typedP, typedS, 20000) + " " + runLeaf(plainP, plainS, 20000));
