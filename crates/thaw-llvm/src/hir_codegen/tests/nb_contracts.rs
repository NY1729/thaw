// Wave NB: timer checkpoints, generator throw identity, exception identity through any callbacks.
// Expected output from node 22.

#[test]
fn nb_contract_each_due_timer_gets_a_microtask_checkpoint() {
    let source = r##"async function main(): Promise<void> {
  const events: string[] = [];
  setTimeout(() => {
    events.push("timer-1");
    Promise.resolve().then(() => { events.push("promise"); });
  }, 0);
  setTimeout(() => { events.push("timer-2"); }, 0);
  await new Promise<void>((resolve) => setTimeout(resolve, 20));
  console.log(events.join(","));
}
"##;
    assert_eq!(compile_and_run(source, "nb_contract_each_due_timer_gets_a_microtask_checkpoint"), "timer-1,promise,timer-2\n");
}

#[test]
fn nb_contract_exceptions_thrown_through_any_callbacks_keep_identity() {
    let source = r##"function main(): void {
  const marker: any = { value: 1 };
  const invoke: any = (callback: any): any => callback();
  try {
    invoke(() => { throw marker; });
  } catch (error: any) {
    const e: any = error;
    console.log(e === marker, e.value);
  }
  try {
    invoke(() => { throw 7; });
  } catch (error: any) {
    const e: any = error;
    console.log(typeof e, e === 7);
  }
  try {
    invoke(() => { throw "s"; });
  } catch (error: any) {
    const e: any = error;
    console.log(typeof e, e === "s");
  }
}
"##;
    assert_eq!(compile_and_run(source, "nb_contract_exceptions_thrown_through_any_callbacks_keep_identity"), "true 1\nnumber true\nstring true\n");
}
