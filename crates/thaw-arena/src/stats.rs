//! Allocation counters and `THAW_ARENA_STATS` / `THAW_ARENA_STRESS` / `THAW_ARENA_POISON`.
//!
//! Counters are thread-local `Cell`s bumped with plain loads and stores (no atomics). Other
//! crates add their own lines through `register_stats_provider`, so no dependency cycle.

use std::cell::Cell;
use std::sync::{Mutex, OnceLock};

thread_local! {
    pub(crate) static ALLOCS: Cell<u64> = const { Cell::new(0) };
    pub(crate) static BYTES_TOTAL: Cell<u64> = const { Cell::new(0) };
    pub(crate) static RESETS: Cell<u64> = const { Cell::new(0) };
    pub(crate) static RECLAIMED_BYTES: Cell<u64> = const { Cell::new(0) };
    pub(crate) static LIVE_AFTER_RESET: Cell<u64> = const { Cell::new(0) };
    pub(crate) static PEAK_PENDING: Cell<u64> = const { Cell::new(0) };
    pub(crate) static PEAK_BYTES: Cell<u64> = const { Cell::new(0) };
    pub(crate) static RESET_NS: Cell<u64> = const { Cell::new(0) };
    pub(crate) static HOOK_NS: Cell<u64> = const { Cell::new(0) };
    /// Nanoseconds of the mark phase and of the sweep (record walk plus generation release).
    pub(crate) static MARK_NS: Cell<u64> = const { Cell::new(0) };
    pub(crate) static SWEEP_NS: Cell<u64> = const { Cell::new(0) };
    pub(crate) static CHUNKS: Cell<u64> = const { Cell::new(0) };
    static STRESS_TICKS: Cell<u64> = const { Cell::new(0) };
}

struct Config {
    stats: bool,
    stress: u64,
    poison: bool,
}

fn config() -> &'static Config {
    static CONFIG: OnceLock<Config> = OnceLock::new();
    CONFIG.get_or_init(|| {
        let flag = |name: &str| std::env::var(name).is_ok_and(|v| !v.is_empty() && v != "0");
        Config {
            stats: flag("THAW_ARENA_STATS"),
            stress: std::env::var("THAW_ARENA_STRESS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
            poison: flag("THAW_ARENA_POISON"),
        }
    })
}

/// True when `THAW_ARENA_STATS` is set; providers register only then.
pub fn stats_enabled() -> bool {
    config().stats
}

pub(crate) fn poison_enabled() -> bool {
    config().poison
}

/// Test-only: with `THAW_ARENA_STRESS=N`, true on every Nth call (callers force a reset).
pub(crate) fn stress_due() -> bool {
    let n = config().stress;
    n != 0
        && STRESS_TICKS.with(|t| {
            t.set(t.get() + 1);
            t.get() % n == 0
        })
}

/// Collects `category key=value ...` lines from stats providers.
#[derive(Default)]
pub struct StatsSink {
    lines: Vec<(String, Vec<(String, u64)>)>,
}

impl StatsSink {
    pub fn add(&mut self, category: &str, key: &str, value: u64) {
        match self.lines.iter_mut().find(|(c, _)| c == category) {
            Some((_, fields)) => fields.push((key.to_string(), value)),
            None => self
                .lines
                .push((category.to_string(), vec![(key.to_string(), value)])),
        }
    }

    /// One `thaw-arena <category>: k=v ...` line per category.
    pub fn render(&self) -> String {
        self.lines
            .iter()
            .map(|(category, fields)| {
                let fields: Vec<String> = fields.iter().map(|(k, v)| format!("{k}={v}")).collect();
                format!("thaw-arena {category}: {}\n", fields.join(" "))
            })
            .collect()
    }
}

static PROVIDERS: Mutex<Vec<fn(&mut StatsSink)>> = Mutex::new(Vec::new());

/// Registers a callback that adds this crate's counters to the stats summary. Providers run on
/// the thread that prints, so they should read thread-local state with `try_with`.
pub fn register_stats_provider(provider: fn(&mut StatsSink)) {
    let mut providers = PROVIDERS.lock().unwrap_or_else(|e| e.into_inner());
    if !providers.contains(&provider) {
        providers.push(provider);
    }
}

#[repr(C)]
struct Mallinfo2 {
    fields: [usize; 10],
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn malloc_in_use() -> Option<u64> {
    extern "C" {
        fn mallinfo2() -> Mallinfo2;
    }
    // fields: arena, ordblks, smblks, hblks, hblkhd, usmblks, fsmblks, uordblks, fordblks, keepcost
    let info = unsafe { mallinfo2() };
    Some((info.fields[7] + info.fields[4]) as u64)
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
fn malloc_in_use() -> Option<u64> {
    None
}

/// Builds the summary: arena counters, the malloc-vs-arena split, then provider lines.
pub fn stats_summary() -> String {
    let mut sink = StatsSink::default();
    let get = |c: &'static std::thread::LocalKey<Cell<u64>>| c.try_with(Cell::get).unwrap_or(0);
    let since = crate::bytes_since_reset() as u64;
    let live = get(&LIVE_AFTER_RESET);
    sink.add("arena", "allocs", get(&ALLOCS));
    sink.add("arena", "bytes", get(&BYTES_TOTAL));
    sink.add("arena", "live_after_reset", live);
    sink.add("arena", "since_reset", since);
    sink.add("arena", "peak", get(&PEAK_BYTES).max(live + since));
    sink.add("arena", "resets", get(&RESETS));
    sink.add("arena", "reclaimed", get(&RECLAIMED_BYTES));
    sink.add("arena", "reset_ms", get(&RESET_NS) / 1_000_000);
    sink.add("arena", "hook_ms", get(&HOOK_NS) / 1_000_000);
    sink.add("arena", "mark_ms", get(&MARK_NS) / 1_000_000);
    sink.add("arena", "sweep_ms", get(&SWEEP_NS) / 1_000_000);
    sink.add("arena", "chunks", crate::chunk_count() as u64);
    sink.add("arena", "peak_pending", get(&PEAK_PENDING));
    sink.add("arena", "tracing", u64::from(crate::is_tracing()));
    sink.add("heap", "arena_bytes", live + since);
    if let Some(malloc) = malloc_in_use() {
        sink.add("heap", "malloc_in_use", malloc);
    }
    let providers = PROVIDERS.lock().unwrap_or_else(|e| e.into_inner()).clone();
    for provider in providers {
        provider(&mut sink);
    }
    sink.render()
}

/// Prints the summary to stderr now (also called at process exit when enabled).
#[no_mangle]
pub extern "C" fn thaw_arena_print_stats() {
    eprint!("{}", stats_summary());
}

extern "C" fn print_at_exit() {
    thaw_arena_print_stats();
}

/// Called once when the first arena is created: arms the exit summary if requested.
pub(crate) fn init() {
    if stats_enabled() {
        extern "C" {
            fn atexit(callback: extern "C" fn()) -> i32;
        }
        static ARMED: std::sync::Once = std::sync::Once::new();
        ARMED.call_once(|| unsafe {
            atexit(print_at_exit);
        });
    }
}
