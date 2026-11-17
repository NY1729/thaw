//! Thread backend of the coroutine seam (`co.rs`).
//!
//! A coroutine is an OS thread with a `STACK_BYTES` stack. Host and coroutine strictly alternate
//! (exactly one runs at a time), each hand-off a message over a channel. A thread does not share
//! thread-locals with the other side, so the realm's host tables (`host::park_host_tls` /
//! `adopt_host_tls`) and the native ready queue are moved with the control at every hand-off.
//! ponytail: each finished coroutine thread leaks its bump chunks (see thaw_arena_detach_thread).
//! Upgrade path: adopt the arena into the host thread.

use std::sync::mpsc::{channel, Receiver, Sender};

use crate::co::{Body, Resume, Yield};
use crate::host::{adopt_host_tls, park_host_tls};

/// Stack size of one coroutine thread; the body gets `configure_stack(STACK_BYTES)` on it.
pub(crate) const STACK_BYTES: usize = 64 << 20;

/// Moves `Rc`-bearing data across the hand-off; sound only because of the strict alternation.
struct SendBox<T>(T);
// SAFETY: see the module docs; only one side touches the payload at a time.
unsafe impl<T> Send for SendBox<T> {}
impl<T> SendBox<T> {
    fn into_inner(self) -> T {
        self.0
    }
}

pub(crate) struct Link {
    to_co: Sender<SendBox<Resume>>,
    from_co: Receiver<SendBox<Yield>>,
}

pub(crate) struct Inside {
    to_host: Sender<SendBox<Yield>>,
    from_host: Receiver<SendBox<Resume>>,
}

pub(crate) fn new_pair() -> (Link, Inside) {
    let (to_co, from_host) = channel();
    let (to_host, from_co) = channel();
    (Link { to_co, from_co }, Inside { to_host, from_host })
}

/// Starts the coroutine thread running `body`. The thread takes realm `realm`'s host tables.
pub(crate) fn start(inside: &Inside, realm: usize, body: Body) -> bool {
    let done_tx = inside.to_host.clone();
    let payload = SendBox(body);
    park_host_tls(realm);
    let spawned = std::thread::Builder::new()
        .stack_size(STACK_BYTES)
        .spawn(move || {
            adopt_host_tls(realm);
            let r = payload.into_inner()();
            // every Rc the body captured is dropped by the call above, before the hand-back
            park_native_ready();
            thaw_arena::thaw_arena_detach_thread();
            park_host_tls(realm);
            let _ = done_tx.send(SendBox(Yield::Done(r)));
        });
    if spawned.is_err() {
        adopt_host_tls(realm);
    }
    spawned.is_ok()
}

impl Link {
    pub(crate) fn send(&self, realm: usize, msg: Resume) {
        park_host_tls(realm);
        let _ = self.to_co.send(SendBox(msg));
    }

    pub(crate) fn recv(&self, realm: usize) -> Option<Yield> {
        let y = self.from_co.recv();
        adopt_native_ready();
        adopt_host_tls(realm);
        y.ok().map(SendBox::into_inner)
    }
}

impl Inside {
    pub(crate) fn suspend(&self, realm: usize, msg: Yield) -> Resume {
        park_native_ready();
        park_host_tls(realm);
        let _ = self.to_host.send(SendBox(msg));
        // the host dropped this coroutine (promise never settled): never touch shared state again
        let Ok(resume) = self.from_host.recv() else {
            park_disconnected()
        };
        adopt_host_tls(realm);
        resume.into_inner()
    }
}

// ----- native ready hand-off and cancellation (thread-only) -----
type ReadyHook = extern "C" fn();

/// The native runtime's ready-continuation hand-off (`thaw_runtime_park_ready` / `thaw_runtime_adopt_ready`),
/// resolved through RTLD_DEFAULT like `drain_native_continuations`. Missing symbols make both no-ops.
static NATIVE_READY_HOOKS: std::sync::OnceLock<Option<(ReadyHook, ReadyHook)>> =
    std::sync::OnceLock::new();

/// Test override for the hand-off hooks (process-wide: coroutine threads must see it too).
#[cfg(test)]
pub(crate) static TEST_READY_HOOKS: std::sync::Mutex<Option<(ReadyHook, ReadyHook)>> =
    std::sync::Mutex::new(None);

/// Test-only count of coroutines that parked because their host dropped its channel ends.
#[cfg(test)]
pub(crate) static DISCONNECT_PARKS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// A suspended coroutine whose host dropped its channel ends: park forever. Callers must not
/// touch the Interp, TLS hand-offs or native-ready state before reaching this.
pub(crate) fn park_disconnected() -> ! {
    #[cfg(test)]
    DISCONNECT_PARKS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    loop {
        std::thread::park();
    }
}

fn ready_hooks() -> Option<(ReadyHook, ReadyHook)> {
    #[cfg(test)]
    if let Some(hooks) = *TEST_READY_HOOKS
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
    {
        return Some(hooks);
    }
    *NATIVE_READY_HOOKS.get_or_init(resolve_native_ready_hooks)
}

#[cfg(unix)]
fn resolve_native_ready_hooks() -> Option<(ReadyHook, ReadyHook)> {
    // SAFETY: the symbols are the compiled runtime's `extern "C" fn()`s, looked up like the other natives.
    unsafe {
        let park = libc::dlsym(libc::RTLD_DEFAULT, c"thaw_runtime_park_ready".as_ptr());
        let adopt = libc::dlsym(libc::RTLD_DEFAULT, c"thaw_runtime_adopt_ready".as_ptr());
        if park.is_null() || adopt.is_null() {
            return None;
        }
        Some((
            std::mem::transmute::<*mut std::ffi::c_void, ReadyHook>(park),
            std::mem::transmute::<*mut std::ffi::c_void, ReadyHook>(adopt),
        ))
    }
}

#[cfg(not(unix))]
fn resolve_native_ready_hooks() -> Option<(ReadyHook, ReadyHook)> {
    None
}

/// Coroutine side, just before every hand-back: the continuations it settled move to the parked queue.
pub(crate) fn park_native_ready() {
    if let Some((park, _)) = ready_hooks() {
        park();
    }
}

/// Host side, right after a coroutine hands back: the parked continuations join this thread's queue.
pub(crate) fn adopt_native_ready() {
    if let Some((_, adopt)) = ready_hooks() {
        adopt();
    }
}
