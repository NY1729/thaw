//! Worker threads, W2b: the worker-side bootstrap (parentPort, workerData, keep-alive, env data, exit codes) and the
//! terminate interrupt. The main script drives everything through the `__thaw_worker_*` natives; its
//! `__thaw_worker_dispatch` logs each event, printed at the worker's exit.
use thaw_js::Interp;

/// Spawns `source` (a worker with `opts`, a JS expression giving the JSON string) and runs the main script.
/// `after_spawn` runs with the handle as `h`; `on_event` runs for each event with the event as `e`.
fn run_worker(source: &str, opts: &str, after_spawn: &str, on_event: &str) -> String {
    let main = format!(
        r#"const seen = [];
        const h = __thaw_worker_spawn({source:?}, {opts});
        globalThis.__thaw_worker_dispatch = (json) => {{
          for (const e of JSON.parse(json)) {{
            if (e.type === 'exit') seen.push('exit:' + e.code);
            else if (e.type === 'error') seen.push('error:' + __thaw_worker_decode(e.value).message);
            else if (e.type === 'message') seen.push('message:' + JSON.stringify(__thaw_worker_decode(e.value)));
            else seen.push(e.type);
            {on_event}
            if (e.type === 'exit') console.log(seen.join('|'));
          }}
        }};
        {after_spawn}"#
    );
    let mut it = Interp::new();
    it.run(&main).expect("main script runs");
    it.run_event_loop().expect("event loop runs");
    it.out.join("|")
}

const NO_OPTS: &str = "'{}'";
const NOTHING: &str = "";

/// Terminates the worker as soon as it is online (its busy loop has not started yet at that point, or is just starting).
const TERMINATE_ONLINE: &str =
    "if (e.type === 'online') setTimeout(() => __thaw_worker_terminate(h), 50);";

#[test]
fn worker_posts_a_value_through_parent_port() {
    let src = "const { parentPort } = require('node:worker_threads'); parentPort.postMessage(42);";
    assert_eq!(
        run_worker(src, NO_OPTS, NOTHING, NOTHING),
        "online|message:42|exit:0"
    );
}

#[test]
fn worker_data_arrives_decoded_from_the_spawn_options() {
    let src = "const { workerData, parentPort } = require('node:worker_threads'); parentPort.postMessage(workerData.n * 2);";
    let opts = "JSON.stringify({ workerData: __thaw_worker_encode({ n: 21 }) })";
    assert_eq!(
        run_worker(src, opts, NOTHING, NOTHING),
        "online|message:42|exit:0"
    );
}

#[test]
fn environment_data_is_inherited_from_the_spawn_options() {
    let src = "const { getEnvironmentData, parentPort } = require('node:worker_threads'); parentPort.postMessage(getEnvironmentData('k'));";
    let opts = "JSON.stringify({ envData: __thaw_worker_encode(new Map([['k', 7]])) })";
    assert_eq!(
        run_worker(src, opts, NOTHING, NOTHING),
        "online|message:7|exit:0"
    );
}

#[test]
fn parent_port_message_listener_echoes_and_keeps_the_worker_alive() {
    let src = "const { parentPort } = require('node:worker_threads'); parentPort.on('message', (m) => parentPort.postMessage(m));";
    assert_eq!(
        run_worker(
            src,
            NO_OPTS,
            "__thaw_worker_post(h, __thaw_worker_encode({ a: 1 }));",
            "if (e.type === 'message') __thaw_worker_terminate(h);",
        ),
        r#"online|message:{"a":1}|exit:1"#
    );
}

#[test]
fn onmessage_setter_keeps_the_worker_alive() {
    let src = "const { parentPort } = require('node:worker_threads'); parentPort.onmessage = (e) => parentPort.postMessage(e.data + 1);";
    assert_eq!(
        run_worker(
            src,
            NO_OPTS,
            "__thaw_worker_post(h, __thaw_worker_encode(1));",
            "if (e.type === 'message') __thaw_worker_terminate(h);",
        ),
        "online|message:2|exit:1"
    );
}

#[test]
fn unref_on_parent_port_lets_the_worker_exit() {
    let src = "const { parentPort } = require('node:worker_threads'); parentPort.on('message', () => {}); parentPort.unref();";
    assert_eq!(run_worker(src, NO_OPTS, NOTHING, NOTHING), "online|exit:0");
}

#[test]
fn closed_parent_port_is_released_but_a_timer_still_keeps_the_worker_alive() {
    let src = "const { parentPort } = require('node:worker_threads'); parentPort.on('message', () => {}); parentPort.close();
      setTimeout(() => __thaw_worker_send(__thaw_worker_encode('late')), 30);";
    assert_eq!(
        run_worker(src, NO_OPTS, NOTHING, NOTHING),
        r#"online|message:"late"|exit:0"#
    );
}

#[test]
fn receive_message_on_port_polls_the_parent_port_queue() {
    let src = "const { parentPort, receiveMessageOnPort } = require('node:worker_threads');
      const t = setInterval(() => {
        const r = receiveMessageOnPort(parentPort);
        if (r) { parentPort.postMessage(r.message * 10); clearInterval(t); }
      }, 5);";
    assert_eq!(
        run_worker(
            src,
            NO_OPTS,
            "__thaw_worker_post(h, __thaw_worker_encode(4));",
            NOTHING
        ),
        "online|message:40|exit:0"
    );
}

#[test]
fn uncaught_error_in_a_message_handler_is_reported_then_exit_one() {
    let src = "const { parentPort } = require('node:worker_threads'); parentPort.on('message', () => { throw new Error('handler boom'); });";
    assert_eq!(
        run_worker(
            src,
            NO_OPTS,
            "__thaw_worker_post(h, __thaw_worker_encode(1));",
            NOTHING
        ),
        "online|error:handler boom|exit:1"
    );
}

// Interrupt: each loop form below spins until its own deadline (a guard so a missing interrupt fails the test with
// exit 0 instead of hanging). terminate() is sent as soon as the worker is online, so the loop must stop with exit 1.

#[test]
fn terminate_interrupts_a_while_loop() {
    let src = "const end = Date.now() + 5000; while (true) { if (Date.now() > end) break; }";
    assert_eq!(
        run_worker(src, NO_OPTS, NOTHING, TERMINATE_ONLINE),
        "online|exit:1"
    );
}

#[test]
fn terminate_interrupts_a_for_loop() {
    let src = "const end = Date.now() + 5000; for (;;) { if (Date.now() > end) break; }";
    assert_eq!(
        run_worker(src, NO_OPTS, NOTHING, TERMINATE_ONLINE),
        "online|exit:1"
    );
}

#[test]
fn terminate_interrupts_a_do_while_loop() {
    let src = "const end = Date.now() + 5000; do { if (Date.now() > end) break; } while (true);";
    assert_eq!(
        run_worker(src, NO_OPTS, NOTHING, TERMINATE_ONLINE),
        "online|exit:1"
    );
}

#[test]
fn terminate_interrupts_a_for_of_loop_over_a_user_iterator() {
    let src = "const end = Date.now() + 5000;
      const endless = { [Symbol.iterator]() { return { next() { return { done: Date.now() > end, value: 1 }; } }; } };
      for (const x of endless) {}";
    assert_eq!(
        run_worker(src, NO_OPTS, NOTHING, TERMINATE_ONLINE),
        "online|exit:1"
    );
}

#[test]
fn terminate_interrupts_a_callback_driven_loop() {
    let src = "const end = Date.now() + 5000; function again() { if (Date.now() > end) return; setImmediate(again); } again();";
    assert_eq!(
        run_worker(src, NO_OPTS, NOTHING, TERMINATE_ONLINE),
        "online|exit:1"
    );
}

// Known limitation, not asserted: a `while (true)` inside a generator body runs on a generator's own OS thread
// (eventloop.rs generators), so the interrupt is only observed at the generator's yields, not inside its body.
