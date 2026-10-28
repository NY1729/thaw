import { numericSort, equalIntervalBreaks } from "simple-statistics";
function main(): void {
  const xs: number[] = [5, 3, 9, 1, 7, 2];
  console.log(numericSort(xs));
  console.log(xs);
  console.log(equalIntervalBreaks(xs, 3));
  let acc = 0;
  for (let i = 0; i < 2000; i++) {
    const r = numericSort([i % 7, 3, i % 5, 1]);
    acc += r[0] + r[3];
  }
  console.log(acc);
}
