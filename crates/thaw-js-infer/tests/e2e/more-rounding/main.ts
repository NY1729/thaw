import { round, roundUp, roundDown } from "more-rounding";
function main(): void {
  console.log(round(2.5), round(-2.5), roundUp(2.1), roundUp(-2.1), roundDown(2.9), roundDown(-2.9));
  let acc = 0;
  for (let i = 0; i < 40000; i++) {
    acc += round(i / 7) + roundUp(i / 3) + roundDown(i / 5);
  }
  console.log(acc);
}
