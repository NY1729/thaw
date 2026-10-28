import { evens, scale, words } from "aot-arrays";
function main(): void {
  console.log(evens(10), scale([1, 2, 3], 2.5), words("a bc  d"));
  let acc = 0;
  for (let i = 0; i < 3000; i++) {
    const e = evens(20);
    const s = scale(e, 3);
    acc += e.length + s[3] + words("x y z").length;
  }
  console.log(acc);
}
