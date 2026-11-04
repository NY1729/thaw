// naive recursive fib(25); expected checksum: fib 75025
function fib(n) {
  return n < 2 ? n : fib(n - 1) + fib(n - 2);
}
console.log("fib " + fib(25));
