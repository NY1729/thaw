import { area, label, longest, allTrue, grow } from "aot-shapes";
function main(): void {
  const words: string[] = ["a", "ccc", "bb"];
  console.log(area({ w: 3, h: 4 }), label({ name: "n", tags: ["x", "y"] }), longest(words), allTrue([true, false]), allTrue([true]));
  const a: number[] = [1];
  console.log(grow(a), a.length);
  let acc = 0;
  for (let i = 0; i < 20000; i++) {
    acc += area({ w: i, h: 2 }) + longest(["a" + i, "bcd"]).length;
  }
  console.log(acc);
}
