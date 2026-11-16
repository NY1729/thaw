//! The process-level IPC channel of a cluster worker. The wire is child_process's: a stdin line starting with
//! `__THAW_IPC__` is a message from the primary, a stdout line with the prefix is a message to it, and
//! `__THAW_IPC__CLOSE` is a disconnect. One host-wide queue holds the events; each realm that installed the
//! channel gets its own copy and emits them on its own `process` (web/ipc_child.js), so the realm the user
//! listens on and the realm the cluster bundle listens on both see every message.
//! ponytail: fd 0 belongs to the channel for the whole process, so process.stdin lines are not forwarded to
//! it; the channel is ref'd only while a realm listens for `message` (or a disconnect is queued). Upgrade =
//! forward non-prefixed lines to the stdin reader and ref the channel on a `process` listener-count hook.

use crate::{arg, Interp, Value, R};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::Write;
use std::sync::Mutex;

/// The env var cluster sets on every worker it forks (registry builtins/diagnostics.rs, `createWorkerProcess`).
pub(crate) const MARKER: &str = "NODE_UNIQUE_ID";
const PREFIX: &str = "__THAW_IPC__";
const IPC_CHILD: &str = include_str!("web/ipc_child.js");

#[derive(Clone, Debug, PartialEq)]
enum Event {
    Message(String),
    Disconnect,
}

struct Channel {
    /// Realm id and the events queued for it, in arrival order.
    realms: Vec<(usize, VecDeque<Event>)>,
    /// Stdin bytes after the last newline.
    partial: Vec<u8>,
    connected: bool,
    /// The stdout writer; one lock covers each frame.
    out: Mutex<Box<dyn Write + Send>>,
}

impl Channel {
    fn new(out: Box<dyn Write + Send>) -> Channel {
        Channel {
            realms: Vec::new(),
            partial: Vec::new(),
            connected: true,
            out: Mutex::new(out),
        }
    }

    fn fan_out(&mut self, event: Event) {
        for (_, queue) in &mut self.realms {
            queue.push_back(event.clone());
        }
    }

    /// The peer closed the channel (EOF or a CLOSE line): queue the disconnect once.
    fn close(&mut self) {
        if self.connected {
            self.connected = false;
            self.fan_out(Event::Disconnect);
        }
    }

    fn feed(&mut self, bytes: &[u8]) {
        self.partial.extend_from_slice(bytes);
        while let Some(end) = self.partial.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.partial.drain(..=end).collect();
            let Some(rest) = line[..line.len() - 1].strip_prefix(PREFIX.as_bytes()) else {
                continue;
            };
            if rest == b"CLOSE" {
                self.close();
            } else if self.connected {
                let message = String::from_utf8_lossy(rest).into_owned();
                self.fan_out(Event::Message(message));
            }
        }
    }

    fn write_frame(&self, body: &str) -> bool {
        let mut out = self.out.lock().unwrap_or_else(|e| e.into_inner());
        let written = (|| -> std::io::Result<()> {
            out.write_all(PREFIX.as_bytes())?;
            out.write_all(body.as_bytes())?;
            out.write_all(b"\n")?;
            out.flush()
        })();
        written.is_ok()
    }
}

thread_local! {
    static CHANNEL: RefCell<Option<Channel>> = const { RefCell::new(None) };
}

/// The channel of this thread, created on first use with stdout as its writer.
fn with<T>(f: impl FnOnce(&mut Channel) -> T) -> T {
    CHANNEL.with(|slot| {
        let mut slot = slot.borrow_mut();
        f(slot.get_or_insert_with(|| Channel::new(Box::new(std::io::stdout()))))
    })
}

/// The channel if one exists; `default` otherwise (a realm that never installed it must not create it).
/// The channel if one exists; `default` otherwise. `try_with`: a realm dropped while thread-locals are torn
/// down (the thread exits) finds nothing to do, and a realm that never installed the channel must not create it.
fn existing<T>(default: T, f: impl FnOnce(&mut Channel) -> T) -> T {
    CHANNEL
        .try_with(|slot| {
            slot.try_borrow_mut()
                .ok()
                .and_then(|mut slot| slot.as_mut().map(f))
        })
        .ok()
        .flatten()
        .unwrap_or(default)
}

fn register_realm(id: usize) {
    with(|ch| {
        if !ch.realms.iter().any(|(r, _)| *r == id) {
            ch.realms.push((id, VecDeque::new()));
        }
    });
}

pub(crate) fn forget_realm(id: usize) {
    existing((), |ch| ch.realms.retain(|(r, _)| *r != id));
}

fn has_realm(id: usize) -> bool {
    existing(false, |ch| ch.realms.iter().any(|(r, _)| *r == id))
}

/// Runs `f` on realm `id`'s queue; `None` when the realm has no queue.
fn with_queue<T>(id: usize, f: impl FnOnce(&mut VecDeque<Event>) -> Option<T>) -> Option<T> {
    existing(None, |ch| {
        match ch.realms.iter_mut().find(|(r, _)| *r == id) {
            Some((_, queue)) => f(queue),
            None => None,
        }
    })
}

/// Whether realm `id` has channel work its loop must wait for: a queued disconnect, a queued message while it
/// listens, or a connected channel it listens on. Without a listener the channel does not keep the loop alive.
fn keeps_alive(id: usize, listening: bool) -> bool {
    existing(false, |ch| {
        let connected = ch.connected;
        match ch.realms.iter().find(|(r, _)| *r == id) {
            Some((_, queue)) => match queue.front() {
                Some(Event::Disconnect) => true,
                Some(Event::Message(_)) => listening,
                None => connected && listening,
            },
            None => false,
        }
    })
}

/// Reads what stdin delivered since the last poll into the channel; the end of stdin closes it.
fn read_input() {
    let (bytes, ended) = crate::sysio::drain_stdin();
    if bytes.is_empty() && !ended {
        return;
    }
    with(|ch| {
        ch.feed(&bytes);
        if ended {
            ch.close();
        }
    });
}

#[cfg(test)]
pub(crate) fn reset_for_test(out: Box<dyn Write + Send>) {
    CHANNEL.with(|slot| *slot.borrow_mut() = Some(Channel::new(out)));
}

#[cfg(test)]
pub(crate) fn feed_for_test(bytes: &[u8]) {
    with(|ch| ch.feed(bytes));
}

impl Interp {
    /// Installs the channel on this realm's `process` when the env marker is set (a cluster worker): starts the
    /// host's one stdin reader, then installs as `install_ipc` does. Without the marker nothing changes.
    pub(crate) fn install_ipc_if_marked(&mut self, marker: Option<&std::ffi::OsStr>) {
        if marker.is_none() {
            return;
        }
        crate::sysio::start_stdin_fd(0);
        self.install_ipc();
    }

    /// Defines `process.send`, `connected`, `disconnect` and the event hooks, and registers this realm for
    /// the channel's events.
    pub(crate) fn install_ipc(&mut self) {
        register_realm(self.realm_id);
        let g = self.global.clone();
        let f = self.native(|it, _, a| {
            it.flush_partial();
            let body = it.to_str(&arg(a, 0)).to_string();
            Ok(Value::Bool(with(|ch| ch.write_frame(&body))))
        });
        g.declare("__thaw_ipc_send", f, true);
        let f = self.native(|_, _, _| {
            with(|ch| {
                if ch.connected {
                    ch.write_frame("CLOSE");
                    ch.close();
                }
            });
            Ok(Value::Undefined)
        });
        g.declare("__thaw_ipc_disconnect", f, true);
        self.run(IPC_CHILD)
            .unwrap_or_else(|_| panic!("thaw-js ipc channel must install"));
    }

    /// True while this realm's loop must keep running for the channel (see `keeps_alive`).
    pub(crate) fn ipc_live(&self) -> bool {
        keeps_alive(self.realm_id, self.listener_count("message") > 0)
    }

    /// Reads stdin into the channel, then hands this realm its queued events. A message waits in the queue
    /// until this realm listens for `message`.
    pub(crate) fn poll_ipc(&mut self) -> R<()> {
        if !has_realm(self.realm_id) {
            return Ok(());
        }
        read_input();
        let g = Value::Object(self.global_object.clone());
        while let Some(event) = with_queue(self.realm_id, |q| q.pop_front()) {
            match event {
                Event::Message(line) => {
                    if self.listener_count("message") == 0 {
                        with_queue(self.realm_id, |q| {
                            q.push_front(Event::Message(line));
                            Some(())
                        });
                        break;
                    }
                    let f = self.get(&g, "__thaw_ipc_message")?;
                    if matches!(f, Value::Object(_)) {
                        self.call(&f, &g, &[Value::str(&line)])?;
                    }
                }
                Event::Disconnect => {
                    let f = self.get(&g, "__thaw_ipc_disconnected")?;
                    if matches!(f, Value::Object(_)) {
                        self.call(&f, &g, &[])?;
                    }
                }
            }
            self.run_jobs()?;
        }
        Ok(())
    }
}
