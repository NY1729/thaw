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
//! vtable and no generic parameter on `Interp`. Only the thread backend exists (co_thread.rs).

use crate::*;

use crate::co_thread as thread;

pub(crate) use crate::co_thread::STACK_BYTES;

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
}

/// Coroutine side of one coroutine. Held by the `CoCtx` the interpreter keeps on `cur_co`.
pub(crate) enum Inside {
    Thread(thread::Inside),
}

/// A connected host/coroutine pair. Nothing runs until `start`.
pub(crate) fn new_pair() -> (Link, Inside) {
    let (link, inside) = thread::new_pair();
    (Link::Thread(link), Inside::Thread(inside))
}

/// Starts the coroutine behind `inside` running `body`; it runs until its first hand-back. `realm`
/// is the Interp key. False when the backend could not start one (the body is dropped unrun).
pub(crate) fn start(inside: &Inside, realm: usize, body: Body) -> bool {
    match inside {
        Inside::Thread(i) => thread::start(i, realm, body),
    }
}

impl Link {
    /// Hands control to the coroutine with `msg`. It runs until it hands control back.
    pub(crate) fn send(&self, realm: usize, msg: Resume) {
        match self {
            Link::Thread(l) => l.send(realm, msg),
        }
    }

    /// Waits until the coroutine hands control back. `None` if it vanished without doing so.
    pub(crate) fn recv(&self, realm: usize) -> Option<Yield> {
        match self {
            Link::Thread(l) => l.recv(realm),
        }
    }
}

impl Inside {
    /// Hands `msg` to the host and waits to be resumed. Called from inside the body only.
    /// Never returns when the host has dropped this coroutine: the backend parks it for good.
    pub(crate) fn suspend(&self, realm: usize, msg: Yield) -> Resume {
        match self {
            Inside::Thread(i) => i.suspend(realm, msg),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    fn num(v: &Value) -> f64 {
        match v {
            Value::Number(n) => *n,
            _ => panic!("number expected"),
        }
    }

    #[test]
    fn resume_yield_finish_pass_messages_both_ways() {
        let realm = 0x5100usize;
        let (link, inside) = new_pair();
        let inside = Rc::new(inside);
        let body_side = inside.clone();
        let body: Body = Box::new(move || {
            let Resume::Value(v) = body_side.suspend(realm, Yield::Yielded(Value::Number(1.0)))
            else {
                panic!("value resume expected");
            };
            Ok(Value::Number(num(&v) + 10.0))
        });
        assert!(start(&inside, realm, body));
        let Some(Yield::Yielded(v)) = link.recv(realm) else {
            panic!("yield expected");
        };
        assert_eq!(num(&v), 1.0);
        link.send(realm, Resume::Value(Value::Number(5.0)));
        let Some(Yield::Done(Ok(v))) = link.recv(realm) else {
            panic!("completion expected");
        };
        assert_eq!(num(&v), 15.0);
    }

    #[test]
    fn nested_coroutine_resumes_another() {
        let (realm_outer, realm_inner) = (0x5200usize, 0x5201usize);
        let (outer_link, outer_inside) = new_pair();
        let outer_inside = Rc::new(outer_inside);
        let outer_side = outer_inside.clone();
        let outer: Body = Box::new(move || {
            let (inner_link, inner_inside) = new_pair();
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
            assert!(start(&inner_inside, realm_inner, inner));
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
        assert!(start(&outer_inside, realm_outer, outer));
        let Some(Yield::Yielded(v)) = outer_link.recv(realm_outer) else {
            panic!("outer yield expected");
        };
        assert_eq!(num(&v), 2.0);
        outer_link.send(realm_outer, Resume::Value(Value::Number(4.0)));
        let Some(Yield::Done(Ok(v))) = outer_link.recv(realm_outer) else {
            panic!("outer completion expected");
        };
        assert_eq!(num(&v), 12.0);
    }

    #[test]
    fn dropping_the_host_end_cancels_a_suspended_coroutine() {
        let realm = 0x5300usize;
        let before = crate::co_thread::DISCONNECT_PARKS.load(Ordering::SeqCst);
        let (link, inside) = new_pair();
        let inside = Rc::new(inside);
        let body_side = inside.clone();
        let body: Body = Box::new(move || {
            body_side.suspend(realm, Yield::Yielded(Value::Undefined));
            Ok(Value::Undefined)
        });
        assert!(start(&inside, realm, body));
        assert!(matches!(link.recv(realm), Some(Yield::Yielded(_))));
        // the host lets go while the body is suspended: the body parks for good, without touching the realm
        drop(link);
        for _ in 0..5000 {
            if crate::co_thread::DISCONNECT_PARKS.load(Ordering::SeqCst) > before {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        panic!("suspended coroutine was not cancelled");
    }
}
