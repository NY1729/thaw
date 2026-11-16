// Native timer queue (runtime/timers.rs).

thread_local! {
    static TIMER_LOG: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
}

extern "C" fn log_timer(context: *mut u8) {
    TIMER_LOG.with(|log| log.borrow_mut().push(context as usize));
}

fn set_fake_now(now: f64) {
    TEST_CLOCK.with(|clock| clock.set(Some(now)));
}

/// Pins the timer clock to a fake value and clears the native timer state. Dropping it (also on
/// panic) restores the real clock and clears the state again, so no test leaks into the next.
struct FakeClock;

impl FakeClock {
    fn install(now: f64) -> Self {
        reset_native_timer_state();
        set_fake_now(now);
        FakeClock
    }

    fn set(&self, now: f64) {
        set_fake_now(now);
    }
}

impl Drop for FakeClock {
    fn drop(&mut self) {
        TEST_CLOCK.with(|clock| clock.set(None));
        reset_native_timer_state();
    }
}

fn reset_native_timer_state() {
    NATIVE_TIMERS.with(|timers| timers.borrow_mut().clear());
    TIMER_PHASE.with(|slot| slot.set((TimerPhase::Timers, -1.0)));
}

/// Runs everything ready at the current fake time; nothing advances the clock on its own.
fn run_ready_timers() {
    while thaw_runtime_poll_one() != 0 {}
}

fn drain_timers_for(ms: u64) {
    let end = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < end {
        thaw_runtime_run_until_idle();
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn native_timers_run_by_due_time_then_creation_and_immediates_after_timers_phase() {
    let clock = FakeClock::install(0.0);
    TIMER_LOG.with(|log| log.borrow_mut().clear());
    thaw_timer_set(
        TIMER_TIMEOUT,
        6.0,
        log_timer,
        std::ptr::dangling_mut::<u8>(),
    );
    thaw_timer_set(TIMER_TIMEOUT, 0.0, log_timer, 2 as *mut u8);
    thaw_timer_set(TIMER_TIMEOUT, 1.0, log_timer, 3 as *mut u8);
    thaw_timer_set(TIMER_IMMEDIATE, 0.0, log_timer, 4 as *mut u8);
    // Clock 0: timers due at 1 and 6 are not due, so the immediate runs first.
    run_ready_timers();
    assert_eq!(TIMER_LOG.with(|log| log.borrow().clone()), vec![4]);
    // Clock 1: the two timers due at 1 run in creation order.
    clock.set(1.0);
    run_ready_timers();
    assert_eq!(TIMER_LOG.with(|log| log.borrow().clone()), vec![4, 2, 3]);
    // Clock 6: the timer due at 6 runs last.
    clock.set(6.0);
    run_ready_timers();
    assert_eq!(TIMER_LOG.with(|log| log.borrow().clone()), vec![4, 2, 3, 1]);
    assert!(!native_timers_pending());
}

#[test]
fn unref_timer_does_not_keep_the_loop_alive_and_interval_repeats_until_cleared() {
    TIMER_LOG.with(|log| log.borrow_mut().clear());
    let quiet = thaw_timer_set(TIMER_TIMEOUT, 1000.0, log_timer, 9 as *mut u8);
    thaw_timer_unref(quiet);
    assert_eq!(thaw_timer_has_ref(quiet), 0);
    assert_eq!(thaw_runtime_async_work_pending(), 0);
    thaw_timer_ref(quiet);
    assert_eq!(thaw_runtime_async_work_pending(), 1);
    thaw_timer_clear(quiet);
    let interval = thaw_timer_set(TIMER_INTERVAL, 2.0, log_timer, 5 as *mut u8);
    drain_timers_for(30);
    thaw_timer_clear(interval);
    let count = TIMER_LOG.with(|log| log.borrow().len());
    assert!((3..=15).contains(&count), "interval ran {count} times");
    drain_timers_for(10);
    assert_eq!(TIMER_LOG.with(|log| log.borrow().len()), count);
    assert_eq!(thaw_runtime_async_work_pending(), 0);
}

thread_local! {
    static INTERVAL_RUNS: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
    static INTERVAL_COUNT: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    static INTERVAL_ID: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

extern "C" fn interval_rearm_run(_: *mut u8) {
    let n = INTERVAL_COUNT.with(|count| {
        count.set(count.get() + 1);
        count.get()
    });
    INTERVAL_RUNS.with(|runs| runs.borrow_mut().push(format!("iv{n}")));
    if n == 1 {
        // Stands in for a 30 ms busy-wait: the clock is at 30 when the callback returns.
        set_fake_now(30.0);
    }
    if n == 2 {
        thaw_timer_clear(INTERVAL_ID.with(std::cell::Cell::get));
    }
}

extern "C" fn interval_rearm_t30(_: *mut u8) {
    INTERVAL_RUNS.with(|runs| runs.borrow_mut().push("t30".into()));
}

#[test]
fn interval_rearm_is_based_on_the_clock_before_its_callback() {
    // the first interval run takes 30 ms (the clock jumps to 30 inside it): its next run is due 10 ms
    // after it started, so it runs before the 30 ms timer (Node prints iv1 iv2 t30); re-armed from the
    // clock after the callback it would come after t30
    let clock = FakeClock::install(0.0);
    INTERVAL_RUNS.with(|runs| runs.borrow_mut().clear());
    INTERVAL_COUNT.with(|count| count.set(0));
    let id = thaw_timer_set(
        TIMER_INTERVAL,
        10.0,
        interval_rearm_run,
        std::ptr::null_mut(),
    );
    INTERVAL_ID.with(|slot| slot.set(id));
    thaw_timer_set(
        TIMER_TIMEOUT,
        30.0,
        interval_rearm_t30,
        std::ptr::null_mut(),
    );
    clock.set(10.0);
    run_ready_timers();
    INTERVAL_RUNS.with(|runs| assert_eq!(runs.borrow().join("|"), "iv1|iv2|t30"));
}
