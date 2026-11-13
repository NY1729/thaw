// Native timer queue (runtime/timers.rs).

thread_local! {
    static TIMER_LOG: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
}

extern "C" fn log_timer(context: *mut u8) {
    TIMER_LOG.with(|log| log.borrow_mut().push(context as usize));
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
    TIMER_LOG.with(|log| log.borrow_mut().clear());
    thaw_timer_set(TIMER_TIMEOUT, 6.0, log_timer, std::ptr::dangling_mut::<u8>());
    thaw_timer_set(TIMER_TIMEOUT, 0.0, log_timer, 2 as *mut u8);
    thaw_timer_set(TIMER_TIMEOUT, 1.0, log_timer, 3 as *mut u8);
    thaw_timer_set(TIMER_IMMEDIATE, 0.0, log_timer, 4 as *mut u8);
    drain_timers_for(40);
    // The first poll starts in the timers phase: nothing is due yet, so the immediate runs first.
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
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(30) {}
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
    // the first interval run busy-waits 30 ms: its next run is due 10 ms after it started, so it
    // runs before the 30 ms timer (Node prints iv1 iv2 t30); re-armed from the clock after the
    // callback it would come after t30
    let id = thaw_timer_set(TIMER_INTERVAL, 10.0, interval_rearm_run, std::ptr::null_mut());
    INTERVAL_ID.with(|slot| slot.set(id));
    thaw_timer_set(TIMER_TIMEOUT, 30.0, interval_rearm_t30, std::ptr::null_mut());
    while thaw_runtime_run_one_event() {}
    INTERVAL_RUNS.with(|runs| assert_eq!(runs.borrow().join("|"), "iv1|iv2|t30"));
}
