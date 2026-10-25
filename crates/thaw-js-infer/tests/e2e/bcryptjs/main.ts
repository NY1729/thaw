import { hashSync, getRounds, getSalt, truncates } from "bcryptjs";
function main(): void {
  const h = hashSync("secret", 4);
  console.log(getRounds(h), getSalt(h).length, truncates("short"), truncates("x".repeat(100)));
  let acc = 0;
  for (let i = 0; i < 20000; i++) {
    acc += getRounds(h) + getSalt(h).length + (truncates("pw" + i) ? 1 : 0);
  }
  console.log(acc);
}
