import { yearsToMonths, minutesToSeconds } from "date-fns";
function main(): void {
  let acc = 0;
  for (let i = 0; i < 30000; i++) {
    acc += yearsToMonths(i % 50) + minutesToSeconds(i % 90);
  }
  console.log(acc);
  console.log(yearsToMonths(3), minutesToSeconds(2));
}
