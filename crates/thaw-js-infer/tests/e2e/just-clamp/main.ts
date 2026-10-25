import clamp from "just-clamp";
function main(): void {
  console.log(clamp(5, 1, 3), clamp(0, 1, 3), clamp(2, 1, 3), clamp(3, 2, 2), clamp(parseFloat("x"), 1, 3), clamp(9, 8.5, 1.5));
  let acc = 0;
  for (let i = 0; i < 40000; i++) {
    acc += clamp(i % 17, 3, 11);
  }
  console.log(acc);
}
