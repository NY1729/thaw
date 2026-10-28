import lerp from "lerpjs";
function main(): void {
  console.log(lerp(0, 10, 0.5), lerp(-4, 4, 0.25), lerp(1, 1, 0.7));
  let acc = 0;
  for (let i = 0; i < 40000; i++) {
    acc += lerp(i, i + 10, 0.3);
  }
  console.log(acc);
}
