import minimist from "minimist";
// minimist is CommonJS (module.exports = fn): the default import is the function itself.
function line(label: string, v: unknown): void {
  console.log(label + ":" + (typeof v === "string" ? v : JSON.stringify(v)));
}
async function main(): Promise<void> {
  line("basic", minimist(["-a", "1", "--bee=two", "--no-cee", "-d", "x", "rest"]));
  line("defaults", minimist(["-x"], { default: { x: 7, y: "d" }, boolean: ["flag"] }));
  line("alias", minimist(["-h", "-n", "5", "--code", "007"], { alias: { h: "help", n: "num" }, string: ["code"] }));
  line("dashes", minimist(["--", "-not-a-flag", "--also"]));
  line("nested", minimist(["--a.b.c", "3", "--a.d", "x", "--list", "1", "--list", "2"]));
  line("bool", minimist(["--yes", "--no=false", "-zv"], { boolean: ["yes", "no", "z", "v"] }));
  line("numbers", minimist(["-n", "12.5", "-m", "0x10", "-s", "12"], { string: ["s"] }));
  // WORKLOAD: N=20000 is an estimate (no QuickJS calibration possible here); lower it if the QuickJS run exceeds ~400 ms.
  const N = 20000;
  let acc = 0;
  for (let i = 0; i < N; i++) {
    acc = (acc * 31 + JSON.stringify(minimist(["--k" + (i % 50), String(i), "-v", "--n.x", String(i * 2)])).length) | 0;
  }
  line("checksum", acc);
}
