// Wave R: `return` inside loops of async (and sync for-of) functions. Expected output recorded from node 22.

#[test]
fn r_contract_for_of_await_return() {
    let source = r##"async function f(xs: number[]): Promise<number> {
  for (const x of xs) {
    if (x > 1) return x;
    await Promise.resolve();
  }
  return -1;
}
async function main(): Promise<void> {
  console.log(await f([1, 2, 3]));
  console.log(await f([0, 1]));
}
"##;
    assert_eq!(
        compile_and_run(source, "r_contract_for_of_await_return"),
        "2\n-1\n"
    );
}

#[test]
fn r_contract_while_true_await_return() {
    let source = r##"function tick(): Promise<void> { return Promise.resolve(); }
async function g(): Promise<string> {
  while (true) {
    await tick();
    return 'a';
  }
}
async function main(): Promise<void> {
  console.log(await g());
}
"##;
    assert_eq!(
        compile_and_run(source, "r_contract_while_true_await_return"),
        "a\n"
    );
}

#[test]
fn r_contract_loops_without_await_return() {
    let source = r##"async function f(n: number): Promise<number> {
  for (let i = 0; i < n; i++) {
    if (i === 2) return i * 10;
  }
  return -1;
}
async function w(n: number): Promise<number> {
  let i = 0;
  while (i < n) {
    i++;
    if (i === 3) return i;
    await Promise.resolve();
  }
  return -1;
}
async function d(n: number): Promise<string> {
  let i = 0;
  do {
    i++;
    if (i > 1) return 'do' + i;
  } while (i < n);
  return 'none';
}
async function main(): Promise<void> {
  console.log(await f(5), await f(1));
  console.log(await w(5), await w(2));
  console.log(await d(5), await d(1));
}
"##;
    assert_eq!(
        compile_and_run(source, "r_contract_loops_without_await_return"),
        "20 -1\n3 -1\ndo2 none\n"
    );
}

#[test]
fn r_contract_for_of_return_sync_element() {
    let source = r##"function f(xs: number[]): number {
  for (const x of xs) { return x; }
  return -1;
}
function main(): void {
  console.log(f([1, 2, 3]), f([0]));
}
"##;
    assert_eq!(
        compile_and_run(source, "r_contract_for_of_return_sync_element"),
        "1 0\n"
    );
}

#[test]
fn r_contract_for_of_return_in_if() {
    let source = r##"function f(xs: number[]): number {
  for (const x of xs) { if (x > 1) { return x; } }
  return -1;
}
function main(): void {
  console.log(f([1, 2, 3]), f([0]));
}
"##;
    assert_eq!(
        compile_and_run(source, "r_contract_for_of_return_in_if"),
        "2 -1\n"
    );
}

#[test]
fn r_contract_for_of_string_return() {
    let source = r##"async function f(xs: string[]): Promise<string> {
  for (const x of xs) {
    if (x.length > 1) return x;
  }
  return 'none';
}
async function main(): Promise<void> {
  console.log(await f(['a', 'bb']), await f(['c']));
}
"##;
    assert_eq!(
        compile_and_run(source, "r_contract_for_of_string_return"),
        "bb none\n"
    );
}

#[test]
fn r_contract_await_around_return_matrix() {
    let source = r##"function tick(): Promise<void> { return Promise.resolve(); }
async function whileAwaitBefore(n: number): Promise<number> {
  let i = 0;
  while (i < n) { await tick(); i++; if (i === 2) return i * 100; }
  return -1;
}
async function whileAwaitAfter(n: number): Promise<number> {
  let i = 0;
  while (i < n) { i++; if (i === 2) return i * 100; await tick(); }
  return -1;
}
async function doWhile(n: number): Promise<string> {
  let i = 0;
  do { i++; await tick(); if (i === 3) return 'd' + i; } while (i < n);
  return 'end';
}
async function forAwaitTick(n: number): Promise<number> {
  for (let i = 0; i < n; i++) { await tick(); if (i === 1) return i + 0.5; }
  return -1;
}
async function forOfAwait(xs: string[]): Promise<string> {
  for (const x of xs) { await tick(); if (x === 'b') return x + '!'; }
  return 'nope';
}
async function nested(): Promise<number> {
  for (let i = 0; i < 3; i++) {
    for (let j = 0; j < 3; j++) {
      await tick();
      if (i === 1 && j === 2) return i * 10 + j;
    }
  }
  return -1;
}
async function labelled(): Promise<number> {
  outer: for (let i = 0; i < 3; i++) {
    for (let j = 0; j < 3; j++) {
      if (j === 1) continue outer;
      if (i === 2) return i + j;
      await tick();
    }
  }
  return -1;
}
async function viaBreak(): Promise<string> {
  let r = 'x';
  while (true) { await tick(); r = 'y'; break; }
  return r;
}
async function main(): Promise<void> {
  console.log(await whileAwaitBefore(5), await whileAwaitBefore(1));
  console.log(await whileAwaitAfter(5), await whileAwaitAfter(1));
  console.log(await doWhile(9), await doWhile(2));
  console.log(await forAwaitTick(5), await forAwaitTick(1));
  console.log(await forOfAwait(['a', 'b']), await forOfAwait(['c']));
  console.log(await nested());
  console.log(await labelled());
  console.log(await viaBreak());
}
"##;
    assert_eq!(
        compile_and_run(source, "r_contract_await_around_return_matrix"),
        "200 -1\n200 -1\nd3 end\n1.5 -1\nb! nope\n12\n2\ny\n"
    );
}

#[test]
fn r_contract_try_finally_catch_void_object_any_return() {
    let source = r##"function tick(): Promise<void> { return Promise.resolve(); }
async function tryRet(n: number): Promise<number> {
  for (let i = 0; i < n; i++) {
    try {
      await tick();
      if (i === 1) return i;
    } finally {
      console.log('fin', i);
    }
  }
  return -1;
}
async function catchRet(): Promise<string> {
  for (let i = 0; i < 3; i++) {
    try {
      await tick();
      if (i === 1) throw new Error('boom');
    } catch (e: any) {
      return 'caught ' + e.message;
    }
  }
  return 'none';
}
async function voidRet(n: number): Promise<void> {
  for (let i = 0; i < n; i++) {
    await tick();
    if (i === 1) { console.log('early'); return; }
  }
  console.log('late');
}
async function objRet(n: number): Promise<{ a: number; b: string }> {
  for (let i = 0; i < n; i++) {
    await tick();
    if (i === 1) return { a: i, b: 'o' };
  }
  return { a: -1, b: 'none' };
}
async function anyRet(n: number): Promise<any> {
  for (let i = 0; i < n; i++) {
    if (i === 1) return { k: i };
  }
  return null;
}
async function main(): Promise<void> {
  console.log(await tryRet(4), await tryRet(1));
  console.log(await catchRet());
  await voidRet(5); await voidRet(1);
  const o = await objRet(3); console.log(o.a, o.b);
  const q = await objRet(1); console.log(q.a, q.b);
  console.log(JSON.stringify(await anyRet(3)), JSON.stringify(await anyRet(1)));
}
"##;
    assert_eq!(
        compile_and_run(source, "r_contract_try_finally_catch_void_object_any_return"),
        "fin 0\nfin 1\nfin 0\n1 -1\ncaught boom\nearly\nlate\n1 o\n-1 none\n{\"k\":1} null\n"
    );
}

#[test]
fn r_contract_class_arrow_for_in_for_await_return() {
    let source = r##"function tick(): Promise<void> { return Promise.resolve(); }
class K {
  base: number = 10;
  async find(xs: number[]): Promise<number> {
    for (const x of xs) { if (x > this.base) return x; }
    return -1;
  }
  async findAwait(xs: number[]): Promise<number> {
    for (const x of xs) { await tick(); if (x > this.base) return x; }
    return -1;
  }
}
const arrow = async (xs: number[]): Promise<string> => {
  for (const x of xs) { if (x > 2) return 'arrow' + x; }
  return 'none';
};
async function innerLambda(xs: number[]): Promise<number> {
  let total = 0;
  for (const x of xs) {
    const f = (v: number): number => { if (v > 1) return v * 2; return 0; };
    total += f(x);
    await tick();
  }
  return total;
}
async function forIn(): Promise<string> {
  const o = { a: 1, b: 2, c: 3 };
  for (const k in o) { await tick(); if (k === 'b') return 'key ' + k; }
  return 'none';
}
async function* gen(): AsyncGenerator<number> { yield 1; yield 2; yield 3; }
async function consume(): Promise<number> {
  for await (const v of gen()) { if (v === 2) return v * 7; }
  return -1;
}
async function main(): Promise<void> {
  const k = new K();
  console.log(await k.find([1, 11]), await k.find([1]), await k.findAwait([1, 12]), await k.findAwait([2]));
  console.log(await arrow([1, 2, 3]), await arrow([1]));
  console.log(await innerLambda([1, 2, 3]));
  console.log(await forIn());
  console.log(await consume());
}
"##;
    assert_eq!(
        compile_and_run(source, "r_contract_class_arrow_for_in_for_await_return"),
        "11 -1 12 -1\narrow3 none\n10\nkey b\n14\n"
    );
}
