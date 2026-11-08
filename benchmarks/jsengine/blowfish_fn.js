// Blowfish-shaped kernel (bcryptjs _encipher: 16 unrolled Feistel rounds over four S-boxes), run with
// Int32Array tables and with plain-array tables; both variants must print the same checksum.
// Each run is a function called many times, so the encipher calls are the compilable hot path.
// expected checksum: blowfish_fn 1459210893 1459210893
function fill(a, seed) {
  let x = seed;
  for (let i = 0; i < a.length; i++) {
    x ^= x << 13; x ^= x >>> 17; x ^= x << 5;
    a[i] = x | 0;
  }
  return a;
}

function encipher(lr, P, S) {
  let l = lr[0], r = lr[1];
  for (let i = 0; i < 16; i += 2) {
    let n = S[l >>> 24];
    n = (n + S[0x100 | ((l >> 16) & 0xff)]) | 0;
    n ^= S[0x200 | ((l >> 8) & 0xff)];
    n = (n + S[0x300 | (l & 0xff)]) | 0;
    r ^= n ^ P[i];
    n = S[r >>> 24];
    n = (n + S[0x100 | ((r >> 16) & 0xff)]) | 0;
    n ^= S[0x200 | ((r >> 8) & 0xff)];
    n = (n + S[0x300 | (r & 0xff)]) | 0;
    l ^= n ^ P[i + 1];
  }
  lr[0] = r ^ P[17];
  lr[1] = l ^ P[16];
}

function run(P, S, lr, rounds) {
  let acc = 0;
  for (let n = 0; n < rounds; n++) {
    lr[0] = n;
    lr[1] = acc;
    encipher(lr, P, S);
    acc = (acc ^ lr[0] ^ lr[1]) | 0;
  }
  return acc;
}

function typedRun(rounds) {
  return run(fill(new Int32Array(18), 0x2545f491), fill(new Int32Array(1024), 0x9e3779b9), new Int32Array(2), rounds);
}

function plainRun(rounds) {
  return run(fill(new Array(18).fill(0), 0x2545f491), fill(new Array(1024).fill(0), 0x9e3779b9), [0, 0], rounds);
}

console.log("blowfish_fn " + typedRun(20000) + " " + plainRun(20000));
