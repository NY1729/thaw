//! `THAW_JS_PROFILE=1`: counts how often AOT code enters the interpreter and how long the outermost
//! entries take, printed to stderr as TSV at process exit:
//!
//! ```text
//! thaw-js-profile<TAB><key><TAB><calls><TAB><total_ms>        (sorted by total_ms, descending)
//! thaw-js-profile-total<TAB><interp_ms><TAB><wall_ms>         (wall since the first entry)
//! ```
//!
//! Keys: a typed call is `pkg::export`; any other entry through `host::with_package` is
//! `entry:<pkg>@<file>:<line>` (the call site, via `#[track_caller]`). Only the outermost entry on a
//! thread is counted: an entry made while another is running (a compiled callback re-entering the
//! interpreter) is inside its caller's time and is not a call of its own.
//!
//! Disabled (the default), an entry is one relaxed load and no lock. Enabled, the table is one
//! process-wide `Mutex` taken once per outermost exit (coroutine stacks and `THAW_JS_CO=thread` both
//! work without per-thread merging). Output is written once: by the `atexit` hook, or by
//! [`flush`] from the `_exit` paths in `host.rs`, which skip `atexit`.

use std::cell::Cell;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::Instant;

const UNKNOWN: u8 = 0;
const OFF: u8 = 1;
const ON: u8 = 2;
static STATE: AtomicU8 = AtomicU8::new(UNKNOWN);
static FLUSHED: AtomicBool = AtomicBool::new(false);
static FIRST_ENTRY: OnceLock<Instant> = OnceLock::new();
static TABLE: Mutex<Option<Table>> = Mutex::new(None);

thread_local! {
    /// Entries currently open on this thread (counted ones only).
    static DEPTH: Cell<u32> = const { Cell::new(0) };
}

/// `(calls, outermost nanoseconds)` per key.
#[derive(Default)]
pub(crate) struct Table {
    rows: std::collections::HashMap<String, (u64, u64)>,
}

impl Table {
    fn record(&mut self, key: &str, ns: u64) {
        // keys are TSV fields: a tab or newline inside one would break the format
        let key: String = key
            .chars()
            .map(|c| if c == '\t' || c == '\n' { ' ' } else { c })
            .collect();
        let row = self.rows.entry(key).or_insert((0, 0));
        row.0 += 1;
        row.1 += ns;
    }

    /// The TSV report: one row per key sorted by total time (descending), then the total line.
    fn render(&self, wall_ns: u64) -> String {
        let mut rows: Vec<(&String, &(u64, u64))> = self.rows.iter().collect();
        rows.sort_by(|a, b| b.1 .1.cmp(&a.1 .1).then_with(|| a.0.cmp(b.0)));
        let mut out = String::new();
        let mut interp_ns = 0;
        for (key, (calls, ns)) in rows {
            interp_ns += ns;
            let _ = writeln!(out, "thaw-js-profile\t{key}\t{calls}\t{:.3}", ms(*ns));
        }
        let _ = writeln!(
            out,
            "thaw-js-profile-total\t{:.3}\t{:.3}",
            ms(interp_ns),
            ms(wall_ns)
        );
        out
    }
}

fn ms(ns: u64) -> f64 {
    ns as f64 / 1_000_000.0
}

/// The typed-call key: `pkg::export`.
pub(crate) fn typed_key(package: &str, export: &str) -> String {
    format!("{package}::{export}")
}

/// The key of an entry through `with_package`: its package (when known) and call site.
pub(crate) fn entry_key(package: Option<&str>, file: &str, line: u32) -> String {
    let file = file.rsplit('/').next().unwrap_or(file);
    format!("entry:{}@{file}:{line}", package.unwrap_or("?"))
}

#[cold]
fn init() -> bool {
    let on = std::env::var_os("THAW_JS_PROFILE").is_some_and(|v| v == "1");
    if on {
        // SAFETY: `flush_at_exit` takes no arguments and is a valid `extern "C"` function.
        unsafe { libc::atexit(flush_at_exit) };
    }
    STATE.store(if on { ON } else { OFF }, Ordering::Relaxed);
    on
}

#[inline]
fn enabled() -> bool {
    match STATE.load(Ordering::Relaxed) {
        ON => true,
        OFF => false,
        _ => init(),
    }
}

extern "C" fn flush_at_exit() {
    flush();
}

/// Counts one entry: keeps the entry open until the returned guard drops. `key` is only evaluated
/// when the entry is counted (outermost and enabled).
pub(crate) fn enter(key: impl FnOnce() -> String) -> Scope {
    enter_if(enabled(), &TABLE, key)
}

/// [`enter`] with the switch and table given by the caller (the global ones in production).
fn enter_if(on: bool, table: &'static Mutex<Option<Table>>, key: impl FnOnce() -> String) -> Scope {
    if !on {
        return Scope {
            counted: false,
            outer: None,
        };
    }
    FIRST_ENTRY.get_or_init(Instant::now);
    let depth = DEPTH.with(|d| {
        let n = d.get();
        d.set(n + 1);
        n
    });
    let outer = (depth == 0).then(|| Outer {
        table,
        key: key(),
        start: Instant::now(),
    });
    Scope {
        counted: true,
        outer,
    }
}

/// An open entry; the outermost one records its key and elapsed time when dropped.
pub(crate) struct Scope {
    counted: bool,
    outer: Option<Outer>,
}

struct Outer {
    table: &'static Mutex<Option<Table>>,
    key: String,
    start: Instant,
}

impl Drop for Scope {
    fn drop(&mut self) {
        if !self.counted {
            return;
        }
        let depth = DEPTH.with(|d| {
            let n = d.get().saturating_sub(1);
            d.set(n);
            n
        });
        if depth != 0 {
            return;
        }
        if let Some(outer) = self.outer.take() {
            let ns = outer.start.elapsed().as_nanos() as u64;
            let mut guard = outer.table.lock().unwrap_or_else(PoisonError::into_inner);
            guard
                .get_or_insert_with(Table::default)
                .record(&outer.key, ns);
        }
    }
}

/// Writes the TSV report to stderr once (later calls and the `atexit` hook are no-ops). Called from
/// the `_exit` paths, which skip `atexit`.
pub(crate) fn flush() {
    if STATE.load(Ordering::Relaxed) != ON || FLUSHED.swap(true, Ordering::SeqCst) {
        return;
    }
    let wall_ns = FIRST_ENTRY
        .get()
        .map_or(0, |t| t.elapsed().as_nanos() as u64);
    let table = TABLE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take()
        .unwrap_or_default();
    let text = table.render(wall_ns);
    let _ = std::io::Write::write_all(&mut std::io::stderr(), text.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaked() -> &'static Mutex<Option<Table>> {
        Box::leak(Box::new(Mutex::new(None)))
    }

    fn rows(table: &'static Mutex<Option<Table>>) -> Vec<(String, u64)> {
        let guard = table.lock().unwrap();
        let mut rows: Vec<(String, u64)> = guard
            .as_ref()
            .map(|t| t.rows.iter().map(|(k, v)| (k.clone(), v.0)).collect())
            .unwrap_or_default();
        rows.sort();
        rows
    }

    #[test]
    fn nested_entries_count_only_the_outermost() {
        let table = leaked();
        let outer = enter_if(true, table, || "pkg::outer".into());
        {
            let inner = enter_if(true, table, || "pkg::inner".into());
            drop(inner);
        }
        drop(outer);
        assert_eq!(rows(table), vec![("pkg::outer".to_string(), 1)]);

        // two sibling outermost entries are two calls
        drop(enter_if(true, table, || "pkg::outer".into()));
        assert_eq!(rows(table), vec![("pkg::outer".to_string(), 2)]);
    }

    #[test]
    fn disabled_entries_record_nothing_and_do_not_evaluate_the_key() {
        let table = leaked();
        let outer = enter_if(false, table, || panic!("key must not be built when off"));
        drop(outer);
        assert!(rows(table).is_empty());
        assert_eq!(DEPTH.with(Cell::get), 0);
    }

    #[test]
    fn report_is_tsv_sorted_by_total_time() {
        let mut table = Table::default();
        table.record("a::slow", 3_000_000);
        table.record("b::fast", 1_000_000);
        table.record("b::fast", 500_000);
        table.record("tab\tkey", 2_000_000);
        let text = table.render(10_000_000);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines,
            vec![
                "thaw-js-profile\ta::slow\t1\t3.000",
                "thaw-js-profile\ttab key\t1\t2.000",
                "thaw-js-profile\tb::fast\t2\t1.500",
                "thaw-js-profile-total\t6.500\t10.000",
            ]
        );
    }

    #[test]
    fn key_names() {
        assert_eq!(
            typed_key("node:crypto", "createHash"),
            "node:crypto::createHash"
        );
        assert_eq!(
            entry_key(Some("yup"), "/work/crates/thaw-js/src/host.rs", 5016),
            "entry:yup@host.rs:5016"
        );
        assert_eq!(entry_key(None, "host.rs", 7), "entry:?@host.rs:7");
    }
}
