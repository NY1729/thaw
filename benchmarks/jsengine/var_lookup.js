// variable lookup: (1) blowfish-like kernel with 8 locals, 2e6 rounds; (2) 6-deep closure nest, 1e6 inner iterations reading outer variables at hops 1..6; (3) 20-local function, 2e5 iterations (past the 16-variable hash threshold); expected checksum: var_lookup <combined>
function blowfishKernel(rounds) {
  let l = 0x2545f491, r = 0, n = 0, i = 0, j = 0, P = [], S = [], k = 0;
  for (i = 0; i < 18; i++) {
    l ^= l << 13;
    l ^= l >>> 17;
    l ^= l << 5;
    P.push(l);
  }
  for (i = 0; i < 256; i++) {
    l ^= l << 13;
    l ^= l >>> 17;
    l ^= l << 5;
    S.push(l);
  }
  l = 0;
  for (n = 0; n < rounds; n++) {
    k = n % 18;
    l ^= P[k];
    i = l >>> 24;
    j = (l >>> 16) & 255;
    k = (((S[i] + S[j]) | 0) ^ S[(l >>> 8) & 255]) + S[l & 255] | 0;
    r ^= k;
    k = l;
    l = r;
    r = k;
    i = n & 255;
    S[i] ^= l;
    S[i] += r | 0;
  }
  k = 0;
  for (i = 0; i < 256; i++) {
    k = ((k << 5) - k + S[i]) | 0;
  }
  return (k ^ l ^ r) | 0;
}

const base = 7;
function closureNest() {
  const a1 = 3;
  return function f2() {
    const a2 = 5;
    return function f3() {
      const a3 = 11;
      return function f4() {
        const a4 = 13;
        return function f5() {
          const a5 = 17;
          return function f6() {
            const a6 = 19;
            let s = 0;
            for (let i = 0; i < 1000000; i++) {
              s = (s + a6 + a5 + a4 + a3 + a2 + a1 + base + (i & 15)) | 0;
            }
            return s;
          };
        };
      };
    };
  };
}
const nestedRun = closureNest()()()()()
const nestResult = nestedRun();

function manyLocals(iters) {
  let v0 = 1, v1 = 2, v2 = 3, v3 = 4, v4 = 5, v5 = 6, v6 = 7, v7 = 8, v8 = 9, v9 = 10;
  let v10 = 11, v11 = 12, v12 = 13, v13 = 14, v14 = 15, v15 = 16, v16 = 17, v17 = 18, v18 = 19, v19 = 20;
  let acc = 0;
  for (let i = 0; i < iters; i++) {
    acc = (acc + v0 + v1 + v2 + v3 + v4 + v5 + v6 + v7 + v8 + v9 + v10 + v11 + v12 + v13 + v14 + v15 + v16 + v17 + v18 + v19 + i) | 0;
    v0 ^= acc;
    v19 += i & 7;
  }
  return acc;
}

const p1 = blowfishKernel(2000000);
const p2 = nestResult;
const p3 = manyLocals(200000);
let combined = 0;
combined = ((combined << 5) - combined + p1) | 0;
combined = ((combined << 5) - combined + p2) | 0;
combined = ((combined << 5) - combined + p3) | 0;
console.log("var_lookup " + combined);
