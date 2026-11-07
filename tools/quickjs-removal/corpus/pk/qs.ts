import { parse, stringify } from "qs";
// N = 1.2k parse+stringify round trips (estimate, see ms.ts).
function line(label: string, v: unknown): void {
  console.log(label + ":" + (typeof v === "string" ? v : JSON.stringify(v)));
}
async function main(): Promise<void> {
  const N = 1200;
  line("parse", parse("a[b][c]=1&a[b][d][]=2&a[b][d][]=3&x=y"));
  line("dupes", parse("a=1&a=2"));
  line("stringify", stringify({ a: { b: { c: 1 } }, arr: [1, 2] }, { encode: false }));
  line("brackets", stringify({ list: ["x", "y"] }, { arrayFormat: "brackets", encode: false }));
  line("enc", stringify({ q: "a b&c" }));
  let acc = 0;
  for (let i = 0; i < N; i++) {
    const s = stringify(parse(`a[${i % 10}][b]=${i}&c=x%20y`));
    acc = (acc * 31 + s.length) | 0;
  }
  line("checksum", acc);
}
