// The costack backend's live-coroutine cap as JS sees it (`costack::MAX_LIVE`), and coroutines suspended
// across an arena reset. The cap tests are `#[ignore]` because the live counter is process-global: a
// 30,000-deep run would starve other tests creating coroutines in parallel. CI runs them alone
// (`--ignored costack_live_limit --test-threads=1`, the thaw-js-costack job). Both return early unless the
// backend is the costack one (`THAW_JS_CO=stack`, or the default once an addon is registered): the thread
// backend cannot hold 30,000 coroutines on a typical host.

use super::*;

/// `n` async calls that each await one shared gate, then the gate opens. Each call's result is `i`.
/// Under the cap every call succeeds; past it a call throws synchronously (`RangeError`), which the loop
/// records. The output lines are: pending count, throws count, first throw, then the settled results.
#[cfg(all(
    unix,
    any(target_arch = "x86_64", target_arch = "aarch64"),
    target_os = "linux"
))]
fn pending_script(n: usize) -> String {
    format!(
        r#"
let release;
const gate = new Promise(r => {{ release = r; }});
async function f(i) {{ await gate; return i; }}
const ps = [];
let threw = 0, first = '';
for (let i = 0; i < {n}; i++) {{
  try {{ ps.push(f(i)); }} catch (e) {{ threw++; if (!first) first = e.name + ': ' + e.message; }}
}}
console.log(ps.length, threw, first);
release();
Promise.all(ps).then(v => console.log('settled', v.length, v[0], v[v.length - 1], v.every((x, i) => x === i)));
"#
    )
}

/// Runs `pending_script(n)` to completion and returns the console lines.
#[cfg(all(
    unix,
    any(target_arch = "x86_64", target_arch = "aarch64"),
    target_os = "linux"
))]
fn run_pending(n: usize) -> Vec<String> {
    let mut it = Interp::new();
    assert!(it.run(&pending_script(n)).is_ok());
    assert!(it.run_event_loop().is_ok());
    it.out.clone()
}

/// 35,000 pending: the first `MAX_LIVE` calls are live, the other 5,000 throw the cap's RangeError.
#[cfg(all(
    unix,
    any(target_arch = "x86_64", target_arch = "aarch64"),
    target_os = "linux"
))]
#[test]
#[ignore = "global live counter: run alone with --ignored --test-threads=1"]
fn js_costack_live_limit_35k_pending_throws_range_error_past_the_cap() {
    if crate::co::backend() != crate::co::Backend::Stack {
        return;
    }
    let total = crate::costack::MAX_LIVE + 5_000;
    let out = run_pending(total);
    assert_eq!(
        out,
        [
            format!(
                "{} {} RangeError: cannot start coroutine (thread limit)",
                crate::costack::MAX_LIVE,
                5_000
            ),
            format!(
                "settled {} 0 {} true",
                crate::costack::MAX_LIVE,
                crate::costack::MAX_LIVE - 1
            ),
        ]
    );
}

/// 25,000 pending (under the cap, about 5/6 of it): all complete, in order, with the right results.
#[cfg(all(
    unix,
    any(target_arch = "x86_64", target_arch = "aarch64"),
    target_os = "linux"
))]
#[test]
#[ignore = "global live counter: run alone with --ignored --test-threads=1"]
fn js_costack_live_limit_25k_pending_completes_with_right_results() {
    if crate::co::backend() != crate::co::Backend::Stack {
        return;
    }
    let out = run_pending(25_000);
    assert_eq!(
        out,
        [
            "25000 0 ".to_string(),
            "settled 25000 0 24999 true".to_string(),
        ]
    );
}

/// JS values held by a suspended generator and async function survive an arena reset while they are
/// suspended. Thaw-js values are Rust-owned (`Rc`, not arena memory), so a reset must not touch them;
/// the reset is the same call the Lambda invocation boundary makes (thaw-runtime lambda.rs, `InvocationArenaReset`).
/// Runs with tracing off and on: on, the reset takes its traced path and the `ArenaRoot`s are live.
fn suspend_reset_resume(tracing: bool) -> Vec<String> {
    if tracing {
        thaw_arena::thaw_arena_enable_tracing();
    }
    let mut it = Interp::new();
    assert!(it
        .run(
            r#"
const gate = new Promise(r => { globalThis.open = r; });
function* g() { const s = 'gen-' + 'x'.repeat(40); yield s; yield [1, 2, 3].concat([new Uint8Array([7, 8]).length]); }
const gi = g();
console.log(gi.next().value.length);
async function f() {
  const s = 'async-' + 'y'.repeat(40);
  const a = [s, { k: 1 }];
  const t = new Uint8Array([4, 5, 6]);
  await gate;
  console.log(s.length, a[0] === s, a[1].k, t[2], a.length);
  return s;
}
f().then(v => console.log('done', v.slice(0, 6)));
"#
        )
        .is_ok());
    thaw_arena::thaw_arena_reset();
    assert!(it
        .run("console.log(gi.next().value.join(','), typeof open);open();")
        .is_ok());
    assert!(it.run_event_loop().is_ok());
    it.out.clone()
}

#[test]
fn suspended_coroutines_keep_their_values_across_an_arena_reset() {
    assert_eq!(
        suspend_reset_resume(false),
        ["44", "1,2,3,2 function", "46 true 1 6 2", "done async-"]
            .map(String::from)
            .to_vec()
    );
}

#[test]
fn suspended_coroutines_keep_their_values_across_a_traced_arena_reset() {
    assert_eq!(
        suspend_reset_resume(true),
        ["44", "1,2,3,2 function", "46 true 1 6 2", "done async-"]
            .map(String::from)
            .to_vec()
    );
}
