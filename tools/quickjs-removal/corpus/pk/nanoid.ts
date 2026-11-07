import { nanoid, customAlphabet, customRandom } from "nanoid";
// Random output is never printed: only lengths and charset shape. The workload uses customRandom
// with a deterministic byte source, so its values are fixed. N = 40k (estimate, see ms.ts).
function line(label: string, v: unknown): void {
  console.log(label + ":" + (typeof v === "string" ? v : JSON.stringify(v)));
}
const ALPHA = "abcdefghijklmnopqrstuvwxyz0123456789";
const det = (n: number): Uint8Array => {
  const b = new Uint8Array(n);
  for (let i = 0; i < n; i++) b[i] = (i * 37 + 11) & 255;
  return b;
};
async function main(): Promise<void> {
  const N = 25000;
  line("len", nanoid().length);
  line("shape", /^[A-Za-z0-9_-]+$/.test(nanoid(64)));
  line("customLen", customAlphabet("abc", 9)().length);
  line("customShape", /^[abc]{9}$/.test(customAlphabet("abc", 9)()));
  const gen = customRandom(ALPHA, 12, det);
  line("fixed", gen());
  let acc = 0;
  for (let i = 0; i < N; i++) {
    const s = gen();
    acc = (acc * 31 + s.charCodeAt(i % 12)) | 0;
  }
  line("checksum", acc);
}
