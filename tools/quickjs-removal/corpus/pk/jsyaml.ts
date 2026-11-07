import { load, dump } from "js-yaml";
// fixed YAML document in, round trip out. N = 20k load+dump (estimate, see ms.ts).
function line(label: string, v: unknown): void {
  console.log(label + ":" + (typeof v === "string" ? v : JSON.stringify(v)));
}
const DOC = "name: thaw\nversion: 1\ntags: [a, b]\nnested:\n  k: true\n  list:\n    - 1.5\n    - null\n    - '007'\n";
async function main(): Promise<void> {
  const N = 400;
  const obj = load(DOC);
  line("load", obj);
  line("dump", dump(obj));
  line("dumpNew", dump({ b: 1, a: [1, 2, { c: "x" }] }));
  let acc = 0;
  for (let i = 0; i < N; i++) {
    const o = load("k: " + i + "\nlist: [" + i + ", 2]\n") as { k: number; list: number[] };
    acc = (acc * 31 + dump(o).length + o.list[0]) | 0;
  }
  line("checksum", acc);
}
