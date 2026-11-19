use super::*;
use std::io::Write;
use std::sync::{Arc, Mutex};

/// The stdout writer of the channel: a shared buffer the test reads back.
#[derive(Clone, Default)]
struct Sink(Arc<Mutex<Vec<u8>>>);

impl Sink {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// A fresh channel on this thread that writes to `sink`; the realms installed after it share it.
fn channel(sink: &Sink) {
    crate::ipc::reset_for_test(Box::new(sink.clone()));
}

fn take_out(it: &mut Interp) -> String {
    std::mem::take(&mut it.out).join("|")
}

#[test]
fn message_reaches_every_installed_realm() {
    let sink = Sink::default();
    channel(&sink);
    let mut a = Interp::new();
    a.install_ipc();
    let mut b = Interp::new();
    b.install_ipc();
    a.run("process.on('message', m => console.log('a', m.n))")
        .unwrap();
    b.run("process.on('message', m => console.log('b', m.n))")
        .unwrap();
    crate::ipc::feed_for_test(b"__THAW_IPC__{\"n\":1}\n__THAW_IPC__CLOSE\n");
    a.run_event_loop().unwrap();
    b.run_event_loop().unwrap();
    assert_eq!(take_out(&mut a), "a 1");
    assert_eq!(take_out(&mut b), "b 1");
}

#[test]
fn message_waits_in_queue_until_first_listener() {
    let sink = Sink::default();
    channel(&sink);
    let mut a = Interp::new();
    a.install_ipc();
    // fed before any listener: the message is queued for this realm, not dropped
    crate::ipc::feed_for_test(b"__THAW_IPC__{\"n\":2}\n");
    a.run("console.log('start')").unwrap();
    a.run("process.on('message', m => console.log('late', m.n))")
        .unwrap();
    crate::ipc::feed_for_test(b"__THAW_IPC__CLOSE\n");
    a.run_event_loop().unwrap();
    assert_eq!(take_out(&mut a), "start|late 2");
}

#[test]
fn send_writes_the_prefixed_line_to_the_channel() {
    let sink = Sink::default();
    channel(&sink);
    let mut a = Interp::new();
    a.install_ipc();
    a.run("console.log(process.connected, process.send({ a: 1 }), typeof process.send)")
        .unwrap();
    a.run("process.send('x')").unwrap();
    assert_eq!(take_out(&mut a), "true true function");
    assert_eq!(sink.text(), "__THAW_IPC__{\"a\":1}\n__THAW_IPC__\"x\"\n");
}

#[test]
fn disconnect_emits_on_every_installed_realm() {
    let sink = Sink::default();
    channel(&sink);
    let mut a = Interp::new();
    a.install_ipc();
    let mut b = Interp::new();
    b.install_ipc();
    a.run("process.on('disconnect', () => console.log('a disconnect'))")
        .unwrap();
    b.run("process.on('disconnect', () => console.log('b disconnect'))")
        .unwrap();
    a.run("process.disconnect(); console.log(process.connected)")
        .unwrap();
    a.run_event_loop().unwrap();
    b.run_event_loop().unwrap();
    assert_eq!(take_out(&mut a), "false|a disconnect");
    assert_eq!(take_out(&mut b), "b disconnect");
    assert_eq!(sink.text(), "__THAW_IPC__CLOSE\n");
}

#[test]
fn no_channel_without_the_env_marker() {
    let sink = Sink::default();
    channel(&sink);
    let mut a = Interp::new();
    a.install_ipc_if_marked(None);
    a.run("console.log(typeof process.send, typeof process.disconnect, process.connected)")
        .unwrap();
    assert_eq!(take_out(&mut a), "undefined undefined undefined");
    assert_eq!(sink.text(), "");
}

/// Node 22's `process.channel` has ref/unref/refCounted/unrefCounted, and each returns undefined. The counter
/// has no hook here (the child's channel reference is the Rust-owned `ipc_live`), so only shape and no-throw are checked.
#[test]
fn channel_has_ref_counting_methods_that_return_undefined() {
    let sink = Sink::default();
    channel(&sink);
    let mut a = Interp::new();
    a.install_ipc();
    a.run("console.log(['ref', 'unref', 'refCounted', 'unrefCounted'].map(k => typeof process.channel[k]).join(','))")
        .unwrap();
    a.run("const c = process.channel; console.log(c.refCounted(), c.refCounted(), c.unrefCounted(), c.unrefCounted(), c.unrefCounted(), c.ref(), c.unref(), c.refCounted())")
        .unwrap();
    assert_eq!(
        take_out(&mut a),
        "function,function,function,function|undefined undefined undefined undefined undefined undefined undefined undefined"
    );
}
