import ms from "ms";
// default import: ms is CJS with a default-only export (measure.sh sed rewrite does not apply).
// Workload N calibrated on Node (~8 ms in-process workload); QuickJS ratio is an estimate, no QuickJS run possible.
function line(label: string, v: unknown): void {
  console.log(label + ":" + (typeof v === "string" ? v : JSON.stringify(v)));
}
async function main(): Promise<void> {
  const N = 100000;
  for (const s of ["1s", "2.5 hours", "-3 days", "100ms", "1w", "0", "1y", "bogus"]) line("parse " + s, ms(s));
  for (const n of [1000, 90061000, -86400000, 0, 1]) line("format " + n, ms(n));
  let acc = 0;
  for (let i = 0; i < N; i++) acc = (acc * 31 + String(ms(i * 7919)).length) | 0;
  line("checksum", acc);
}
