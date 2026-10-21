// R1 option 1: Promise.resolve(thenable) defers the `then` call to the await point
// (callDynamicMethodHandleRaw), so user code between Promise.resolve and await runs first.
// Expected outputs recorded from node 22.

#[test]
fn r1o1_contract_promise_resolve_thenable_runs_then_after_sync_code() {
    let source = r##"async function main(): Promise<void> {
  const events: string[] = [];
  const thenable: any = {
    get then() {
      events.push("get");
      return function (resolve: any): void {
        events.push("call");
        resolve(9);
      };
    },
  };
  events.push("before");
  const promise = Promise.resolve(thenable);
  events.push("after");
  console.log(await promise, events.join(","));
}
"##;
    assert_eq!(
        compile_and_run(source, "r1o1_contract_promise_resolve_thenable_runs_then_after_sync_code"),
        "9 before,get,after,call\n"
    );
}
