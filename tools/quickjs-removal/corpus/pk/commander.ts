import { Command } from "commander";
// exitOverride + configureOutput keep --help/errors from exiting or printing; output is captured.
function line(label: string, v: unknown): void {
  console.log(label + ":" + (typeof v === "string" ? v : JSON.stringify(v)));
}
function build(): { program: Command; out: string[]; result: { file?: string; opts?: any } } {
  const out: string[] = [];
  const result: { file?: string; opts?: any } = {};
  const program = new Command();
  program.name("tool").exitOverride()
    .configureOutput({ writeOut: (s) => out.push(s), writeErr: (s) => out.push("E:" + s) })
    .option("-v, --verbose", "verbose output").option("-n, --num <n>", "a number", "3")
    .argument("[file]", "input file")
    .action((file: string | undefined) => { result.file = file; result.opts = program.opts(); });
  return { program, out, result };
}
async function main(): Promise<void> {
  const a = build();
  a.program.parse(["-v", "-n", "7", "data.txt"], { from: "user" });
  line("parsed", a.result);
  const b = build();
  try { b.program.parse(["--help"], { from: "user" }); } catch (e) { line("help", [(e as any).code, b.out.join("")]); }
  const c = build();
  try { c.program.parse(["--bogus"], { from: "user" }); } catch (e) { line("unknown", (e as any).code); }
  // WORKLOAD: N=300 fresh parsers (estimate).
  const N = 300;
  let acc = 0;
  for (let i = 0; i < N; i++) {
    const w = build();
    w.program.parse(["-n", String(i), "file" + i], { from: "user" });
    acc = (acc * 31 + String(w.result.file).length + Number(w.result.opts.num)) | 0;
  }
  line("checksum", acc);
}
