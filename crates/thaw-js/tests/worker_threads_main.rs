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
    // Node 22 (verified): set clones the value, get returns the stored object itself, so a mutation through
    // one get is visible to the next get.
    assert_eq!(run_loop(src), "42|9|undefined");
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

#[test]
fn in_thread_port_transfer_moves_the_port_and_its_queue() {
    let src = "const { MessageChannel } = __thaw_worker_threads_exports;
      const carrier = new MessageChannel(), inner = new MessageChannel();
      inner.port2.postMessage('q1'); inner.port2.postMessage('q2');
      carrier.port2.on('message', (m) => {
        console.log('moved', m.p === inner.port1, typeof m.p.postMessage);
        console.log('sender copy', inner.port1.postMessage('after'));
        m.p.on('message', (v) => { console.log('moved got', v); if (v === 'q2') m.p.postMessage('back'); });
        inner.port2.on('message', (v) => { console.log('peer got', v); inner.port2.close(); carrier.port1.close(); carrier.port2.close(); });
      });
      carrier.port1.postMessage({ tag: 'moved', p: inner.port1 }, [inner.port1]);";
    assert_eq!(
        run_loop(src),
        "moved false function|sender copy undefined|moved got q1|moved got q2|peer got back"
    );
}

#[test]
fn port_transfer_list_errors_and_return_values_match_node() {
    let src = "const { MessageChannel } = __thaw_worker_threads_exports;
      const err = (f) => { try { const r = f(); return 'ok ' + String(r); } catch (e) { return `${e.constructor.name} code=${e.code} ${JSON.stringify(e.message)}`; } };
      const c = new MessageChannel(), d = new MessageChannel();
      console.log('live return', c.port1.postMessage('a'));
      console.log('transfer', err(() => c.port1.postMessage(1, [d.port1])));
      console.log('again', err(() => c.port1.postMessage(1, [d.port1])));
      console.log('dup', err(() => c.port1.postMessage(1, [d.port2, d.port2])));
      console.log('own', err(() => c.port1.postMessage(1, [c.port1])));
      console.log('own options', err(() => c.port1.postMessage(1, { transfer: [c.port1] })));
      console.log('unlisted', err(() => c.port1.postMessage({ p: d.port2 })));
      const e = new MessageChannel(); e.port1.close();
      console.log('closed', err(() => c.port1.postMessage(1, [e.port1])));
      console.log('closed return', err(() => e.port1.postMessage(2)));
      console.log('bad', err(() => c.port1.postMessage(1, [{}])));
      console.log('non-iterable', err(() => c.port1.postMessage(1, 'x')));
      console.log('transferred copy', err(() => d.port1.postMessage('x')));
      console.log('clone unlisted', err(() => structuredClone(new MessageChannel().port1)));
      c.port1.close(); c.port2.close(); d.port2.close();";
    assert_eq!(
        run_loop(src),
        "live return true|transfer ok true|again DOMException code=25 \"MessagePort in transfer list is already detached\"|dup DOMException code=25 \"Transfer list contains duplicate MessagePort\"|own DOMException code=25 \"Transfer list contains source port\"|own options DOMException code=25 \"Transfer list contains source port\"|unlisted DOMException code=25 \"Object that needs transfer was found in message but not listed in transferList\"|closed DOMException code=25 \"MessagePort in transfer list is already detached\"|closed return ok undefined|bad DOMException code=25 \"Found invalid value in transferList.\"|non-iterable TypeError code=ERR_INVALID_ARG_TYPE \"Optional transferList argument must be an iterable\"|transferred copy ok undefined|clone unlisted DOMException code=25 \"Object that needs transfer was found in message but not listed in transferList\""
    );
}

#[test]
fn structured_clone_transfer_moves_a_port_to_a_new_port() {
    let src = "const { MessageChannel } = __thaw_worker_threads_exports;
      const s = new MessageChannel();
      const sc = structuredClone({ q: s.port1 }, { transfer: [s.port1] });
      console.log('clone is port', typeof sc.q.postMessage, sc.q === s.port1);
      sc.q.on('message', (v) => { console.log('through clone', v); sc.q.close(); s.port2.close(); });
      s.port2.postMessage('x');";
    assert_eq!(
        run_loop(src),
        "clone is port function false|through clone x"
    );
}
