import { hashSync, compareSync } from "bcryptjs";
// Fixed salt string, cost 4 (16 rounds): no random salt generation, so hashes are reproducible.
const SALT = "$2a$04$abcdefghijklmnopqrstuv";
function line(label: string, v: unknown): void {
  console.log(label + ":" + (typeof v === "string" ? v : JSON.stringify(v)));
}
async function main(): Promise<void> {
  const h = hashSync("password123", SALT);
  line("hash", h);
  line("empty", hashSync("", SALT));
  line("prefix", h.slice(0, 7));
  line("compareOk", compareSync("password123", h));
  line("compareBad", compareSync("wrong", h));
  line("unicode", hashSync("pässwörd-日本", SALT));
  // WORKLOAD: N=40 cost-4 hashes; each bcrypt hash is pure-JS and expensive under QuickJS. Estimate only.
  const N = 40;
  let acc = 0;
  for (let i = 0; i < N; i++) {
    acc = (acc * 31 + hashSync("pw" + i, SALT).charCodeAt(40)) | 0;
  }
  line("checksum", acc);
}
