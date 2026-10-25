import { sum, max } from "simple-statistics";
// the arguments are `Json`: the static types prove nothing, so the clone sits behind a run-time guard
// and a contract violation (`"2"` in a number array) takes the original QuickJS path
function main(): void {
  const good: Json = JSON.parse("[1,2,3]");
  const bad: Json = JSON.parse('[1,"2",3]');
  console.log(sum(good), max(good), sum(bad), max(bad));
}
