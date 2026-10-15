// Wave T: an async function returning a promise settles with the promise's value, also for a
// `for...of` element of a possibly sparse array. Expected output recorded from node 22.

#[test]
fn t_contract_async_returns_promise_of_loop_element() {
    let source = r##"async function l(xs: number[]): Promise<number> {
  for (const x of xs) {
    if (x > 1) return Promise.resolve(x);
  }
  return -1;
}
async function s(xs: string[]): Promise<string> {
  for (const x of xs) { return Promise.resolve(x); }
  return 'none';
}
async function main(): Promise<void> {
  console.log(await l([1, 2, 3]));
  console.log(await l([0, 1]));
  console.log(await s(['a', 'b']));
  console.log(await s([]));
}
"##;
    assert_eq!(
        compile_and_run(source, "t_contract_async_returns_promise_of_loop_element"),
        "2\n-1\na\nnone\n"
    );
}

#[test]
fn t_contract_async_returns_promise_in_every_function_shape() {
    let source = r##"async function g(n: number): Promise<number> { return n + 1; }
async function f(n: number): Promise<number> { return g(n); }
const h = async (n: number): Promise<string> => { const p: Promise<string> = Promise.resolve('s' + n); return p; };
class K { async m(n: number): Promise<number> { return Promise.resolve(n * 2); } }
async function t(n: number): Promise<number> { try { return Promise.resolve(n); } finally { console.log('fin'); } }
async function main(): Promise<void> {
  console.log(await f(1), await h(2), await new K().m(3), await t(4));
}
"##;
    assert_eq!(
        compile_and_run(source, "t_contract_async_returns_promise_in_every_function_shape"),
        "fin\n2 s2 6 4\n"
    );
}
