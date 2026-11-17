//! Costack backend of the coroutine seam (`co.rs`).
//!
//! A coroutine is a `costack` stack switched to and from with assembly. Host and body run on one
//! OS thread, so nothing is parked or adopted at a hand-off: the realm's host tables and the
//! native ready queue are already the same ones on both sides.
//!
//! The hand-off is a pair of cells shared by the two ends (`Mailbox`). `send` stores the resume
//! message; `recv` switches to the body (`Costack::resume`) and takes what the body left in the
//! outbox. `Inside::suspend` leaves its message in the outbox and switches back.
//!
//! Lifetime: the `Costack` lives in the `Link`. When the body finishes, `recv` drops it. Dropping a
//! `Link` while its body is suspended leaks that body's frames (their captures are never dropped)
//! and recycles the stack; nothing ever resumes those frames again.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::co::{Body, Resume, Yield};
use crate::costack::{self, Costack, Status};

/// The hand-off cells of one coroutine, shared by its `Link` and `Inside`. Same thread: plain cells.
pub(crate) struct Mailbox {
    inbox: Cell<Option<Resume>>,
    outbox: Cell<Option<Yield>>,
}

/// Host side. Owns the costack for as long as the body has not finished.
pub(crate) struct Link {
    mb: Rc<Mailbox>,
    cs: RefCell<Option<Costack>>,
}

/// Coroutine side, held by the `CoCtx` the interpreter keeps on `cur_co`.
pub(crate) struct Inside {
    mb: Rc<Mailbox>,
}

pub(crate) fn new_pair() -> (Link, Inside) {
    let mb = Rc::new(Mailbox {
        inbox: Cell::new(None),
        outbox: Cell::new(None),
    });
    (
        Link {
            mb: mb.clone(),
            cs: RefCell::new(None),
        },
        Inside { mb },
    )
}

/// Wraps `body` so that its completion lands in the outbox before the final switch back.
fn wrap(mb: Rc<Mailbox>, body: Body) -> Box<dyn FnOnce()> {
    Box::new(move || {
        let r = body();
        mb.outbox.set(Some(Yield::Done(r)));
    })
}

/// Creates the costack for `body` on `link`. False when no stack could be made (live limit,
/// map failure, unsupported target); the body is dropped unrun.
pub(crate) fn start(link: &Link, body: Body) -> bool {
    let made = Costack::new(wrap(link.mb.clone(), body));
    match made {
        Ok(cs) => {
            *link.cs.borrow_mut() = Some(cs);
            true
        }
        Err(_) => false,
    }
}

/// `start` with a small stack, for the live-limit test (the default size is `costack::STACK_BYTES`).
#[cfg(all(
    test,
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
pub(crate) fn start_small(link: &Link, body: Body, len: usize) -> bool {
    match Costack::with_stack_size(wrap(link.mb.clone(), body), len) {
        Ok(cs) => {
            *link.cs.borrow_mut() = Some(cs);
            true
        }
        Err(_) => false,
    }
}

impl Link {
    pub(crate) fn send(&self, msg: Resume) {
        let prev = self.mb.inbox.replace(Some(msg));
        debug_assert!(
            prev.is_none(),
            "stack coroutine: resume message not consumed"
        );
    }

    pub(crate) fn recv(&self) -> Option<Yield> {
        let status = {
            let cs = self.cs.borrow();
            let cs = cs.as_ref()?;
            // the coroutine runs on its own stack: `too_deep` must measure against these bounds, not the host thread's
            let _bounds = crate::costack::StackBoundsScope::enter(cs.bounds());
            cs.resume()
        };
        if status == Status::Finished {
            // the body is done and its stack already released by `resume`
            self.cs.borrow_mut().take();
        }
        self.mb.outbox.take()
    }
}

impl Inside {
    /// Leaves `msg` for the host and switches back. Returns the host's next message on resume.
    pub(crate) fn suspend(&self, msg: Yield) -> Resume {
        let prev = self.mb.outbox.replace(Some(msg));
        debug_assert!(prev.is_none(), "stack coroutine: yield message not taken");
        costack::suspend();
        self.mb
            .inbox
            .take()
            .expect("stack coroutine resumed without a message")
    }
}
