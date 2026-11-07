import { chunk, groupBy, merge, get, sortBy, uniq } from "lodash";
// N = 389 loop iterations of small lodash calls (estimate, see ms.ts).
function line(label: string, v: unknown): void {
  console.log(label + ":" + (typeof v === "string" ? v : JSON.stringify(v)));
}
async function main(): Promise<void> {
  const N = 389;
  line("chunk", chunk([1, 2, 3, 4, 5, 6, 7], 3));
  line("groupBy", groupBy(["one", "two", "three", "four"], (s: string) => s.length));
  line("merge", merge({ a: { x: 1, y: [1, 2] } }, { a: { y: [3] }, b: 2 }));
  line("get", get({ a: [{ b: "c" }] }, "a[0].b"));
  line("sortBy", sortBy([{ n: "b", v: 2 }, { n: "a", v: 2 }, { n: "c", v: 1 }], ["v", "n"]));
  line("uniq", uniq([3, 1, 3, 2, 1]));
  let acc = 0;
  for (let i = 0; i < N; i++) {
    const c = chunk([i, i + 1, i + 2, i + 3], 2);
    acc = (acc * 31 + String(get(merge({}, { v: c[1][0] }), "v")).length + c.length) | 0;
  }
  line("checksum", acc);
}
