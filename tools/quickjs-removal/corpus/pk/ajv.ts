import Ajv from "ajv";
// Ajv is CommonJS (module.exports = class): default import.
function line(label: string, v: unknown): void {
  console.log(label + ":" + (typeof v === "string" ? v : JSON.stringify(v)));
}
async function main(): Promise<void> {
  const ajv = new Ajv({ allErrors: true });
  const validate = ajv.compile({
    type: "object",
    properties: {
      name: { type: "string", minLength: 2 },
      age: { type: "integer", minimum: 0 },
      tags: { type: "array", items: { type: "string" } },
    },
    required: ["name", "age"],
    additionalProperties: false,
  });
  line("valid", validate({ name: "Al", age: 3, tags: ["x"] }));
  line("invalid", validate({ name: "A", age: -1, tags: [1], extra: true }));
  line("errors", validate.errors);
  // WORKLOAD: N=5000 validations of a compiled schema (estimate).
  const N = 5000;
  let acc = 0;
  for (let i = 0; i < N; i++) {
    const ok = validate({ name: "n" + i, age: (i % 50) - 5, tags: [i % 3 === 0 ? 1 : "t"] });
    acc = (acc * 31 + String(ok ? 1 : (validate.errors ?? []).length).length + (i % 7)) | 0;
  }
  line("checksum", acc);
}
