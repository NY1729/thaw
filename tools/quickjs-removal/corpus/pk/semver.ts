import { satisfies, inc, compare, valid, coerce, diff, validRange, sort } from "semver";
// N = 5k over a fixed 200-version list (estimate, see ms.ts).
function line(label: string, v: unknown): void {
  console.log(label + ":" + (typeof v === "string" ? v : JSON.stringify(v)));
}
async function main(): Promise<void> {
  const N = 5000;
  const vers: string[] = Array.from({ length: 200 }, (_, i) => `${i % 7}.${(i * 3) % 11}.${(i * 5) % 13}`);
  line("satisfies", satisfies("1.2.3", "^1.2.0"));
  line("inc", inc("1.2.3", "minor"));
  line("compare", compare("1.0.0-alpha", "1.0.0"));
  line("diff", diff("1.0.0", "2.0.0-rc.1"));
  line("valid", valid("v1.2.3"));
  line("validRange", validRange(">=1.2 <2"));
  const c = coerce("v3.4 foo");
  line("coerce", c === null ? null : c.version);
  line("sort", sort(["1.10.0", "1.2.0", "1.2.0-beta.1"]));
  let acc = 0;
  for (let i = 0; i < N; i++) {
    const a = vers[i % 200];
    const b = vers[(i * 7) % 200];
    const s = satisfies(a, "^3.5.0") ? "y" : "n";
    acc = (acc * 31 + compare(a, b) + String(inc(a, "patch")).length + s.length) | 0;
  }
  line("checksum", acc);
}
