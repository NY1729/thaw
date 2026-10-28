import { isLeapYear } from "leap-year-checker";
function main(): void {
  console.log(isLeapYear(1900), isLeapYear(2000), isLeapYear(2024), isLeapYear(2023));
  let n = 0;
  for (let y = 1; y < 30000; y++) {
    if (isLeapYear(y)) {
      n++;
    }
  }
  console.log(n);
}
