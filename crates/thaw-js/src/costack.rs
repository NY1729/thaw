//! Stackful coroutine primitive: a body runs on its own mmap'd stack and switches to and from its
//! resumer with hand-written assembly. Same OS thread throughout, so every thread-local is shared
//! with the host by construction. Not wired into the coroutine seam yet (`co.rs`, PR 3).
//!
//! Supported: Linux on x86_64 and aarch64. Every other target gets a stub whose `Costack::new`
//! returns `CostackError::Unsupported`; the thread backend stays in use there.
//!
//! Layout and policy:
//! - One mapping per stack: a 4 KiB `PROT_NONE` guard at the bottom, then 64 MiB of usable stack
//!   (`MAP_NORESERVE | MAP_STACK`, `MADV_NOHUGEPAGE`). Committed memory is the touched pages only.
//! - Idle stacks are pooled per thread (LIFO, at most `POOL_CAP`). A stack that went deep (a canary
//!   word 128 KiB below the top was overwritten) is decommitted with `MADV_DONTNEED` on release;
//!   a shallow one costs no syscall.
//! - Stacks held by coroutines are capped at `MAX_LIVE` (2 VMAs each, `vm.max_map_count` is 65530
//!   by default). Beyond it `Costack::new` fails with `CostackError::TooManyLive`.
//! - A panic in the body is caught at the entry trampoline, stored, and re-raised by the `resume`
//!   that observed it. It never unwinds across the assembly frames.
//! - Dropping a suspended costack leaks its frames: no destructor runs on the suspended stack, the
//!   captures it holds are leaked, and the stack is recycled as is. `cancel` is the unwinding
//!   alternative: the body's `suspend` panics with a private token, its Rust frames drop normally,
//!   and the entry returns. A body must not swallow that token.
//!
//! DWARF and backtraces: the trampoline `thaw_costack_start` carries `.cfi_undefined` for its return
//! address, so unwinders stop there. Backtraces and panic payloads are complete only within one
//! stack segment; the cross-stack hop is the `resume`/`suspend` pair. Frame pointers are zeroed at
//! the stack start for the same reason.
//!
//! Stack bounds: `pthread_getattr_np` reports the HOST thread's stack, which is wrong while a costack
//! runs. `current_bounds()` gives the running costack's `(lo, hi)`, and a resumer holds a
//! `StackBoundsScope` around `resume` so that thaw-arena's stack guard uses the same bounds.

/// While alive, the thaw-arena stack guard treats the given costack bounds as this thread's stack
/// (`thaw_arena::stack_guard::set_stack_bounds_override`). Hold one around each `Costack::resume` on
/// the resumer's side; dropping it restores what was set before, so nested resumes unwind cleanly.
#[allow(dead_code)] // held by the coroutine seam (co3a) around each resume
pub(crate) struct StackBoundsScope {
    prev: Option<(usize, usize)>,
}

impl StackBoundsScope {
    #[allow(dead_code)]
    pub(crate) fn enter(bounds: (usize, usize)) -> StackBoundsScope {
        StackBoundsScope {
            prev: thaw_arena::stack_guard::set_stack_bounds_override(Some(bounds)),
        }
    }
}

impl Drop for StackBoundsScope {
    fn drop(&mut self) {
        thaw_arena::stack_guard::set_stack_bounds_override(self.prev);
    }
}

#[cfg(all(
    unix,
    any(target_arch = "x86_64", target_arch = "aarch64"),
    target_os = "linux"
))]
mod imp {
    use std::any::Any;
    use std::cell::{Cell, RefCell};
    use std::fmt;
    use std::io;
    use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Virtual size of one coroutine stack (the same figure the thread backend uses).
    pub(crate) const STACK_BYTES: usize = 64 << 20;
    const GUARD_BYTES: usize = 4096;
    /// Idle stacks kept for reuse per thread; the rest are unmapped.
    const POOL_CAP: usize = 256;
    /// Cap on stacks held by live coroutines at once, process-wide (all threads share it).
    ///
    /// Why: each stack is two VMAs (the mapping, and the `PROT_NONE` guard that splits it), and the kernel
    /// refuses new mappings past `vm.max_map_count`, 65530 by default. With the pool, mappings stay under
    /// 2 * (MAX_LIVE + POOL_CAP) = 60,512, which is under that default with headroom for the rest of the
    /// process. Past the cap `Costack::new` fails with `TooManyLive`, and the JS caller gets
    /// `RangeError: cannot start coroutine (thread limit)`. Raising it means a larger `vm.max_map_count` or
    /// fewer VMAs per stack (no guard page). `THAW_JS_CO=thread` has no such cap: it is bounded only by
    /// threads and pids, so it is the escape hatch when a program really holds more than 30,000 at once.
    pub(crate) const MAX_LIVE: usize = 30_000;
    /// Depth of the canary word below the top: a stack that overwrote it went deep.
    const CANARY_DEPTH: usize = 128 << 10;
    const CANARY: usize = 0x7a3c_c05c_ca9a_7c01;
    /// Top bytes kept committed when a deep stack is decommitted on release.
    const KEEP_TOP: usize = 32 << 10;
    /// Initial x87 control word and MXCSR for a fresh coroutine (the Rust defaults).
    #[cfg(target_arch = "x86_64")]
    const DEFAULT_FPUCW: u64 = 0x037f;
    #[cfg(target_arch = "x86_64")]
    const DEFAULT_MXCSR: u64 = 0x1f80;

    /// Stacks currently held by coroutines (checked against `MAX_LIVE`).
    static IN_USE: AtomicUsize = AtomicUsize::new(0);

    #[derive(Debug)]
    pub(crate) enum CostackError {
        TooManyLive,
        Map(io::Error),
        Unsupported,
    }

    impl fmt::Display for CostackError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                CostackError::TooManyLive => write!(f, "cannot start coroutine (thread limit)"),
                CostackError::Map(e) => write!(f, "cannot map coroutine stack: {e}"),
                CostackError::Unsupported => {
                    write!(f, "stackful coroutines are not supported here")
                }
            }
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) enum Status {
        Suspended,
        Finished,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum State {
        Fresh,
        Running,
        Suspended,
        Finished,
        /// Finished by a panic; the payload is re-raised by the resume that observed it.
        Poisoned,
    }

    /// Payload of the private unwind that `cancel` triggers inside `suspend`.
    struct Cancelled;

    /// One mapping: guard page at the bottom, usable stack above it.
    struct Stack {
        base: *mut u8,
        len: usize,
    }

    impl Stack {
        fn map(len: usize) -> io::Result<Stack> {
            // SAFETY: anonymous private mapping, no address hint; checked for MAP_FAILED below.
            let base = unsafe {
                libc::mmap(
                    std::ptr::null_mut(),
                    len,
                    libc::PROT_READ | libc::PROT_WRITE,
                    libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_NORESERVE | libc::MAP_STACK,
                    -1,
                    0,
                )
            };
            if base == libc::MAP_FAILED {
                return Err(io::Error::last_os_error());
            }
            // From here on `Drop` unmaps the region on any early return.
            let stack = Stack {
                base: base.cast(),
                len,
            };
            // SAFETY: the guard is the lowest page of the mapping created above.
            if unsafe { libc::mprotect(base, GUARD_BYTES, libc::PROT_NONE) } != 0 {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: advisory hint on our own mapping; failure only costs THP avoidance.
            unsafe { libc::madvise(base, len, libc::MADV_NOHUGEPAGE) };
            stack.set_canary();
            Ok(stack)
        }

        fn top(&self) -> usize {
            self.base as usize + self.len
        }

        fn set_canary(&self) {
            // SAFETY: CANARY_DEPTH is well inside the usable part of the mapping (len >= 256 KiB).
            unsafe { ((self.top() - CANARY_DEPTH) as *mut usize).write(CANARY) }
        }

        fn is_deep(&self) -> bool {
            // SAFETY: same word as `set_canary`; reading it inside our mapping.
            unsafe { ((self.top() - CANARY_DEPTH) as *const usize).read() != CANARY }
        }
    }

    impl Drop for Stack {
        fn drop(&mut self) {
            // SAFETY: base and len are exactly what mmap returned for this stack.
            unsafe { libc::munmap(self.base.cast(), self.len) };
        }
    }

    thread_local! {
        static POOL: RefCell<Vec<Stack>> = const { RefCell::new(Vec::new()) };
    }

    fn acquire(len: usize) -> io::Result<Stack> {
        if len == STACK_BYTES {
            if let Ok(Some(stack)) = POOL.try_with(|p| p.borrow_mut().pop()) {
                return Ok(stack);
            }
        }
        Stack::map(len)
    }

    /// Returns a finished or abandoned stack. Host side only: nothing runs on it any more.
    fn release(stack: Stack) {
        IN_USE.fetch_sub(1, Ordering::SeqCst);
        if stack.is_deep() {
            // SAFETY: the range is inside our usable mapping and no coroutine runs on it.
            unsafe {
                libc::madvise(
                    (stack.base as usize + GUARD_BYTES) as *mut libc::c_void,
                    stack.len - GUARD_BYTES - KEEP_TOP,
                    libc::MADV_DONTNEED,
                )
            };
            stack.set_canary();
        }
        if stack.len == STACK_BYTES {
            // If the pool is full or thread-local storage is gone, `stack` drops here and unmaps.
            let _ = POOL.try_with(|p| {
                let mut p = p.borrow_mut();
                if p.len() < POOL_CAP {
                    p.push(stack);
                }
            });
        }
    }

    struct Inner {
        /// Owned while the coroutine holds it; taken back by `release`.
        stack: Cell<Option<Stack>>,
        /// Lowest usable address (just above the guard page) and the top of the stack.
        lo: usize,
        top: usize,
        body: Cell<Option<Box<dyn FnOnce()>>>,
        state: Cell<State>,
        /// Saved stack pointer of the coroutine while it is suspended (or its initial frame).
        co_sp: Cell<usize>,
        /// Saved stack pointer of the resumer while the coroutine runs.
        host_sp: Cell<usize>,
        cancel: Cell<bool>,
        panic: Cell<Option<Box<dyn Any + Send>>>,
    }

    /// A stackful coroutine. Not `Send`: it must be resumed on the thread that created it.
    pub(crate) struct Costack {
        inner: Box<Inner>,
    }

    thread_local! {
        /// The coroutine running on this thread, null on the host stack.
        static CURRENT: Cell<*const Inner> = const { Cell::new(std::ptr::null()) };
    }

    // Context switch: save the callee-saved registers (plus MXCSR and the x87 control word on x86_64)
    // on the current stack, store the stack pointer in `*save`, load `to` and restore the same set.
    #[cfg(target_arch = "x86_64")]
    core::arch::global_asm!(
        ".globl thaw_costack_switch",
        ".p2align 4",
        "thaw_costack_switch:",
        "push rbp",
        "push rbx",
        "push r12",
        "push r13",
        "push r14",
        "push r15",
        "sub rsp, 8",
        "stmxcsr [rsp]",
        "fnstcw [rsp + 4]",
        "mov [rdi], rsp",
        "mov rsp, rsi",
        "ldmxcsr [rsp]",
        "fldcw [rsp + 4]",
        "add rsp, 8",
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop rbx",
        "pop rbp",
        "ret",
        // First activation: `ret` lands here with r12 = Inner pointer and r13 = entry.
        ".globl thaw_costack_start",
        ".p2align 4",
        "thaw_costack_start:",
        ".cfi_startproc",
        ".cfi_undefined rip",
        "mov rdi, r12",
        "call r13",
        "ud2",
        ".cfi_endproc",
    );

    // Same contract on aarch64: x19-x30 and d8-d15 (the AAPCS64 callee-saved set), 160 bytes.
    #[cfg(target_arch = "aarch64")]
    core::arch::global_asm!(
        ".globl thaw_costack_switch",
        ".p2align 4",
        "thaw_costack_switch:",
        "sub sp, sp, #160",
        "stp x19, x20, [sp, #0]",
        "stp x21, x22, [sp, #16]",
        "stp x23, x24, [sp, #32]",
        "stp x25, x26, [sp, #48]",
        "stp x27, x28, [sp, #64]",
        "stp x29, x30, [sp, #80]",
        "stp d8, d9, [sp, #96]",
        "stp d10, d11, [sp, #112]",
        "stp d12, d13, [sp, #128]",
        "stp d14, d15, [sp, #144]",
        "mov x2, sp",
        "str x2, [x0]",
        "mov sp, x1",
        "ldp d14, d15, [sp, #144]",
        "ldp d12, d13, [sp, #128]",
        "ldp d10, d11, [sp, #112]",
        "ldp d8, d9, [sp, #96]",
        "ldp x29, x30, [sp, #80]",
        "ldp x27, x28, [sp, #64]",
        "ldp x25, x26, [sp, #48]",
        "ldp x23, x24, [sp, #32]",
        "ldp x21, x22, [sp, #16]",
        "ldp x19, x20, [sp, #0]",
        "add sp, sp, #160",
        "ret",
        // First activation: `ret` lands here with x19 = entry and x20 = Inner pointer.
        ".globl thaw_costack_start",
        ".p2align 4",
        "thaw_costack_start:",
        ".cfi_startproc",
        ".cfi_undefined x30",
        "mov x0, x20",
        "blr x19",
        "brk #0",
        ".cfi_endproc",
    );

    extern "C" {
        fn thaw_costack_switch(save: *mut usize, to: usize);
        fn thaw_costack_start();
    }

    /// Writes the frame that the first `thaw_costack_switch` into this stack pops; returns its sp.
    /// The `ret` into `thaw_costack_start` leaves sp 16-byte aligned, as the `call` there needs.
    fn initial_frame(top: usize, inner: *const Inner) -> usize {
        let entry: extern "C" fn(*const Inner) -> ! = entry;
        #[cfg(target_arch = "x86_64")]
        {
            // [ctl][r15][r14][r13=entry][r12=inner][rbx][rbp][ret]: 64 bytes, sp = top - 80.
            let sp = top - 80;
            let words = [
                DEFAULT_MXCSR | (DEFAULT_FPUCW << 32),
                0,
                0,
                entry as usize as u64,
                inner as u64,
                0,
                0,
                thaw_costack_start as *const () as usize as u64,
            ];
            for (i, w) in words.iter().enumerate() {
                // SAFETY: [sp, sp + 64) is inside the top of a fresh or pooled stack, 16-byte aligned.
                unsafe { ((sp + 8 * i) as *mut u64).write(*w) };
            }
            sp
        }
        #[cfg(target_arch = "aarch64")]
        {
            // 20 words: x19..x30 then d8..d15; x19 = entry, x20 = inner, x30 = trampoline.
            let sp = top - 160;
            let mut words = [0u64; 20];
            words[0] = entry as usize as u64;
            words[1] = inner as u64;
            words[11] = thaw_costack_start as *const () as usize as u64;
            for (i, w) in words.iter().enumerate() {
                // SAFETY: [sp, sp + 160) is inside the top of a fresh or pooled stack, 16-byte aligned.
                unsafe { ((sp + 8 * i) as *mut u64).write(*w) };
            }
            sp
        }
    }

    /// First code on a coroutine stack. Never returns to its caller.
    extern "C" fn entry(inner: *const Inner) -> ! {
        // SAFETY: `resume` passed the Inner that owns this stack; its owner keeps it alive for the run.
        let inner = unsafe { &*inner };
        let body = inner.body.take().expect("costack body already taken");
        match catch_unwind(AssertUnwindSafe(body)) {
            Ok(()) => inner.state.set(State::Finished),
            Err(p) if p.is::<Cancelled>() => inner.state.set(State::Finished),
            Err(p) => {
                inner.panic.set(Some(p));
                inner.state.set(State::Poisoned);
            }
        }
        // SAFETY: co_sp receives this frame; host_sp was saved by the resume that started the run.
        unsafe { thaw_costack_switch(inner.co_sp.as_ptr(), inner.host_sp.get()) };
        // A finished coroutine is never switched to again.
        std::process::abort()
    }

    impl Costack {
        pub(crate) fn new(body: Box<dyn FnOnce()>) -> Result<Costack, CostackError> {
            Self::with_stack_size(body, STACK_BYTES)
        }

        /// `new` with an explicit stack size (>= 256 KiB). Only the default size is pooled.
        pub(crate) fn with_stack_size(
            body: Box<dyn FnOnce()>,
            len: usize,
        ) -> Result<Costack, CostackError> {
            assert!(
                len >= 256 << 10 && len.is_multiple_of(4096),
                "bad costack size {len}"
            );
            // claim a live slot (a manual CAS loop: `fetch_update` is deprecated on newer toolchains,
            // its replacement `try_update` does not exist on older ones)
            let mut live = IN_USE.load(Ordering::SeqCst);
            loop {
                if live >= MAX_LIVE {
                    return Err(CostackError::TooManyLive);
                }
                match IN_USE.compare_exchange_weak(
                    live,
                    live + 1,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                ) {
                    Ok(_) => break,
                    Err(now) => live = now,
                }
            }
            let stack = match acquire(len) {
                Ok(s) => s,
                Err(e) => {
                    IN_USE.fetch_sub(1, Ordering::SeqCst);
                    return Err(CostackError::Map(e));
                }
            };
            let top = stack.top() & !15;
            let inner = Box::new(Inner {
                stack: Cell::new(None),
                lo: stack.base as usize + GUARD_BYTES,
                top,
                body: Cell::new(Some(body)),
                state: Cell::new(State::Fresh),
                co_sp: Cell::new(0),
                host_sp: Cell::new(0),
                cancel: Cell::new(false),
                panic: Cell::new(None),
            });
            let sp = initial_frame(top, &*inner);
            inner.co_sp.set(sp);
            inner.stack.set(Some(stack));
            Ok(Costack { inner })
        }

        /// Runs the body until its next `suspend` or its end. Panics from the body are re-raised here.
        pub(crate) fn resume(&self) -> Status {
            let inner = &*self.inner;
            match inner.state.get() {
                State::Fresh | State::Suspended => {}
                State::Running => panic!("costack: resume of a running coroutine"),
                State::Finished | State::Poisoned => return Status::Finished,
            }
            inner.state.set(State::Running);
            let prev = CURRENT.with(|c| c.replace(inner as *const Inner));
            // SAFETY: co_sp is a frame written by `new` or `suspend` on this stack and is not running.
            unsafe { thaw_costack_switch(inner.host_sp.as_ptr(), inner.co_sp.get()) };
            CURRENT.with(|c| c.set(prev));
            if inner.state.get() == State::Suspended {
                return Status::Suspended;
            }
            if let Some(stack) = inner.stack.take() {
                release(stack);
            }
            if let Some(payload) = inner.panic.take() {
                resume_unwind(payload);
            }
            Status::Finished
        }

        /// Unwinds a suspended body so its Rust frames drop their captures. The body must not
        /// catch the private token. A body that is not suspended is simply dropped.
        pub(crate) fn cancel(self) {
            if self.inner.state.get() == State::Suspended {
                self.inner.cancel.set(true);
                self.resume();
            }
        }

        /// `(lo, hi)` of the usable stack: what a `StackBoundsScope` around `resume` must advertise.
        #[allow(dead_code)]
        pub(crate) fn bounds(&self) -> (usize, usize) {
            (self.inner.lo, self.inner.top)
        }

        #[cfg(test)]
        fn stack_top(&self) -> usize {
            self.inner.top
        }
    }

    /// Bounds of the costack running on this thread, `None` on the host stack.
    pub(crate) fn current_bounds() -> Option<(usize, usize)> {
        let inner = CURRENT.with(|c| c.get());
        if inner.is_null() {
            return None;
        }
        // SAFETY: CURRENT is set only while the `resume` that owns this Inner is running its body.
        let inner = unsafe { &*inner };
        Some((inner.lo, inner.top))
    }

    /// True while a costack body runs on this thread (co3a's `Inside::suspend` debug check).
    #[allow(dead_code)]
    pub(crate) fn in_coroutine() -> bool {
        !CURRENT.with(|c| c.get()).is_null()
    }

    impl Drop for Costack {
        /// Dropping a suspended body leaks its frames (see the module docs); the stack is recycled.
        fn drop(&mut self) {
            if let Some(stack) = self.inner.stack.take() {
                release(stack);
            }
        }
    }

    /// Suspends the running coroutine and returns control to its resumer. Call from inside a body.
    /// Returns when the body is resumed again; panics with the cancel token if it was cancelled.
    pub(crate) fn suspend() {
        let inner = CURRENT.with(|c| c.get());
        assert!(
            !inner.is_null(),
            "costack::suspend called outside a coroutine"
        );
        // SAFETY: CURRENT is set by the `resume` that runs this body; its Inner outlives the run.
        let inner = unsafe { &*inner };
        inner.state.set(State::Suspended);
        // SAFETY: co_sp receives this frame; host_sp was saved by the `resume` that runs us.
        unsafe { thaw_costack_switch(inner.co_sp.as_ptr(), inner.host_sp.get()) };
        if inner.cancel.get() {
            resume_unwind(Box::new(Cancelled));
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::costack::StackBoundsScope;
        use std::rc::Rc;
        use std::sync::Arc;
        use std::time::Instant;

        fn rss_bytes() -> Option<usize> {
            let s = std::fs::read_to_string("/proc/self/statm").ok()?;
            let pages: usize = s.split_whitespace().nth(1)?.parse().ok()?;
            Some(pages * 4096)
        }

        #[test]
        fn costack_create_resume_suspend_finish() {
            let log = Rc::new(RefCell::new(Vec::new()));
            let l = log.clone();
            let c = Costack::new(Box::new(move || {
                l.borrow_mut().push(1);
                suspend();
                l.borrow_mut().push(2);
                suspend();
                l.borrow_mut().push(3);
            }))
            .unwrap();
            assert!(log.borrow().is_empty());
            assert_eq!(c.resume(), Status::Suspended);
            assert_eq!(*log.borrow(), [1]);
            assert_eq!(c.resume(), Status::Suspended);
            assert_eq!(*log.borrow(), [1, 2]);
            assert_eq!(c.resume(), Status::Finished);
            assert_eq!(*log.borrow(), [1, 2, 3]);
            assert_eq!(c.resume(), Status::Finished);
        }

        #[test]
        fn costack_values_pass_through_a_shared_cell() {
            let cell = Rc::new(Cell::new(5u64));
            let c2 = cell.clone();
            let c = Costack::new(Box::new(move || {
                c2.set(c2.get() * 2);
                suspend();
                c2.set(c2.get() + 1);
            }))
            .unwrap();
            assert_eq!(c.resume(), Status::Suspended);
            assert_eq!(cell.get(), 10);
            cell.set(7);
            assert_eq!(c.resume(), Status::Finished);
            assert_eq!(cell.get(), 8);
        }

        #[test]
        fn costack_ten_thousand_sequential() {
            let total = Rc::new(Cell::new(0u64));
            for i in 0..10_000u64 {
                let t = total.clone();
                let c = Costack::new(Box::new(move || {
                    suspend();
                    t.set(t.get() + i);
                }))
                .unwrap();
                assert_eq!(c.resume(), Status::Suspended);
                assert_eq!(c.resume(), Status::Finished);
            }
            assert_eq!(total.get(), (0..10_000u64).sum::<u64>());
        }

        #[test]
        fn costack_two_thousand_simultaneously_suspended() {
            const N: usize = 2_000;
            let done = Rc::new(Cell::new(0usize));
            let before = rss_bytes().unwrap_or(0);
            let mut all: Vec<Costack> = (0..N)
                .map(|_| {
                    let d = done.clone();
                    Costack::new(Box::new(move || {
                        suspend();
                        d.set(d.get() + 1);
                    }))
                    .unwrap()
                })
                .collect();
            for c in &all {
                assert_eq!(c.resume(), Status::Suspended);
            }
            let per = rss_bytes().unwrap_or(0).saturating_sub(before) / N;
            println!("costack: {per} bytes committed per suspended coroutine");
            assert!(per <= 64 << 10, "too much committed per coroutine: {per}");
            for c in &all {
                assert_eq!(c.resume(), Status::Finished);
            }
            assert_eq!(done.get(), N);
            all.clear();
        }

        #[test]
        fn costack_nested_coroutine_resumes_another() {
            let out = Rc::new(Cell::new(0u64));
            let o = out.clone();
            let outer = Costack::new(Box::new(move || {
                let inner_val = Rc::new(Cell::new(0u64));
                let iv = inner_val.clone();
                let inner = Costack::new(Box::new(move || {
                    iv.set(3);
                    suspend();
                    iv.set(iv.get() * 10);
                }))
                .unwrap();
                assert_eq!(inner.resume(), Status::Suspended);
                o.set(inner_val.get());
                suspend();
                assert_eq!(inner.resume(), Status::Finished);
                o.set(o.get() + inner_val.get());
            }))
            .unwrap();
            assert_eq!(outer.resume(), Status::Suspended);
            assert_eq!(out.get(), 3);
            assert_eq!(outer.resume(), Status::Finished);
            assert_eq!(out.get(), 33);
        }

        #[inline(never)]
        fn recurse(n: u32) -> u64 {
            let buf = [n as u8; 1024];
            std::hint::black_box(&buf);
            if n == 0 {
                0
            } else {
                1 + recurse(n - 1)
            }
        }

        #[test]
        fn costack_deep_recursion_uses_the_big_stack() {
            // The host thread has only 256 KiB; 5000 frames of 1 KiB need about 5 MB.
            std::thread::Builder::new()
                .stack_size(256 << 10)
                .spawn(|| {
                    let got = Rc::new(Cell::new(0u64));
                    let g = got.clone();
                    let c = Costack::new(Box::new(move || {
                        suspend();
                        g.set(recurse(5000));
                    }))
                    .unwrap();
                    assert_eq!(c.resume(), Status::Suspended);
                    assert_eq!(c.resume(), Status::Finished);
                    assert_eq!(got.get(), 5000);
                })
                .unwrap()
                .join()
                .unwrap();
        }

        #[test]
        fn costack_panic_surfaces_at_resume() {
            let c = Costack::new(Box::new(|| {
                suspend();
                panic!("boom in costack");
            }))
            .unwrap();
            assert_eq!(c.resume(), Status::Suspended);
            let err = catch_unwind(AssertUnwindSafe(|| c.resume())).unwrap_err();
            assert_eq!(err.downcast_ref::<&str>().copied(), Some("boom in costack"));
            // poisoned: the payload is raised once, later resumes are no-ops
            assert_eq!(c.resume(), Status::Finished);
        }

        #[test]
        fn costack_stack_is_reused_from_the_pool() {
            let c = Costack::new(Box::new(|| {})).unwrap();
            let top = c.stack_top();
            assert_eq!(c.resume(), Status::Finished);
            drop(c);
            let d = Costack::new(Box::new(|| {})).unwrap();
            assert_eq!(d.stack_top(), top);
        }

        #[test]
        fn costack_drop_while_suspended_leaks_and_cancel_unwinds() {
            thread_local! { static DROPS: Cell<usize> = const { Cell::new(0) }; }
            struct Tracker;
            impl Drop for Tracker {
                fn drop(&mut self) {
                    DROPS.with(|d| d.set(d.get() + 1));
                }
            }
            // dropped while suspended: policy is leak, so the captured Tracker never drops
            let c = Costack::new(Box::new(|| {
                let _t = Tracker;
                suspend();
            }))
            .unwrap();
            assert_eq!(c.resume(), Status::Suspended);
            drop(c);
            assert_eq!(DROPS.with(|d| d.get()), 0);
            // cancelled while suspended: the body unwinds and its locals drop
            let c = Costack::new(Box::new(|| {
                let _t = Tracker;
                suspend();
            }))
            .unwrap();
            assert_eq!(c.resume(), Status::Suspended);
            c.cancel();
            assert_eq!(DROPS.with(|d| d.get()), 1);
            // never started: the captured Tracker drops with the box
            let t = Tracker;
            let c = Costack::new(Box::new(move || {
                let _t = t;
                suspend();
            }))
            .unwrap();
            drop(c);
            assert_eq!(DROPS.with(|d| d.get()), 2);
        }

        // the live counter is process-global: with other tests running costacks concurrently (the whole lib
        // suite under THAW_JS_CO=stack) the 30,000 fill hits the cap early, so this one runs alone (CI step).
        #[test]
        #[ignore = "global live counter: run alone with --ignored --test-threads=1"]
        fn costack_live_limit_is_a_clear_error() {
            let mut held = Vec::with_capacity(MAX_LIVE);
            for _ in 0..MAX_LIVE {
                held.push(Costack::with_stack_size(Box::new(|| {}), 256 << 10).unwrap());
            }
            let err = Costack::with_stack_size(Box::new(|| {}), 256 << 10)
                .err()
                .unwrap();
            assert!(matches!(err, CostackError::TooManyLive));
            assert_eq!(err.to_string(), "cannot start coroutine (thread limit)");
            drop(held);
            assert!(Costack::with_stack_size(Box::new(|| {}), 256 << 10).is_ok());
        }

        /// Host and body each set callee-saved registers, switch, and check their own values come back.
        #[cfg(target_arch = "x86_64")]
        #[test]
        fn costack_callee_saved_registers_survive_switches() {
            extern "C" fn suspend_thunk() {
                suspend();
            }
            extern "C" fn resume_thunk(c: *const Costack) -> u8 {
                // SAFETY: the test passes a live costack pointer.
                (unsafe { &*c }.resume() == Status::Finished) as u8
            }
            let seen = Rc::new(Cell::new([0u64; 4]));
            let s = seen.clone();
            let c = Costack::new(Box::new(move || {
                let (mut a, mut b, mut cc, mut d) = (0u64, 0u64, 0u64, 0u64);
                // SAFETY: calls a thunk that suspends; r12-r15 are callee-saved, so the thunk must keep them.
                unsafe {
                    core::arch::asm!(
                        "call {f}",
                        f = sym suspend_thunk,
                        inout("r12") 0x1111u64 => a,
                        inout("r13") 0x2222u64 => b,
                        inout("r14") 0x3333u64 => cc,
                        inout("r15") 0x4444u64 => d,
                        clobber_abi("C"),
                    );
                }
                s.set([a, b, cc, d]);
            }))
            .unwrap();
            // host side: its own values must survive the body's register use
            let mut finished = 0u8;
            let (mut ha, mut hb, mut hc, mut hd) = (0u64, 0u64, 0u64, 0u64);
            let ptr = &c as *const Costack;
            // SAFETY: calls a thunk that resumes `c`; r12-r15 are callee-saved, so they must come back.
            unsafe {
                core::arch::asm!(
                    "call {f}",
                    f = sym resume_thunk,
                    in("rdi") ptr,
                    inout("r12") 0xaaaau64 => ha,
                    inout("r13") 0xbbbbu64 => hb,
                    inout("r14") 0xccccu64 => hc,
                    inout("r15") 0xddddu64 => hd,
                    lateout("al") finished,
                    clobber_abi("C"),
                );
            }
            assert_eq!(finished, 0);
            assert_eq!([ha, hb, hc, hd], [0xaaaa, 0xbbbb, 0xcccc, 0xdddd]);
            assert_eq!(seen.get(), [0, 0, 0, 0]);
            assert_eq!(c.resume(), Status::Finished);
            assert_eq!(seen.get(), [0x1111, 0x2222, 0x3333, 0x4444]);
        }

        // aarch64: x19 cannot be an asm operand, so a small assembly helper sets x19-x22, calls the
        // suspend thunk and stores them back. The helper keeps its own frame 16-byte aligned.
        #[cfg(target_arch = "aarch64")]
        core::arch::global_asm!(
            ".globl thaw_costack_test_regs",
            ".p2align 4",
            "thaw_costack_test_regs:",
            "stp x29, x30, [sp, #-32]!",
            "mov x29, sp",
            "str x0, [sp, #16]",
            "mov x19, #0x1111",
            "mov x20, #0x2222",
            "mov x21, #0x3333",
            "mov x22, #0x4444",
            "bl {susp}",
            "ldr x0, [sp, #16]",
            "str x19, [x0, #0]",
            "str x20, [x0, #8]",
            "str x21, [x0, #16]",
            "str x22, [x0, #24]",
            "ldp x29, x30, [sp], #32",
            "ret",
            susp = sym aarch64_suspend_thunk,
        );

        #[cfg(target_arch = "aarch64")]
        extern "C" fn aarch64_suspend_thunk() {
            suspend();
        }

        #[cfg(target_arch = "aarch64")]
        extern "C" {
            fn thaw_costack_test_regs(out: *mut [u64; 4]);
        }

        #[cfg(target_arch = "aarch64")]
        #[test]
        fn costack_callee_saved_registers_survive_switches() {
            let seen = Rc::new(Cell::new([0u64; 4]));
            let s = seen.clone();
            let c = Costack::new(Box::new(move || {
                let mut out = [0u64; 4];
                // SAFETY: the helper only touches its own frame and x19-x22, and calls the suspend thunk.
                unsafe { thaw_costack_test_regs(&mut out) };
                s.set(out);
            }))
            .unwrap();
            assert_eq!(c.resume(), Status::Suspended);
            assert_eq!(c.resume(), Status::Finished);
            assert_eq!(seen.get(), [0x1111, 0x2222, 0x3333, 0x4444]);
        }

        /// Microbenchmark: ns per resume + suspend round trip. Run with `--ignored --nocapture`.
        #[test]
        #[ignore]
        fn costack_bench() {
            const N: u32 = 1_000_000;
            let c = Costack::new(Box::new(|| loop {
                suspend();
            }))
            .unwrap();
            for _ in 0..1000 {
                c.resume();
            }
            let t = Instant::now();
            for _ in 0..N {
                c.resume();
            }
            let ns = t.elapsed().as_nanos() as f64 / N as f64;
            println!("costack round trip (resume + suspend): {ns:.1} ns");
            c.cancel();
            if !cfg!(debug_assertions) {
                assert!(ns < 60.0, "round trip {ns:.1} ns");
            }
        }

        #[test]
        fn current_bounds_is_the_running_costack_and_none_on_the_host() {
            assert_eq!(current_bounds(), None);
            assert!(!in_coroutine());
            let seen = Rc::new(Cell::new(None));
            let s = seen.clone();
            let c = Costack::new(Box::new(move || {
                let marker = 0u8;
                let sp = &marker as *const u8 as usize;
                s.set(Some((current_bounds(), in_coroutine(), sp)));
                suspend();
                s.set(Some((current_bounds(), in_coroutine(), 0)));
            }))
            .unwrap();
            let want = c.bounds();
            assert_eq!(want.1 - want.0, STACK_BYTES - GUARD_BYTES);
            assert_eq!(c.resume(), Status::Suspended);
            let (got, inside, sp) = seen.get().unwrap();
            assert_eq!(got, Some(want));
            assert!(inside);
            assert!(want.0 < sp && sp < want.1, "sp {sp:#x} outside {want:x?}");
            assert_eq!(c.resume(), Status::Finished);
            assert_eq!(seen.get().unwrap().0, Some(want));
            assert_eq!(current_bounds(), None, "back on the host after the run");
            assert!(!in_coroutine());
        }

        #[test]
        fn current_bounds_follows_nested_resumes_and_returns_to_the_outer_costack() {
            type Log = Vec<(&'static str, Option<(usize, usize)>)>;
            let log: Rc<RefCell<Log>> = Rc::new(RefCell::new(Vec::new()));
            let l = log.clone();
            let outer = Costack::new(Box::new(move || {
                l.borrow_mut().push(("outer", current_bounds()));
                let l2 = l.clone();
                let inner = Costack::new(Box::new(move || {
                    l2.borrow_mut().push(("inner", current_bounds()));
                    suspend();
                    l2.borrow_mut().push(("inner-again", current_bounds()));
                }))
                .unwrap();
                let _scope = StackBoundsScope::enter(inner.bounds());
                assert_eq!(inner.resume(), Status::Suspended);
                drop(_scope);
                // back in the outer body: its own bounds again, not the inner's
                l.borrow_mut().push(("outer-after", current_bounds()));
                suspend();
                l.borrow_mut().push(("outer-end", current_bounds()));
            }))
            .unwrap();
            let outer_bounds = outer.bounds();
            assert_eq!(outer.resume(), Status::Suspended);
            assert_eq!(outer.resume(), Status::Finished);
            let log = log.borrow();
            let names: Vec<&str> = log.iter().map(|(n, _)| *n).collect();
            assert_eq!(names, ["outer", "inner", "outer-after", "outer-end"]);
            assert_eq!(log[0].1, Some(outer_bounds));
            assert_ne!(
                log[1].1,
                Some(outer_bounds),
                "the inner body sees its own stack"
            );
            assert_eq!(log[2].1, Some(outer_bounds));
            assert_eq!(log[3].1, Some(outer_bounds));
        }

        #[test]
        fn stack_bounds_scope_sets_the_guard_override_and_restores_it_on_drop() {
            let c = Costack::new(Box::new(|| {})).unwrap();
            let bounds = c.bounds();
            assert_eq!(
                thaw_arena::stack_guard::set_stack_bounds_override(None),
                None
            );
            {
                let _scope = StackBoundsScope::enter(bounds);
                // the scope installed `bounds`: setting it again returns exactly that
                let seen = thaw_arena::stack_guard::set_stack_bounds_override(Some((1, 2)));
                assert_eq!(seen, Some(bounds));
                thaw_arena::stack_guard::set_stack_bounds_override(Some(bounds));
            }
            assert_eq!(
                thaw_arena::stack_guard::set_stack_bounds_override(None),
                None
            );
            assert_eq!(
                c.resume(),
                Status::Finished,
                "a body that never ran finishes at once"
            );
        }

        /// `thread_stack_has_room` judges a costack by its own bounds, whatever the host thread's stack is:
        /// the threshold is `lo + min(size / 8 + 4 MiB, size / 2)`, i.e. 12 MiB above the bottom of the
        /// 64 MiB costack. Layout-independent, unlike the JS-level test below.
        #[test]
        fn has_room_on_a_costack_uses_the_costack_bounds() {
            let c = Costack::new(Box::new(|| {
                let (lo, _hi) = current_bounds().unwrap();
                let mib = 1usize << 20;
                assert!(
                    !crate::thread_stack_has_room(lo + mib),
                    "1 MiB above the bottom: no room"
                );
                assert!(
                    !crate::thread_stack_has_room(lo + 11 * mib),
                    "11 MiB above: no room"
                );
                assert!(
                    crate::thread_stack_has_room(lo + 13 * mib),
                    "13 MiB above: room"
                );
                let here = 0u8;
                assert!(crate::thread_stack_has_room(&here as *const u8 as usize));
            }))
            .unwrap();
            assert_eq!(c.resume(), Status::Finished);
        }

        /// Deep JS recursion inside a costack body must be a `RangeError`, not a fault on the guard page.
        /// The costack is mapped first and the resumer thread spawned after it, so the thread's own stack
        /// lies BELOW the costack: the host thread's bounds then say "room" for a sp on the costack, which
        /// is exactly what `thread_stack_has_room` must not believe. Ignored: CI runs it alone with `--ignored`
        /// in the thaw-js-costack job, where the costack backend is the default.
        #[test]
        #[ignore = "run alone with --ignored (CI: thaw-js-costack job)"]
        fn js_deep_recursion_on_a_costack_is_a_range_error() {
            let realm = Arc::new(AtomicUsize::new(0));
            let r2 = realm.clone();
            let c = Costack::new(Box::new(move || {
                let ip = r2.load(Ordering::SeqCst) as *mut crate::Interp;
                // SAFETY: the spawned thread stores its live Interp before it resumes this costack.
                let it = unsafe { &mut *ip };
                it.configure_stack(STACK_BYTES);
                let _ = it.run("function r(n) { return r(n + 1) + 1; } try { r(0); } catch (e) { console.log(e.name); }");
            }))
            .unwrap();
            let c_addr = &c as *const Costack as usize;
            let out = std::thread::Builder::new()
                .stack_size(8 << 20)
                .spawn(move || {
                    let mut it = Box::new(crate::Interp::new());
                    realm.store(&mut *it as *mut crate::Interp as usize, Ordering::SeqCst);
                    // SAFETY: `c` lives in the test frame, which joins this thread before it is dropped.
                    let c = unsafe { &*(c_addr as *const Costack) };
                    let _scope = StackBoundsScope::enter(c.bounds());
                    assert_eq!(c.resume(), Status::Finished);
                    it.out.clone()
                })
                .unwrap()
                .join()
                .unwrap();
            assert_eq!(out, ["RangeError"]);
        }
    }
}

#[cfg(all(
    unix,
    any(target_arch = "x86_64", target_arch = "aarch64"),
    target_os = "linux"
))]
#[allow(unused_imports)]
pub(crate) use imp::{
    current_bounds, in_coroutine, suspend, Costack, CostackError, Status, MAX_LIVE, STACK_BYTES,
};

/// Stub for targets without the assembly backend: construction always fails.
#[cfg(not(all(
    unix,
    any(target_arch = "x86_64", target_arch = "aarch64"),
    target_os = "linux"
)))]
mod stub {
    use std::fmt;

    pub(crate) const STACK_BYTES: usize = 64 << 20;

    #[derive(Debug)]
    pub(crate) enum CostackError {
        TooManyLive,
        Map(std::io::Error),
        Unsupported,
    }

    impl fmt::Display for CostackError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                CostackError::TooManyLive => write!(f, "cannot start coroutine (thread limit)"),
                CostackError::Map(e) => write!(f, "cannot map coroutine stack: {e}"),
                CostackError::Unsupported => {
                    write!(f, "stackful coroutines are not supported here")
                }
            }
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) enum Status {
        Suspended,
        Finished,
    }

    pub(crate) struct Costack(());

    impl Costack {
        pub(crate) fn new(body: Box<dyn FnOnce()>) -> Result<Costack, CostackError> {
            drop(body);
            Err(CostackError::Unsupported)
        }

        pub(crate) fn resume(&self) -> Status {
            Status::Finished
        }

        pub(crate) fn cancel(self) {}

        #[allow(dead_code)]
        pub(crate) fn bounds(&self) -> (usize, usize) {
            (0, 0)
        }
    }

    pub(crate) fn suspend() {
        unreachable!("suspend outside a stackful coroutine")
    }

    pub(crate) fn current_bounds() -> Option<(usize, usize)> {
        None
    }

    #[allow(dead_code)]
    pub(crate) fn in_coroutine() -> bool {
        false
    }
}

#[cfg(not(all(
    unix,
    any(target_arch = "x86_64", target_arch = "aarch64"),
    target_os = "linux"
)))]
#[allow(unused_imports)]
pub(crate) use stub::{
    current_bounds, in_coroutine, suspend, Costack, CostackError, Status, STACK_BYTES,
};
