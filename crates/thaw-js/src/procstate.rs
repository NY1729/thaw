//! Process state shared by every realm of one host thread: `process.exitCode` and the request to exit.
//! Each package has its own interpreter (and its own `process` object), so `process.exit` in one package
//! and `process.exitCode = n` in another must agree on the status. Thread-local: a worker keeps its own.

use std::cell::Cell;
use std::rc::Rc;

#[derive(Default)]
pub(crate) struct ProcessShared {
    /// `process.exitCode`; None is `undefined`.
    pub(crate) exit_code: Cell<Option<f64>>,
    /// Set by `process.exit()`, read by the host so the process ends at the host boundary.
    pub(crate) exit_requested: Cell<bool>,
}

thread_local! {
    static HOST_SHARED: Rc<ProcessShared> = Rc::new(ProcessShared::default());
}

/// The state every package loaded on this thread shares.
pub(crate) fn host_shared() -> Rc<ProcessShared> {
    HOST_SHARED.with(Rc::clone)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_state_is_per_thread() {
        std::thread::spawn(|| host_shared().exit_code.set(Some(1.0)))
            .join()
            .unwrap();
        assert_eq!(host_shared().exit_code.get(), None);
    }
}
