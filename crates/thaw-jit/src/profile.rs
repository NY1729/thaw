//! `THAW_JIT_PROFILE=1`: counts calls into JIT stubs per symbol (the program text given to
//! `thaw_jit_call_f64`), printed to stderr at process exit, most-called first:
//!
//! ```text
//! thaw-jit-profile<TAB><symbol><TAB><calls>
//! ```
//!
//! Disabled, a call costs one relaxed load. The report is written by an `atexit` hook only, so a
//! process that ends through `_exit` (thaw-js's fatal paths) loses it. No dependency beyond `libc`.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Mutex, PoisonError};

const UNKNOWN: u8 = 0;
const OFF: u8 = 1;
const ON: u8 = 2;
static STATE: AtomicU8 = AtomicU8::new(UNKNOWN);
static CALLS: Mutex<Option<HashMap<String, u64>>> = Mutex::new(None);

#[cold]
fn init() -> bool {
    let on = std::env::var_os("THAW_JIT_PROFILE").is_some_and(|v| v == "1");
    if on {
        // SAFETY: `flush_at_exit` takes no arguments and is a valid `extern "C"` function.
        unsafe { libc::atexit(flush_at_exit) };
    }
    STATE.store(if on { ON } else { OFF }, Ordering::Relaxed);
    on
}

/// Counts one call into the stub for `symbol`.
#[inline]
pub(crate) fn record(symbol: &str) {
    let on = match STATE.load(Ordering::Relaxed) {
        ON => true,
        OFF => false,
        _ => init(),
    };
    if on {
        record_enabled(symbol);
    }
}

#[cold]
fn record_enabled(symbol: &str) {
    let mut guard = CALLS.lock().unwrap_or_else(PoisonError::into_inner);
    let calls = guard.get_or_insert_with(HashMap::new);
    *calls.entry(symbol.to_string()).or_insert(0) += 1;
}

/// The TSV report: one row per symbol, most calls first (ties by symbol).
fn render(calls: &HashMap<String, u64>) -> String {
    let mut rows: Vec<(&String, &u64)> = calls.iter().collect();
    rows.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
    let mut out = String::new();
    for (symbol, count) in rows {
        // symbols are TSV fields: a tab or newline inside one would break the format
        let symbol: String = symbol
            .chars()
            .map(|c| if c == '\t' || c == '\n' { ' ' } else { c })
            .collect();
        let _ = writeln!(out, "thaw-jit-profile\t{symbol}\t{count}");
    }
    out
}

extern "C" fn flush_at_exit() {
    // taking the table makes a second call print nothing
    let calls = CALLS.lock().unwrap_or_else(PoisonError::into_inner).take();
    if let Some(calls) = calls {
        let _ = std::io::Write::write_all(&mut std::io::stderr(), render(&calls).as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_is_tsv_most_called_first() {
        let calls = HashMap::from([
            ("expr:a0,a1,add".to_string(), 2),
            ("expr:a0,tab\tsymbol".to_string(), 5),
            ("expr:a0,ld".to_string(), 2),
        ]);
        assert_eq!(
            render(&calls),
            "thaw-jit-profile\texpr:a0,tab symbol\t5\n\
             thaw-jit-profile\texpr:a0,a1,add\t2\n\
             thaw-jit-profile\texpr:a0,ld\t2\n"
        );
    }
}
