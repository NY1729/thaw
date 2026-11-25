import { v5, validate, version, parse, stringify, NIL } from "uuid";
// Only deterministic APIs: v5 (SHA-1 namespace hash), validate, version, parse/stringify. No v4/v7 (random/time).
// N = 2k v5 calls (estimate, see ms.ts).
function line(label: string, v: unknown): void {
  console.log(label + ":" + (typeof v === "string" ? v : JSON.stringify(v)));
}
const NS = "6ba7b810-9dad-11d1-80b4-00c04fd430c8"; // RFC 4122 DNS namespace
async function main(): Promise<void> {
  const N = 2000;
  const id = v5("hello.example.com", NS);
  line("v5", id);
  line("v5empty", v5("", NS));
  line("version", version(id));
  line("validate", [validate(id), validate("not-a-uuid"), validate(NIL)]);
  line("roundtrip", stringify(parse(id)) === id);
  line("parse", Array.from(parse(id)).slice(0, 4));
  let acc = 0;
  for (let i = 0; i < N; i++) {
    acc = (acc * 31 + parseInt(v5("n" + i, NS).slice(0, 8), 16) % 65536) | 0;
  }
  line("checksum", acc);
}
