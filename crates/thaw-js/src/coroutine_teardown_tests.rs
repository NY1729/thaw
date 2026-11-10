// Dropping or moving a realm that still has suspended coroutines (generators, async functions,
// async generators). Each realm lives on its own thread. A dropped realm wakes its coroutines
// with a disconnected channel, which must park them without touching the Interp (see
// `park_disconnected`).

use super::*;
use crate::eventloop::DISCONNECT_PARKS;
use std::sync::atomic::Ordering;

const SUSPENDED: &str = r#"
function* g() { yield 1; yield 2 }
const gi = g();
console.log(gi.next().value);
async function f() { await new Promise(() => {}); console.log('never'); }
f();
async function* ag() { yield 5; yield 6 }
const ai = ag();
ai.next().then(r => console.log('agen', r.value));
"#;

fn passthrough(it: Interp) -> Interp {
    it
}

/// Run `SUSPENDED` in a fresh realm, check the output, then drop it (or move it first).
fn one_realm(moved: bool) {
    let mut it = Interp::new();
    assert!(it.run(SUSPENDED).is_ok());
    assert!(it.run_event_loop().is_ok());
    assert_eq!(it.out, ["1", "agen 5"]);
    let it = if moved { passthrough(it) } else { it };
    drop(it);
}

#[test]
fn suspended_coroutines_survive_realm_drop_across_threads() {
    let before = DISCONNECT_PARKS.load(Ordering::SeqCst);
    std::thread::scope(|s| {
        for t in 0..4usize {
            std::thread::Builder::new()
                .stack_size(64 << 20)
                .spawn_scoped(s, move || {
                    for i in 0..15 {
                        one_realm((t + i) % 2 == 1);
                    }
                })
                .unwrap();
        }
    });
    assert!(DISCONNECT_PARKS.load(Ordering::SeqCst) > before);
}
