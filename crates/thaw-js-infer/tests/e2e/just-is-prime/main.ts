import isPrime from "just-is-prime";
function main(): void {
  console.log(isPrime(1), isPrime(2), isPrime(17), isPrime(10), isPrime(97), isPrime(-5));
  let count = 0;
  for (let i = 0; i < 20000; i++) {
    if (isPrime(i)) {
      count++;
    }
  }
  console.log(count);
}
