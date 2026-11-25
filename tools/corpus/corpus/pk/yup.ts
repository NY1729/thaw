import { object, string, number, array } from "yup";
function line(label: string, v: unknown): void {
  console.log(label + ":" + (typeof v === "string" ? v : JSON.stringify(v)));
}
const schema = object({
  name: string().required().min(2),
  age: number().integer().min(0).required(),
  tags: array(string()).default([]),
});
async function main(): Promise<void> {
  line("ok", schema.validateSync({ name: "Al", age: 3 }));
  try {
    schema.validateSync({ name: "A", age: -1 }, { abortEarly: false });
  } catch (e: any) {
    line("errors", e.errors);
  }
  line("isValid", schema.isValidSync({}));
  line("async", await schema.validate({ name: "Bo", age: 5, tags: ["x"] }));
  // WORKLOAD: N=3000 synchronous validations (estimate).
  const N = 3000;
  let acc = 0;
  for (let i = 0; i < N; i++) {
    const ok = schema.isValidSync({ name: "n".repeat(i % 5), age: (i % 9) - 2 });
    acc = (acc * 31 + (ok ? 1 : 0) + (i % 4)) | 0;
  }
  line("checksum", acc);
}
