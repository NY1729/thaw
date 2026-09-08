function fib(n: number): number {
  return n < 2 ? n : fib(n - 1) + fib(n - 2);
}

function main(): void {
  console.log(fib(40));
}
