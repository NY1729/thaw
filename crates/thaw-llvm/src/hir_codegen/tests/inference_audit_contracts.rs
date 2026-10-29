// Contextual callback typing for function-valued globals and recursion through them, plus
// async arrows awaiting a plain Promise-returning function. Expected output recorded from node 22.

#[test]
fn contextually_typed_global_callbacks_and_recursive_global_functions() {
    let source = r##"const add: (a: number, b: number) => number = (x, y) => x + y;
const fib = (n: number): number => (n < 2 ? n : fib(n - 1) + fib(n - 2));
const fact = function g(n: number): number { return n <= 1 ? 1 : n * g(n - 1); };
const handlers: { on: (cb: (v: string) => void) => void } = { on(cb) { cb("hi"); } };
function main(): void {
  console.log(add(1, 2), fib(15), fact(5));
  handlers.on((v) => console.log(v));
}
"##;
    assert_eq!(
        compile_and_run(source, "contextually_typed_global_callbacks_and_recursive_global_functions"),
        "3 610 120\nhi\n"
    );
}

#[test]
fn async_arrows_await_plain_promise_returning_functions() {
    let source = r##"const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));
function pause(ms: number): Promise<void> { return new Promise<void>((r) => setTimeout(r, ms)); }
async function main(): Promise<void> {
  const rs = await Promise.all([1, 2, 3].map(async (x) => { await sleep(5 * (4 - x)); return x; }));
  console.log(rs);
  const g = async (x: number) => { await pause(1); return x * 2; };
  console.log(await g(21));
  let n = 0;
  await Promise.all([1, 2].map(async (x) => { await pause(1); n += x; }));
  console.log(n);
}
"##;
    assert_eq!(
        compile_and_run(source, "async_arrows_await_plain_promise_returning_functions"),
        "[ 1, 2, 3 ]\n42\n3\n"
    );
}
