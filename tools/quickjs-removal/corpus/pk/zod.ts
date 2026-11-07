import { z } from "zod";
// N = 8k safeParse calls (estimate, see ms.ts).
function line(label: string, v: unknown): void {
  console.log(label + ":" + (typeof v === "string" ? v : JSON.stringify(v)));
}
const User = z.object({
  name: z.string().min(2),
  age: z.number().int().min(0),
  tags: z.array(z.string()).default([]),
});
async function main(): Promise<void> {
  const N = 8000;
  const ok = User.safeParse({ name: "Ada", age: 36, tags: ["x"] });
  line("ok", ok.success ? ok.data : null);
  const bad = User.safeParse({ name: "A", age: -1 });
  line("issues", bad.success ? null : bad.error.issues.map((i) => ({ path: i.path, code: i.code, message: i.message })));
  let acc = 0;
  for (let i = 0; i < N; i++) {
    const r = User.safeParse({ name: "n" + i, age: i % 100 - 5, tags: ["t"] });
    acc = (acc * 31 + (r.success ? 1 : 2)) | 0;
  }
  line("checksum", acc);
}
