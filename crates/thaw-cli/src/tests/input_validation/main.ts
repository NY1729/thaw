import * as fs from "node:fs";
import * as fsp from "node:fs/promises";
import * as path from "node:path";
import * as url from "node:url";
import * as util from "node:util";
import * as qs from "node:querystring";

function t(label: string, f: () => any): void {
  try { console.log(label, "ok", String(f())); } catch (e: any) { console.log(label, e.name, e.code, e.message); }
}
async function run(): Promise<void> {
  const shapes: any[] = [["a"], new Date(0), { a: 1 }, 5, null, undefined, 5n];
  for (let i = 0; i < shapes.length; i++) {
    const v: any = shapes[i];
    t("stat" + i, () => fs.statSync(v));
    t("readdir" + i, () => fs.readdirSync(v));
    t("exists" + i, () => fs.existsSync(v));
    t("rename" + i, () => fs.renameSync("/nonexistent-r75-a", v));
    t("copy" + i, () => fs.copyFileSync(v, "/nonexistent-r75-b"));
    t("basename" + i, () => path.basename(v));
    t("join" + i, () => path.join("a", v));
    t("f2p" + i, () => url.fileURLToPath(v));
    t("p2f" + i, () => url.pathToFileURL(v));
    t("strip" + i, () => util.stripVTControlCharacters(v));
    t("qs" + i, () => JSON.stringify(qs.parse(v)));
  }
  const u8: any = new Uint8Array([97]);
  t("stat-u8", () => fs.statSync(u8));
  t("f2p-http", () => url.fileURLToPath("http://x/a"));
  t("f2p-host", () => url.fileURLToPath("file://host/a"));
  t("f2p-slash", () => url.fileURLToPath("file:///a%2Fb"));
  t("f2p-ok", () => url.fileURLToPath("file:///a/b"));
  t("nul", () => fs.statSync("a\0b"));
  const bad: any = ["a"];
  fs.stat("/nonexistent-r75-c", () => {});
  t("cb", () => fs.stat(bad, () => {}));
  t("rs", () => fs.createReadStream(bad));
  const big: any = 5n;
  try { await fsp.stat(bad); } catch (e: any) { console.log("fsp", e.code, e.message); }
  try { await fsp.readFile(big); } catch (e: any) { console.log("fsp", e.code, e.message); }
}
run().then(() => {});
