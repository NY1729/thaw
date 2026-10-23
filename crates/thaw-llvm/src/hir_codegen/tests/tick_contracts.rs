// process.nextTick vs promise ordering depends on the entry's module kind. Expected output
// recorded from node v22: a CommonJS script (.cts, no import/export) vs an ES module (.mts).

fn run_as_commonjs(source: &str, test_name: &str) -> String {
    let output = compile_and_run_raw_as(source, test_name, &[], true);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).into_owned()
}

const TICK_VS_PROMISE: &str = "function main(): void { console.log('sync'); process.nextTick((): void => console.log('tick')); Promise.resolve().then((): void => console.log('promise')); queueMicrotask((): void => console.log('microtask')); }";

#[test]
fn tick_contract_commonjs_runs_ticks_before_promises() {
    assert_eq!(
        run_as_commonjs(TICK_VS_PROMISE, "tick_commonjs_sync"),
        "sync\ntick\npromise\nmicrotask\n"
    );
}

#[test]
fn tick_contract_es_module_runs_promises_before_ticks() {
    assert_eq!(
        compile_and_run(TICK_VS_PROMISE, "tick_esm_sync"),
        "sync\npromise\nmicrotask\ntick\n"
    );
}

#[test]
fn tick_contract_commonjs_async_main_ticks_first() {
    let source = "async function main(): Promise<void> { const order: string[] = []; process.nextTick(() => order.push('tick')); Promise.resolve().then(() => order.push('promise')); queueMicrotask(() => order.push('microtask')); await new Promise<void>((resolve) => setImmediate(resolve)); console.log(order.join(',')); }";
    assert_eq!(run_as_commonjs(source, "tick_commonjs_async"), "tick,promise,microtask\n");
    assert_eq!(compile_and_run(source, "tick_esm_async"), "promise,microtask,tick\n");
}

#[test]
fn tick_contract_commonjs_nested_ticks_promises_and_timers() {
    let source = "function main(): void { console.log('sync'); process.nextTick((): void => { console.log('t1'); process.nextTick((): void => console.log('t2')); Promise.resolve().then((): void => console.log('p-in-t1')); }); Promise.resolve().then((): void => { console.log('p1'); process.nextTick((): void => console.log('t-in-p1')); }).then((): void => console.log('p2')); setTimeout((): void => console.log('timeout'), 0); }";
    assert_eq!(
        run_as_commonjs(source, "tick_commonjs_nested"),
        "sync\nt1\nt2\np1\np-in-t1\np2\nt-in-p1\ntimeout\n"
    );
}
