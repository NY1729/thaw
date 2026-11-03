//! `node:worker_threads` main-thread subset on thaw-js (src/web/worker_threads.js): message channels,
//! environment data and receiveMessageOnPort. `Worker` itself is tested in worker_threads_worker.rs.
use thaw_js::Interp;

fn run_loop(src: &str) -> String {
    let mut it = Interp::new();
    it.run(src).expect("script runs");
    it.run_event_loop().expect("loop runs");
    it.out.join("|")
}

#[test]
fn worker_message_channel_round_trips_a_structured_clone() {
    let src = "const { MessageChannel } = __thaw_worker_threads_exports; const channel = new MessageChannel();
      channel.port2.on('message', (value) => { console.log(value.value); channel.port1.close(); channel.port2.close(); });
      channel.port1.postMessage({ value: 42 });";
    assert_eq!(run_loop(src), "42");
}

#[test]
fn environment_data_is_cloned_and_undefined_deletes() {
    let src = "const { setEnvironmentData, getEnvironmentData } = __thaw_worker_threads_exports;
      const value = { value: 42 }; setEnvironmentData('k', value); value.value = 7;
      const stored = getEnvironmentData('k'); console.log(stored.value);
      stored.value = 9; console.log(getEnvironmentData('k').value);
      setEnvironmentData('k', undefined); console.log(typeof getEnvironmentData('k'));";
    assert_eq!(run_loop(src), "42|42|undefined");
}

#[test]
fn receive_message_on_port_drains_the_queue_synchronously() {
    let src = "const { MessageChannel, receiveMessageOnPort } = __thaw_worker_threads_exports;
      const ch = new MessageChannel(); ch.port1.postMessage({ n: 7 }); ch.port1.postMessage('x');
      console.log(JSON.stringify(receiveMessageOnPort(ch.port2)));
      console.log(JSON.stringify(receiveMessageOnPort(ch.port2)));
      console.log(String(receiveMessageOnPort(ch.port2)));";
    assert_eq!(
        run_loop(src),
        "{\"message\":{\"n\":7}}|{\"message\":\"x\"}|undefined"
    );
}

#[test]
fn main_thread_identity_is_exposed_and_worker_runs() {
    let src = "const wt = __thaw_worker_threads_exports;
      console.log(wt.isMainThread, wt.threadId, wt.parentPort, wt.workerData);
      const worker = new wt.Worker('1', { eval: true });
      console.log(typeof worker.terminate, worker.threadId);
      worker.on('exit', (code) => console.log('exit ' + code));";
    assert_eq!(run_loop(src), "true 0 null null|function 1|exit 0");
}
