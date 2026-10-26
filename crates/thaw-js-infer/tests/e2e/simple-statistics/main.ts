import { sum, mean, max, min, variance, product, geometricMean, zScore } from "simple-statistics";
function main(): void {
  const xs: number[] = [3, 1, 4, 1, 5, 9, 2, 6];
  // `variance` stays a QuickJS call: it gets its own array (an array shared with a typed JS call
  // lives in dynamic storage and makes the clone's array accesses slow)
  const vs: number[] = [3, 1, 4, 1, 5, 9, 2, 6];
  console.log(sum(xs), mean(xs), max(xs), min(xs), variance(vs), product([1, 2, 3, 4]), geometricMean(xs), zScore(5, 3, 2));
  try {
    console.log(max([]));
  } catch (e) {
    console.log("max([]) threw");
  }
  let acc = 0;
  for (let i = 0; i < 20000; i++) {
    const ys: number[] = [i, i + 1, i + 2, i * 2];
    acc += sum(ys) + mean(ys) + max(ys) + min(ys);
  }
  console.log(acc);
}
