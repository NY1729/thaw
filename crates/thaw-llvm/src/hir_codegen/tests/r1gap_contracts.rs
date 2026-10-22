// Awaiting an already-settled promise must run the microtasks queued before it,
// as node does. Expected outputs recorded from node 22.

#[test]
fn r1gap_contract_await_settled_promise_runs_pending_microtasks_first() {
    let source = r##"async function main(): Promise<void> {
  const events: string[] = [];
  Promise.resolve().then(() => {
    events.push("micro");
  });
  events.push("sync");
  await Promise.resolve();
  events.push("after");
  console.log(events.join(","));
}
"##;
    assert_eq!(
        compile_and_run(
            source,
            "r1gap_contract_await_settled_promise_runs_pending_microtasks_first"
        ),
        "sync,micro,after\n"
    );
}

#[test]
fn r1gap_contract_await_settled_promise_runs_deferred_thenable_first() {
    let source = r##"async function main(): Promise<void> {
  const events: string[] = [];
  const thenable: any = {
    then(resolve: any): void {
      events.push("then");
      resolve(1);
    },
  };
  Promise.resolve().then(() => {
    events.push("micro");
  });
  Promise.resolve(thenable);
  events.push("sync");
  await Promise.resolve();
  events.push("after");
  console.log(events.join(","));
}
"##;
    assert_eq!(
        compile_and_run(
            source,
            "r1gap_contract_await_settled_promise_runs_deferred_thenable_first"
        ),
        "sync,micro,then,after\n"
    );
}

#[test]
fn r1gap_contract_await_pending_thenable_promise_after_settled_await() {
    let source = r##"async function main(): Promise<void> {
  const events: string[] = [];
  const thenable: any = {
    get then() {
      events.push("get");
      return function (resolve: any): void {
        events.push("then");
        resolve(1);
      };
    },
  };
  Promise.resolve().then(() => {
    events.push("micro");
  });
  const promise = Promise.resolve(thenable);
  events.push("before");
  await promise;
  events.push("after");
  console.log(events.join(","));
}
"##;
    assert_eq!(
        compile_and_run(
            source,
            "r1gap_contract_await_pending_thenable_promise_after_settled_await"
        ),
        "get,before,micro,then,after\n"
    );
}

#[test]
fn r1gap_contract_await_settled_promise_runs_queued_thenable_before_later_microtask() {
    let source = r##"async function main(): Promise<void> {
  const events: string[] = [];
  const thenable: any = {
    then(resolve: any): void {
      events.push("then");
      resolve(1);
    },
  };
  Promise.resolve(thenable);
  Promise.resolve().then(() => {
    events.push("micro");
  });
  events.push("sync");
  await Promise.resolve();
  events.push("after");
  console.log(events.join(","));
}
"##;
    assert_eq!(
        compile_and_run(
            source,
            "r1gap_contract_await_settled_promise_runs_queued_thenable_before_later_microtask"
        ),
        "sync,then,micro,after\n"
    );
}

#[test]
fn r1gap_contract_job_barrier_leaves_global_object_unchanged() {
    let source = r##"async function main(): Promise<void> {
  const before = Object.keys(globalThis).join(",");
  const thenable: any = {
    then(resolve: any): void {
      resolve(1);
    },
  };
  Promise.resolve().then(() => {});
  Promise.resolve(thenable);
  await Promise.resolve();
  console.log(before === Object.keys(globalThis).join(","), "__thaw_queue_job_barrier" in globalThis);
}
"##;
    assert_eq!(
        compile_and_run(
            source,
            "r1gap_contract_job_barrier_leaves_global_object_unchanged"
        ),
        "true false\n"
    );
}
