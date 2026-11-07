import { Chalk } from "chalk";
// Level forced to 1 (no TTY detection), so escapes are fixed. N = 50k (estimate, see ms.ts).
function line(label: string, v: unknown): void {
  console.log(label + ":" + (typeof v === "string" ? v : JSON.stringify(v)));
}
const chalk = new Chalk({ level: 1 });
async function main(): Promise<void> {
  const N = 50000;
  line("red", chalk.red("error"));
  line("bold", chalk.bold("x"));
  line("combo", chalk.bgBlue.white.underline("combo"));
  line("hex", chalk.hex("#ff8800")("hex"));
  line("rgb", chalk.rgb(10, 20, 30)("rgb"));
  let acc = 0;
  for (let i = 0; i < N; i++) {
    acc = (acc * 31 + chalk.green(String(i)).length) | 0;
  }
  line("checksum", acc);
}
