//! Coroutine seam: the only way the interpreter talks to a coroutine.
//!
//! Async function bodies and generator bodies run as coroutines. The evaluator is a recursive
//! Rust tree-walker, so a suspended body keeps its native stack; a backend supplies that stack
//! and the hand-off. The interpreter calls only the functions below: `new_pair` and `start` to
//! create one, `Link::send` / `Link::recv` on the host side (resume, wait for the next yield), and
//! `Inside::suspend` from inside the body (yield or await). Each call takes the realm key (the
//! `Interp` address) that names the host-thread state moved across the hand-off.
//!
//! Enum dispatch rather than a trait: the backend is a concrete type of `Co`, so the hot path has no
//! vtable and no generic parameter on `Interp`. Two backends: the thread backend (co_thread.rs) and
//! the costack backend (co_stack.rs). `THAW_JS_CO=stack|thread` picks one explicitly; unset, the costack
//! backend is used wherever it is supported (Linux x86_64 / aarch64) and the thread backend elsewhere
//! (see `choose_backend`).
//!
//! The costack backend holds at most `costack::MAX_LIVE` (30,000) coroutines alive at once, because each
//! stack is two kernel mappings and `vm.max_map_count` (65530 by default) bounds them. Creating one more
//! throws `RangeError: cannot start coroutine (thread limit)`. The thread backend has no such cap, so
//! `THAW_JS_CO=thread` is the way out for a program that needs more pending coroutines than that.

use std::sync::OnceLock;

use crate::*;

use crate::co_stack as stack;
use crate::co_thread as thread;

pub(crate) use crate::co_thread::STACK_BYTES;

/// Which transport a new coroutine uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Backend {
    Thread,
    Stack,
}

/// The costack backend exists only where `costack.rs` has its assembly (Linux x86_64 / aarch64).
const STACK_SUPPORTED: bool = cfg!(all(
    unix,
    any(target_arch = "x86_64", target_arch = "aarch64"),
    target_os = "linux"
));

/// The backend for a new coroutine, from `THAW_JS_CO` (read once). Native addons need no special case:
/// the costack backend keeps addon calls on the thread that loaded them.
pub(crate) fn backend() -> Backend {
    static ENV: OnceLock<Option<String>> = OnceLock::new();
    let env = ENV.get_or_init(|| {
        let value = std::env::var("THAW_JS_CO").ok();
        match value.as_deref() {
            Some("stack") if !STACK_SUPPORTED => eprintln!(
                "thaw-js: THAW_JS_CO=stack is not supported on this target; using the thread backend"
            ),
            Some(other) if !matches!(other, "" | "stack" | "thread") => {
                eprintln!("thaw-js: unknown THAW_JS_CO={other:?}; using the thread backend")
            }
            _ => {}
        }
        value
    });
    choose_backend(env.as_deref(), STACK_SUPPORTED)
}

/// The selection rule, pure so the table test can cover every input. `thread` always wins, and so does
/// any unknown value. `stack`, unset and empty pick the costack backend where it is supported, else the
/// thread backend.
pub(crate) fn choose_backend(env: Option<&str>, stack_supported: bool) -> Backend {
    match env {
        Some("thread") => Backend::Thread,
        Some(other) if !matches!(other, "" | "stack") => Backend::Thread,
        _ if stack_supported => Backend::Stack,
        _ => Backend::Thread,
    }
}

/// What the host sends into a suspended coroutine.
pub(crate) enum Resume {
    Value(Value),
    Throw(Value),
    /// `generator.return(v)`: unwinds the generator body (running `finally` blocks).
    Return(Value),
}

/// What a coroutine hands back to the host.
pub(crate) enum Yield {
    Await(Value),
    /// `yield v` inside a generator body.
    Yielded(Value),
    Done(R<Value>),
}

/// The body of a coroutine: runs once, on the coroutine's own stack, and returns its completion.
pub(crate) type Body = Box<dyn FnOnce() -> R<Value>>;

/// Host side of one coroutine.
pub(crate) enum Link {
    Thread(thread::Link),
    Stack(stack::Link),
}

/// Coroutine side of one coroutine. Held by the `CoCtx` the interpreter keeps on `cur_co`.
pub(crate) enum Inside {
    Thread(thread::Inside),
    Stack(stack::Inside),
}

/// A connected host/coroutine pair on the process backend. Nothing runs until `start`.
pub(crate) fn new_pair() -> (Link, Inside) {
    new_pair_on(backend())
}

/// A connected pair on `backend`. The seam tests use it to run both backends in one process.
pub(crate) fn new_pair_on(backend: Backend) -> (Link, Inside) {
    match backend {
        Backend::Thread => {
            let (link, inside) = thread::new_pair();
            (Link::Thread(link), Inside::Thread(inside))
        }
        Backend::Stack => {
            let (link, inside) = stack::new_pair();
            (Link::Stack(link), Inside::Stack(inside))
        }
    }
}

/// Starts the coroutine of `link`/`inside` running `body`; it runs until its first hand-back. `realm`
/// is the Interp key (the thread backend moves its host tables with it; the costack backend ignores
/// it). False when the backend could not start one (the body is dropped unrun).
pub(crate) fn start(link: &Link, inside: &Inside, realm: usize, body: Body) -> bool {
    match (link, inside) {
        (Link::Thread(_), Inside::Thread(i)) => thread::start(i, realm, body),
        (Link::Stack(l), Inside::Stack(_)) => stack::start(l, body),
        _ => unreachable!("coroutine link and inside from different backends"),
    }
}

impl Link {
    /// Hands control to the coroutine with `msg`. It runs until it hands control back.
    pub(crate) fn send(&self, realm: usize, msg: Resume) {
        match self {
            Link::Thread(l) => l.send(realm, msg),
            Link::Stack(l) => l.send(msg),
        }
    }

    /// Waits until the coroutine hands control back. `None` if it vanished without doing so.
    pub(crate) fn recv(&self, realm: usize) -> Option<Yield> {
        match self {
            Link::Thread(l) => l.recv(realm),
            Link::Stack(l) => l.recv(),
        }
    }
}

impl Inside {
    /// Hands `msg` to the host and waits to be resumed. Called from inside the body only.
    /// Never returns when the host has dropped this coroutine: the backend parks it for good.
    pub(crate) fn suspend(&self, realm: usize, msg: Yield) -> Resume {
        match self {
            Inside::Thread(i) => i.suspend(realm, msg),
            Inside::Stack(i) => i.suspend(msg),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::sync::atomic::Ordering;

    /// Every backend that exists on this target: the seam tests run each scenario on each of them.
    fn backends() -> Vec<Backend> {
        let mut all = vec![Backend::Thread];
        if STACK_SUPPORTED {
            all.push(Backend::Stack);
        }
        all
    }

    /// `co_stack::start_small` on a seam link (test only; the live-limit test needs the small stacks).
    #[cfg(all(
        test,
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    ))]
    fn start_small_stack(link: &Link, body: Body) -> bool {
        match link {
            Link::Stack(l) => stack::start_small(l, body, 256 << 10),
            Link::Thread(_) => unreachable!("small stacks are a costack-backend test"),
        }
    }

    fn num(v: &Value) -> f64 {
        match v {
            Value::Number(n) => *n,
            _ => panic!("number expected"),
        }
    }

    #[test]
    fn backend_choice_table() {
        for supported in [false, true] {
            let stack = if supported {
                Backend::Stack
            } else {
                Backend::Thread
            };
            let cases: [(Option<&str>, Backend); 5] = [
                (Some("stack"), stack),
                (Some("thread"), Backend::Thread),
                (Some("bogus"), Backend::Thread),
                (None, stack),
                (Some(""), stack),
            ];
            for (env, want) in cases {
                assert_eq!(
                    choose_backend(env, supported),
                    want,
                    "env={env:?} supported={supported}"
                );
            }
        }
    }

    /// The default (env unset) is the costack backend on a supported target; `THAW_JS_CO=thread` is the
    /// escape hatch. Uses the pure rule, so it does not depend on the process env.
    #[test]
    fn unset_selects_stack_where_supported_and_thread_is_explicit() {
        assert_eq!(choose_backend(None, true), Backend::Stack);
        assert_eq!(choose_backend(Some("thread"), true), Backend::Thread);
        assert_eq!(choose_backend(None, false), Backend::Thread);
    }

    #[test]
    fn resume_yield_finish_pass_messages_both_ways() {
        for b in backends() {
            let realm = 0x5100usize;
            let (link, inside) = new_pair_on(b);
            let inside = Rc::new(inside);
            let body_side = inside.clone();
            let body: Body = Box::new(move || {
                let Resume::Value(v) = body_side.suspend(realm, Yield::Yielded(Value::Number(1.0)))
                else {
                    panic!("value resume expected");
                };
                Ok(Value::Number(num(&v) + 10.0))
            });
            assert!(start(&link, &inside, realm, body), "{b:?}");
            let Some(Yield::Yielded(v)) = link.recv(realm) else {
                panic!("{b:?}: yield expected");
            };
            assert_eq!(num(&v), 1.0);
            link.send(realm, Resume::Value(Value::Number(5.0)));
            let Some(Yield::Done(Ok(v))) = link.recv(realm) else {
                panic!("{b:?}: completion expected");
            };
            assert_eq!(num(&v), 15.0);
        }
    }

    #[test]
    fn nested_coroutine_resumes_another() {
        for b in backends() {
            let (realm_outer, realm_inner) = (0x5200usize, 0x5201usize);
            let (outer_link, outer_inside) = new_pair_on(b);
            let outer_inside = Rc::new(outer_inside);
            let outer_side = outer_inside.clone();
            let outer: Body = Box::new(move || {
                let (inner_link, inner_inside) = new_pair_on(b);
                let inner_inside = Rc::new(inner_inside);
                let inner_side = inner_inside.clone();
                let inner: Body = Box::new(move || {
                    let Resume::Value(v) =
                        inner_side.suspend(realm_inner, Yield::Yielded(Value::Number(2.0)))
                    else {
                        panic!("value resume expected");
                    };
                    Ok(Value::Number(num(&v) * 3.0))
                });
                assert!(start(&inner_link, &inner_inside, realm_inner, inner));
                let Some(Yield::Yielded(v)) = inner_link.recv(realm_inner) else {
                    panic!("inner yield expected");
                };
                // the outer body forwards the inner yield to its own host, then feeds the answer back
                let Resume::Value(r) = outer_side.suspend(realm_outer, Yield::Yielded(v)) else {
                    panic!("value resume expected");
                };
                inner_link.send(realm_inner, Resume::Value(r));
                let Some(Yield::Done(done)) = inner_link.recv(realm_inner) else {
                    panic!("inner completion expected");
                };
                done
            });
            assert!(start(&outer_link, &outer_inside, realm_outer, outer));
            let Some(Yield::Yielded(v)) = outer_link.recv(realm_outer) else {
                panic!("{b:?}: outer yield expected");
            };
            assert_eq!(num(&v), 2.0);
            outer_link.send(realm_outer, Resume::Value(Value::Number(4.0)));
            let Some(Yield::Done(Ok(v))) = outer_link.recv(realm_outer) else {
                panic!("{b:?}: outer completion expected");
            };
            assert_eq!(num(&v), 12.0);
        }
    }

    #[test]
    fn dropping_the_host_end_cancels_a_suspended_coroutine() {
        for b in backends() {
            let realm = 0x5300usize;
            let (link, inside) = new_pair_on(b);
            let inside = Rc::new(inside);
            let body_side = inside.clone();
            // a capture that the body keeps alive while it is suspended
            let token = Rc::new(());
            let held = token.clone();
            let body: Body = Box::new(move || {
                let _held = held;
                body_side.suspend(realm, Yield::Yielded(Value::Undefined));
                Ok(Value::Undefined)
            });
            assert!(start(&link, &inside, realm, body));
            assert!(matches!(link.recv(realm), Some(Yield::Yielded(_))));
            match b {
                Backend::Thread => {
                    // the host lets go while the body is suspended: the body parks for good
                    let before = crate::co_thread::DISCONNECT_PARKS.load(Ordering::SeqCst);
                    drop(link);
                    drop(inside);
                    let mut parked = false;
                    for _ in 0..5000 {
                        if crate::co_thread::DISCONNECT_PARKS.load(Ordering::SeqCst) > before {
                            parked = true;
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(1));
                    }
                    assert!(parked, "suspended coroutine was not cancelled");
                }
                Backend::Stack => {
                    // the frames leak (their capture is never dropped) and the stack is recycled
                    drop(link);
                    drop(inside);
                    assert_eq!(
                        Rc::strong_count(&token),
                        2,
                        "suspended frames must leak, not unwind"
                    );
                    // the recycled stack serves the next coroutine
                    let (l2, i2) = new_pair_on(b);
                    let i2 = Rc::new(i2);
                    let body2: Body = Box::new(|| Ok(Value::Number(7.0)));
                    assert!(start(&l2, &i2, realm, body2));
                    let Some(Yield::Done(Ok(v))) = l2.recv(realm) else {
                        panic!("completion expected");
                    };
                    assert_eq!(num(&v), 7.0);
                }
            }
        }
    }

    #[test]
    fn panic_in_the_body_surfaces_on_the_host() {
        for b in backends() {
            let realm = 0x5400usize;
            let (link, inside) = new_pair_on(b);
            let inside = Rc::new(inside);
            let body_side = inside.clone();
            let body: Body = Box::new(move || {
                body_side.suspend(realm, Yield::Yielded(Value::Undefined));
                panic!("boom in body");
            });
            assert!(start(&link, &inside, realm, body));
            assert!(matches!(link.recv(realm), Some(Yield::Yielded(_))));
            link.send(realm, Resume::Value(Value::Undefined));
            drop(inside);
            let outcome = catch_unwind(AssertUnwindSafe(|| link.recv(realm)));
            match b {
                // the costack re-raises the body's own panic on the host stack
                Backend::Stack => {
                    let payload = match outcome {
                        Err(payload) => payload,
                        Ok(_) => panic!("host must see the panic"),
                    };
                    assert_eq!(
                        payload.downcast_ref::<&str>().copied(),
                        Some("boom in body")
                    );
                }
                // the coroutine thread dies; the host sees its channel close
                Backend::Thread => assert!(matches!(outcome, Ok(None))),
            }
        }
    }

    /// Holds every costack slot at once (`MAX_LIVE`), then expects the next start to fail. Needs the
    /// process to itself: the live count is shared by every test thread. Run with `--ignored`.
    #[cfg(all(
        test,
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    ))]
    #[test]
    #[ignore = "holds all 30,000 costack slots; run alone with --ignored"]
    fn stack_start_reports_the_live_limit_as_false() {
        const MAX_LIVE: usize = 30_000;
        let mut held = Vec::with_capacity(MAX_LIVE);
        for _ in 0..MAX_LIVE {
            let (link, inside) = new_pair_on(Backend::Stack);
            let body: Body = Box::new(|| Ok(Value::Undefined));
            assert!(start_small_stack(&link, body));
            held.push((link, inside));
        }
        let (link, inside) = new_pair_on(Backend::Stack);
        let body: Body = Box::new(|| Ok(Value::Undefined));
        assert!(!start_small_stack(&link, body));
        drop(inside);
        drop(held);
        let (link, inside) = new_pair_on(Backend::Stack);
        let body: Body = Box::new(|| Ok(Value::Undefined));
        assert!(start_small_stack(&link, body), "slots return on drop");
        drop(inside);
    }
}
