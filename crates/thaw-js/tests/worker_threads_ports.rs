//! MessagePort transfer across threads (worker_threads PR3): a port in a transfer list moves between the main thread
//! and a worker (workerData, `worker.postMessage`, `parentPort.postMessage`), both ends keep working, queued messages
//! travel with it, the sender's copy goes silent, and a peer closes when its end does. Every expectation is what
//! Node 22.23.1 prints for the same source (scratch probes under thaw-tmp/wp3-scratch/cases).
use thaw_js::Interp;

/// Runs `src` on the main thread and its event loop; returns the main thread's console output joined by `|`.
fn run_main(src: &str) -> String {
    let mut it = Interp::new();
    it.run(src).expect("main script runs");
    it.run_event_loop().expect("event loop runs");
    it.out.join("|")
}

#[test]
fn port_in_workerdata_echoes_then_both_ends_close() {
    let src = r#"const wt = __thaw_worker_threads_exports;
(async () => {
  const { port1, port2 } = new wt.MessageChannel();
  const worker = new wt.Worker(`const { port } = require('node:worker_threads').workerData;
    port.on('message', (m) => { port.postMessage('echo:' + m); if (m === 'bye') port.close(); });
    port.postMessage('hello');`, { eval: true, workerData: { port: port2 }, transferList: [port2] });
  const exited = new Promise((resolve) => worker.once('exit', resolve));
  const next = () => new Promise((resolve) => port1.once('message', resolve));
  console.log(await next());
  port1.postMessage('a');
  console.log(await next());
  const closed = new Promise((resolve) => port1.once('close', resolve));
  port1.postMessage('bye');
  console.log(await next());
  await closed;
  console.log('closed');
  console.log('exit', await exited);
})().catch((e) => console.log('failed ' + e));"#;
    assert_eq!(run_main(src), "hello|echo:a|echo:bye|closed|exit 0");
}

#[test]
fn port_posted_to_a_worker_is_answered_through_a_port_the_worker_returns() {
    let src = r#"const wt = __thaw_worker_threads_exports;
(async () => {
  const worker = new wt.Worker(`const { parentPort, MessageChannel } = require('node:worker_threads');
    parentPort.once('message', ({ p }) => {
      const channel = new MessageChannel();
      parentPort.postMessage({ tag: 'back', q: channel.port2 }, [channel.port2]);
      channel.port1.on('message', (m) => { p.postMessage('echo ' + m); p.close(); channel.port1.close(); });
    });`, { eval: true });
  const exited = new Promise((resolve) => worker.once('exit', resolve));
  const { port1, port2 } = new wt.MessageChannel();
  const back = new Promise((resolve) => worker.once('message', resolve));
  worker.postMessage({ p: port2 }, [port2]);
  const reply = await back;
  console.log(reply.tag);
  const echoed = new Promise((resolve) => port1.once('message', resolve));
  reply.q.postMessage('ping');
  console.log(await echoed);
  console.log('exit', await exited);
})().catch((e) => console.log('failed ' + e));"#;
    assert_eq!(run_main(src), "back|echo ping|exit 0");
}

#[test]
fn queued_messages_travel_with_the_transferred_port() {
    let src = r#"const wt = __thaw_worker_threads_exports;
(async () => {
  const { port1, port2 } = new wt.MessageChannel();
  port1.postMessage('q1');
  port1.postMessage({ n: 2 });
  const worker = new wt.Worker(`const { port } = require('node:worker_threads').workerData;
    let seen = 0;
    port.on('message', (m) => { port.postMessage('ack ' + JSON.stringify(m)); if (++seen === 3) port.close(); });`, { eval: true, workerData: { port: port2 }, transferList: [port2] });
  const exited = new Promise((resolve) => worker.once('exit', resolve));
  const closed = new Promise((resolve) => port1.once('close', resolve));
  const acks = [];
  port1.on('message', (m) => { acks.push(m); if (acks.length === 2) port1.postMessage('third'); });
  await closed;
  console.log(acks.join(' / '));
  console.log('exit', await exited);
})().catch((e) => console.log('failed ' + e));"#;
    assert_eq!(
        run_main(src),
        "ack \"q1\" / ack {\"n\":2} / ack \"third\"|exit 0"
    );
}

#[test]
fn sender_copy_goes_silent_and_the_peer_closes_when_the_worker_exits() {
    let src = r#"const wt = __thaw_worker_threads_exports;
(async () => {
  const { port1, port2 } = new wt.MessageChannel();
  const hi = new Promise((resolve) => port1.once('message', resolve));
  const copyClosed = new Promise((resolve) => port2.once('close', resolve));
  const peerClosed = new Promise((resolve) => port1.once('close', resolve));
  new wt.Worker(`const { port } = require('node:worker_threads').workerData; port.postMessage('hi');`, { eval: true, workerData: { port: port2 }, transferList: [port2] });
  console.log('copy', port2.postMessage('x'));
  console.log(await hi);
  await copyClosed;
  console.log('sender closed');
  await peerClosed;
  console.log('peer closed');
})().catch((e) => console.log('failed ' + e));"#;
    assert_eq!(run_main(src), "copy undefined|hi|sender closed|peer closed");
}

#[test]
fn unclosed_remote_port_hangs_up_when_its_worker_exits() {
    // the worker ends without closing its port: the main end hears 'close', and the loop ends (no listener left)
    let src = r#"const wt = __thaw_worker_threads_exports;
(async () => {
  const { port1, port2 } = new wt.MessageChannel();
  new wt.Worker(`const { port } = require('node:worker_threads').workerData; port.postMessage('one');`, { eval: true, workerData: { port: port2 }, transferList: [port2] });
  port1.on('message', (m) => console.log('got', m));
  port1.on('close', () => console.log('close event'));
})().catch((e) => console.log('failed ' + e));"#;
    assert_eq!(run_main(src), "got one|close event");
}

#[test]
fn a_message_listener_on_a_worker_port_keeps_the_worker_alive_until_the_port_closes() {
    let src = r#"const wt = __thaw_worker_threads_exports;
(async () => {
  const { port1, port2 } = new wt.MessageChannel();
  const worker = new wt.Worker(`const { port } = require('node:worker_threads').workerData; port.on('message', (m) => { port.postMessage('got ' + m); });`, { eval: true, workerData: { port: port2 }, transferList: [port2] });
  const exited = new Promise((resolve) => worker.once('exit', resolve));
  const reply = new Promise((resolve) => port1.once('message', resolve));
  port1.postMessage('x');
  console.log(await reply);
  port1.close();
  console.log('exit', await exited);
})().catch((e) => console.log('failed ' + e));"#;
    assert_eq!(run_main(src), "got x|exit 0");
}

#[test]
fn transfer_list_port_errors_match_node() {
    let src = r#"const wt = __thaw_worker_threads_exports;
(async () => {
  const err = (f) => { try { f(); return 'ok'; } catch (e) { return `${e.name} ${e.code} ${e.message}`; } };
  const worker = new wt.Worker('setInterval(() => {}, 1000)', { eval: true });
  const { port1, port2 } = new wt.MessageChannel();
  console.log('dup', err(() => worker.postMessage({}, [port2, port2])));
  console.log('unlisted', err(() => worker.postMessage({ p: port2 })));
  const c = new wt.MessageChannel();
  console.log('transfer', err(() => worker.postMessage({}, [c.port1])));
  console.log('again', err(() => worker.postMessage({}, [c.port1])));
  c.port2.close();
  console.log('closed', err(() => worker.postMessage({}, [c.port2])));
  console.log('own', err(() => port1.postMessage('x', [port1])));
  await worker.terminate();
  console.log('terminated');
})().catch((e) => console.log('failed ' + e));"#;
    assert_eq!(
        run_main(src),
        "dup DataCloneError 25 Transfer list contains duplicate MessagePort\
|unlisted DataCloneError 25 Object that needs transfer was found in message but not listed in transferList\
|transfer ok\
|again DataCloneError 25 MessagePort in transfer list is already detached\
|closed DataCloneError 25 MessagePort in transfer list is already detached\
|own DataCloneError 25 Transfer list contains source port\
|terminated"
    );
}

#[test]
fn transferred_or_closed_port_in_a_value_is_uncloneable() {
    let src = r#"const wt = __thaw_worker_threads_exports;
(async () => {
  const err = (f) => { try { f(); return 'ok'; } catch (e) { return `${e.name} ${e.code} ${e.message}`; } };
  const worker = new wt.Worker('setInterval(() => {}, 1000)', { eval: true });
  const c = new wt.MessageChannel();
  console.log('transfer', err(() => worker.postMessage({}, [c.port1])));
  console.log('transferred-in-value', err(() => worker.postMessage({ p: c.port1 })));
  const d = new wt.MessageChannel();
  d.port1.close();
  console.log('closed-in-value', err(() => worker.postMessage({ p: d.port1 })));
  console.log('closed-in-value-listed', err(() => worker.postMessage({ p: d.port1 }, [d.port1])));
  await worker.terminate();
  console.log('done');
})().catch((e) => console.log('failed ' + e));"#;
    assert_eq!(
        run_main(src),
        "transfer ok\
|transferred-in-value DataCloneError 25 Cannot clone object of unsupported type.\
|closed-in-value DataCloneError 25 Cannot clone object of unsupported type.\
|closed-in-value-listed DataCloneError 25 MessagePort in transfer list is already detached\
|done"
    );
}

#[test]
fn a_port_a_worker_sends_to_main_moves_on_to_another_worker_through_the_hub() {
    // worker A's port end reaches main, which forwards it to worker B; B's send then arrives in A's own end
    let src = r#"const wt = __thaw_worker_threads_exports;
(async () => {
  const a = new wt.Worker(`
    const { parentPort, MessageChannel } = require('node:worker_threads');
    const ch = new MessageChannel();
    parentPort.postMessage({ p: ch.port2 }, [ch.port2]);
    ch.port1.on('message', (m) => parentPort.postMessage('A got ' + m));
  `, { eval: true });
  const b = new wt.Worker(`
    const { parentPort } = require('node:worker_threads');
    parentPort.once('message', ({ p }) => p.postMessage('from B'));
  `, { eval: true });
  const m = await new Promise((resolve) => { a.once('message', resolve); });
  const got = new Promise((resolve) => { a.once('message', resolve); });
  b.postMessage(m, [m.p]);
  console.log(await got);
  await a.terminate();
  await b.terminate();
  console.log('done');
})().catch((e) => console.log('failed ' + e));"#;
    assert_eq!(run_main(src), "A got from B|done");
}
