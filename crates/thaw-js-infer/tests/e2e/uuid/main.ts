import { validate, version, v5 } from "uuid";
function main(): void {
  const id = v5("hello", "6ba7b810-9dad-11d1-80b4-00c04fd430c8");
  console.log(id, validate(id), validate("not-a-uuid"), version(id));
}
