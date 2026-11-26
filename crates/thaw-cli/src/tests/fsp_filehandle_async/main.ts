import * as fsp from "node:fs/promises";

const guard = async (name: string, fn: any) => {
  try {
    console.log(name, await fn());
  } catch (e: any) {
    console.log(name, "caught", e.code === undefined ? e.message : e.code);
  }
};

async function main() {
  const p = process.cwd() + "/fh.txt";
  await fsp.writeFile(p, "hello");
  await guard("stat", async () => {
    const h: any = await fsp.open(p, "r+");
    try { return (await h.stat()).size; } finally { await h.close(); }
  });
  await guard("write-reject", async () => {
    const h: any = await fsp.open(p, "r+");
    try { return await h.writeFile(null); } finally { await h.close(); }
  });
  await guard("write-rethrow", async () => {
    const h = await fsp.open(p, "r+");
    try { return await h.writeFile(null as any); } catch (e) { throw e; } finally { await h.close(); }
  });
  await guard("closed-rethrow", async () => {
    const h: any = await fsp.open(p, "r+");
    await h.close();
    try { return await h.stat(); } catch (e) { throw e; }
  });
  await guard("nested-await-call", async () => (await fsp.open(p, "r")).close());
  await guard("nested-await-stat", async () => await (await fsp.open(p, "r")).stat() !== undefined);
  await guard("statfs", async () => await typeof (await fsp.statfs(process.cwd())).bsize);
  await guard("union-catch", async () => {
    const h: any = await fsp.open(p, "r+");
    await h.close();
    try { return await h.stat(); } catch (e) { return "c"; }
  });
  await guard("union-return", async () => {
    await 0;
    try { await 0; return 1; } catch (e) { return "c"; }
  });
  const rethrow = async () => {
    await 0;
    try { throw new Error("t"); } catch (e) { throw new Error("n"); }
  };
  try { await rethrow(); } catch (e: any) { console.log("rethrow-after-await caught", e.message); }
}
