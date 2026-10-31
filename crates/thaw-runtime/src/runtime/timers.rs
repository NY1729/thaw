// Native `setTimeout`/`setInterval`/`setImmediate` queue for programs that never create a JS
// engine. Callbacks run from `thaw_runtime_poll_one` (after ready continuations, so promise
// jobs drain between callbacks) in Node's phase order: due timers by (due, seq), then the
// immediates queued before the check phase began.

type NativeTimerFn = extern "C" fn(*mut u8);

const TIMER_TIMEOUT: u8 = 0;
const TIMER_INTERVAL: u8 = 1;
const TIMER_IMMEDIATE: u8 = 2;
/// Node's `TIMEOUT_MAX`: larger or non-finite delays become 1 ms.
const TIMER_MAX_MS: f64 = 2_147_483_647.0;

struct NativeTimer {
    id: u64,
    kind: u8,
    /// Repeat/delay in ms (already normalised).
    delay: f64,
    /// Due time in loop milliseconds (`timer_now_ms`); unused by immediates.
    due: f64,
    seq: u64,
    refed: bool,
    callback: NativeTimerFn,
    context: *mut u8,
    _root: thaw_arena::ArenaRoot,
}

#[derive(Clone, Copy, PartialEq)]
enum TimerPhase {
    /// Running timers due at the phase's snapshot of the clock.
    Timers,
    /// Running immediates queued before this sequence number.
    Check(u64),
}

thread_local! {
    static NATIVE_TIMERS: RefCell<Vec<NativeTimer>> = const { RefCell::new(Vec::new()) };
    static NEXT_TIMER_ID: Cell<u64> = const { Cell::new(1) };
    static TIMER_PHASE: Cell<(TimerPhase, f64)> = const { Cell::new((TimerPhase::Timers, -1.0)) };
}

fn timer_now_ms() -> f64 {
    let epoch = *PROCESS_START.get_or_init(Instant::now);
    epoch.elapsed().as_millis() as f64
}

fn timer_delay_ms(delay: f64) -> f64 {
    if delay >= 1.0 && delay <= TIMER_MAX_MS {
        delay
    } else {
        1.0
    }
}

/// Registers a timer and returns its id (never 0). `kind` is 0 timeout, 1 interval, 2 immediate.
#[no_mangle]
pub extern "C" fn thaw_timer_set(
    kind: u8,
    delay_ms: f64,
    callback: NativeTimerFn,
    context: *mut u8,
) -> u64 {
    let id = NEXT_TIMER_ID.with(|next| {
        let id = next.get();
        next.set(id + 1);
        id
    });
    let delay = timer_delay_ms(delay_ms);
    let seq = id;
    NATIVE_TIMERS.with(|timers| {
        timers.borrow_mut().push(NativeTimer {
            id,
            kind,
            delay,
            due: timer_now_ms() + delay,
            seq,
            refed: true,
            callback,
            context,
            _root: thaw_arena::ArenaRoot::new(context as usize),
        });
    });
    id
}

fn with_timer<R>(id: u64, f: impl FnOnce(&mut NativeTimer) -> R) -> Option<R> {
    NATIVE_TIMERS.with(|timers| timers.borrow_mut().iter_mut().find(|t| t.id == id).map(f))
}

#[no_mangle]
pub extern "C" fn thaw_timer_clear(id: u64) {
    NATIVE_TIMERS.with(|timers| timers.borrow_mut().retain(|timer| timer.id != id));
}

#[no_mangle]
pub extern "C" fn thaw_timer_ref(id: u64) {
    with_timer(id, |timer| timer.refed = true);
}

#[no_mangle]
pub extern "C" fn thaw_timer_unref(id: u64) {
    with_timer(id, |timer| timer.refed = false);
}

#[no_mangle]
pub extern "C" fn thaw_timer_has_ref(id: u64) -> u8 {
    with_timer(id, |timer| u8::from(timer.refed)).unwrap_or(0)
}

/// `timeout.refresh()`: restart the countdown from now (a fired one-shot timer stays gone).
#[no_mangle]
pub extern "C" fn thaw_timer_refresh(id: u64) {
    let now = timer_now_ms();
    with_timer(id, |timer| {
        if timer.kind != TIMER_IMMEDIATE {
            timer.due = now + timer.delay;
        }
    });
}

/// A ref'd timer or immediate keeps the process alive.
fn native_timers_keep_alive() -> bool {
    NATIVE_TIMERS.with(|timers| timers.borrow().iter().any(|timer| timer.refed))
}

fn native_timers_pending() -> bool {
    NATIVE_TIMERS.with(|timers| !timers.borrow().is_empty())
}

/// Time until the earliest timer is due (zero for an immediate).
fn next_native_timer_delay() -> Option<Duration> {
    let now = timer_now_ms();
    NATIVE_TIMERS.with(|timers| {
        timers
            .borrow()
            .iter()
            .map(|timer| {
                if timer.kind == TIMER_IMMEDIATE {
                    Duration::ZERO
                } else {
                    Duration::from_secs_f64(((timer.due - now).max(0.0)) / 1000.0)
                }
            })
            .min()
    })
}

/// Runs the next ready timer or immediate; false when nothing can run now.
fn run_one_native_timer() -> bool {
    if !native_timers_pending() {
        return false;
    }
    // Two phase flips at most: Timers -> Check -> Timers (a fresh snapshot) finding nothing.
    for _ in 0..3 {
        let (phase, mut snapshot) = TIMER_PHASE.with(Cell::get);
        if snapshot < 0.0 {
            snapshot = timer_now_ms();
            TIMER_PHASE.with(|slot| slot.set((phase, snapshot)));
        }
        let picked = NATIVE_TIMERS.with(|timers| {
            let timers = timers.borrow();
            match phase {
                TimerPhase::Timers => timers
                    .iter()
                    .filter(|t| t.kind != TIMER_IMMEDIATE && t.due <= snapshot)
                    .min_by(|a, b| a.due.total_cmp(&b.due).then(a.seq.cmp(&b.seq))),
                TimerPhase::Check(limit) => timers
                    .iter()
                    .filter(|t| t.kind == TIMER_IMMEDIATE && t.seq < limit)
                    .min_by_key(|t| t.seq),
            }
            .map(|t| (t.id, t.kind, t.callback, t.context))
        });
        let Some((id, kind, callback, context)) = picked else {
            let next = match phase {
                TimerPhase::Timers => (TimerPhase::Check(NEXT_TIMER_ID.with(Cell::get)), snapshot),
                TimerPhase::Check(_) => (TimerPhase::Timers, timer_now_ms()),
            };
            TIMER_PHASE.with(|slot| slot.set(next));
            continue;
        };
        if kind == TIMER_IMMEDIATE {
            thaw_timer_clear(id);
        } else if kind == TIMER_TIMEOUT {
            // A one-shot is gone before its callback runs; `hasRef` of it is false inside.
            thaw_timer_clear(id);
        }
        callback(context);
        if kind == TIMER_INTERVAL {
            let now = timer_now_ms();
            // Still registered (not cleared by its own callback): schedule the next run.
            with_timer(id, |timer| {
                timer.due = now + timer.delay;
                timer.seq = NEXT_TIMER_ID.with(|next| {
                    let seq = next.get();
                    next.set(seq + 1);
                    seq
                });
            });
        }
        return true;
    }
    false
}

/// Drains ready continuations and timers until neither has work ready now.
#[no_mangle]
pub extern "C" fn thaw_timer_run_until_idle() -> usize {
    thaw_runtime_run_until_idle()
}
