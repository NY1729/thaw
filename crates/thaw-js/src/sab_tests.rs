//! SharedArrayBuffer storage and sharing (plan A1/A2): one block that every Interp holding it reads and writes; it
//! crosses threads only as a `{t:'SAB', id}` wire node, and `structuredClone` on one thread returns the same block.
//! Each thread runs its own Interp (Interp is not Send), so every test builds its Interp inside its thread.

use super::*;
use crate::typed::{new_sab_block, sab_store, SabBlock};
use std::sync::Arc;

/// Runs `f` on a thread with a worker-sized stack (every Interp is built inside it).
fn in_thread<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(f)
        .expect("thread spawns")
        .join()
        .expect("thread joins")
}

/// Runs `src` on a fresh Interp (with `wire` declared as `__wire_in` when given) and returns its console lines.
fn lines(src: &str, wire: Option<&str>) -> Vec<String> {
    let mut it = Interp::new();
    if let Some(wire) = wire {
        it.global.declare("__wire_in", Value::str(wire), true);
    }
    match it.run(src) {
        Ok(_) => {}
        Err(Ctl::Throw(v)) => panic!("uncaught {}", it.to_str(&v)),
        Err(_) => panic!("stray completion"),
    }
    std::mem::take(&mut it.out)
}

#[test]
fn shared_buffer_views_over_one_block_see_each_others_writes() {
    let src =
        "const b = new SharedArrayBuffer(8); new Uint8Array(b)[3] = 9; const w = new Uint8Array(b);
      console.log(w[3], b.byteLength, b.slice(2, 5).byteLength, new Uint8Array(b.slice(0))[3]);";
    assert_eq!(lines(src, None), ["9 8 3 9"]);
}

#[test]
fn structured_clone_of_a_shared_buffer_returns_the_same_block() {
    let src = "const a = new SharedArrayBuffer(4); const b = structuredClone(a); new Uint8Array(a)[0] = 3;
      const v = structuredClone(new Uint8Array(a)); v[1] = 4;
      console.log(new Uint8Array(b)[0], b instanceof SharedArrayBuffer, b === a, new Uint8Array(a)[1], v.buffer instanceof SharedArrayBuffer);";
    assert_eq!(lines(src, None), ["3 true false 4 true"]);
}

#[test]
fn shared_buffer_crosses_threads_as_a_wire_node_and_comes_back_shared() {
    // thread A makes the block and writes byte 0; thread B (only the wire string crosses) reads it, writes byte 1
    // and sends the block back; the test thread reads both writes. Ordering is the join between the threads.
    let wire = in_thread(|| {
        let src = "const s = new SharedArrayBuffer(8); new Uint8Array(s)[0] = 5;
          console.log(__thaw_worker_encode({ s, view: new Uint8Array(s, 4, 2) }));";
        lines(src, None).remove(0)
    });
    let back = in_thread(move || {
        let src = "const g = __thaw_worker_decode(__wire_in);
          console.log(g.s instanceof SharedArrayBuffer, new Uint8Array(g.s)[0], g.view.byteOffset, g.view.length);
          new Uint8Array(g.s)[1] = 6;
          console.log(__thaw_worker_encode(g.s));";
        lines(src, Some(wire.as_str())).remove(1)
    });
    let src = "const g = __thaw_worker_decode(__wire_in);
      console.log(new Uint8Array(g)[0], new Uint8Array(g)[1], g.byteLength);";
    assert_eq!(lines(src, Some(back.as_str())), ["5 6 8"]);
}

#[test]
fn shared_buffer_views_on_the_wire_keep_their_window() {
    let src = "const s = new SharedArrayBuffer(8); const v = new Int32Array(s, 4, 1); v[0] = 7;
      const g = __thaw_worker_decode(__thaw_worker_encode({ v }));
      console.log(g.v.buffer instanceof SharedArrayBuffer, g.v.byteOffset, g.v[0], g.v.buffer.byteLength);
      g.v[0] = 8; console.log(v[0]);";
    assert_eq!(lines(src, None), ["true 4 7 8", "8"]);
}

#[test]
fn grep_guard_no_length_change_on_a_sab_backed_vec() {
    // SabBlock invariant 1 (typed.rs): the Vec aliasing a shared block is never resized, truncated or replaced.
    // Any length-changing call on a RefCell<Vec> borrow in these files is a candidate, so it must not appear.
    const LEN_CHANGING: &[&str] = &[
        ".push(",
        ".resize(",
        ".resize_with(",
        ".truncate(",
        ".clear(",
        ".extend(",
        ".insert(",
        ".remove(",
        ".drain(",
        ".pop(",
        ".reserve(",
        ".append(",
        ".split_off(",
        ".retain(",
        ".shrink_to_fit(",
    ];
    for (name, src) in [
        ("typed.rs", include_str!("typed.rs")),
        ("worker.rs", include_str!("worker.rs")),
    ] {
        for (at, _) in src.match_indices("borrow_mut()") {
            let rest = &src[at + "borrow_mut()".len()..];
            for method in LEN_CHANGING {
                assert!(
                    !rest.starts_with(method),
                    "{name}: length-changing `{method}` on a borrow_mut (SabBlock invariant 1)"
                );
            }
        }
    }
    // the one whole-Vec replacement is detach_bytes, which must leave a shared block alone
    let typed = include_str!("typed.rs");
    assert_eq!(typed.matches("cell.borrow_mut() =").count(), 1);
    let detach = typed
        .find("pub(crate) fn detach_bytes")
        .expect("detach_bytes exists");
    assert!(typed[detach..].contains("if b.sab.is_some()"));
}

#[test]
fn registry_entry_is_taken_by_the_single_import_that_reads_it() {
    let block = crate::typed::new_sab_block(8);
    let id = crate::worker::sab_export(block.clone());
    assert_eq!(
        Arc::strong_count(&block),
        2,
        "the table holds one reference"
    );
    let got = crate::worker::sab_import(id).expect("the entry is there until it is read");
    assert!(Arc::ptr_eq(&got, &block));
    assert!(
        crate::worker::sab_import(id).is_none(),
        "an id is read once"
    );
    drop(got);
    assert_eq!(Arc::strong_count(&block), 1);
}

#[test]
fn undelivered_wire_releases_the_blocks_it_carries() {
    let wire = lines(
        "console.log(__thaw_worker_encode({ s: new SharedArrayBuffer(4) }));",
        None,
    )
    .remove(0);
    let id: u64 = wire
        .split("\"id\":")
        .nth(1)
        .and_then(|rest| rest.split(|c: char| !c.is_ascii_digit()).next())
        .and_then(|digits| digits.parse().ok())
        .expect("the wire names the block");
    crate::worker::release_wire(&wire);
    assert!(
        crate::worker::sab_import(id).is_none(),
        "release_wire takes the entry out"
    );
}

#[test]
fn shared_block_is_eight_byte_aligned_and_keeps_its_length_across_stores() {
    for len in [0usize, 1, 7, 8, 13, 64] {
        let block = crate::typed::new_sab_block(len);
        let a = crate::typed::sab_store(block.clone());
        let b = crate::typed::sab_store(block.clone());
        assert_eq!(a.borrow().as_ptr() as usize % 8, 0);
        assert_eq!(a.borrow().len(), len);
        if len > 0 {
            a.borrow_mut()[len - 1] = 9;
        }
        // dropping one store leaves the block (and the other store's view of it) alive
        drop(a);
        assert_eq!(b.borrow().len(), len);
        if len > 0 {
            assert_eq!(b.borrow()[len - 1], 9);
        }
    }
}

/// Runs `src` on a fresh Interp whose global `__sab` views `block` (the same bytes as every other Interp holding it).
fn lines_on_block(block: &Arc<SabBlock>, src: &str) -> Vec<String> {
    let mut it = Interp::new();
    let obj = it.new_shared_buffer(sab_store(block.clone()));
    it.global.declare("__sab", Value::Object(obj), true);
    match it.run(src) {
        Ok(_) => {}
        Err(Ctl::Throw(v)) => panic!("uncaught {}", it.to_str(&v)),
        Err(_) => panic!("stray completion"),
    }
    std::mem::take(&mut it.out)
}

#[test]
fn atomics_single_thread_results_match_node() {
    // Expected lines were produced by Node 22 running the same statements (single agent, one buffer).
    let src = "const a = new Int32Array(new SharedArrayBuffer(16));
      console.log(Atomics.store(a,0,5), Atomics.add(a,0,3), Atomics.sub(a,0,2), Atomics.exchange(a,0,7), Atomics.compareExchange(a,0,7,9), Atomics.compareExchange(a,0,1,4), Atomics.load(a,0));
      console.log(Atomics.store(a,1,2**32+5), a[1], Atomics.store(a,1,-0.9), Atomics.store(a,1,2**40+3), a[1]);
      const b = new Uint8Array(new SharedArrayBuffer(4));
      console.log(Atomics.add(b,0,250), Atomics.add(b,0,10), b[0], Atomics.sub(b,0,20), b[0], Atomics.load(b,0));
      const c = new Int8Array(new SharedArrayBuffer(4));
      console.log(Atomics.store(c,0,127), Atomics.add(c,0,1), c[0], Atomics.load(c,0), Atomics.exchange(c,0,-1), Atomics.load(c,0));
      console.log(Atomics.compareExchange(c,0,-1,200), c[0], Atomics.compareExchange(c,0,-56,3), c[0]);
      const d = new Int16Array(new SharedArrayBuffer(4));
      console.log(Atomics.store(d,0,40000), d[0], Atomics.and(d,0,0xff), Atomics.or(d,0,0x100), Atomics.xor(d,0,-1), d[0]);
      const e = new Uint16Array(new SharedArrayBuffer(4));
      console.log(Atomics.sub(e,0,1), e[0], Atomics.add(e,0,65536+2), e[0]);
      const f = new Uint32Array(new SharedArrayBuffer(8));
      console.log(Atomics.sub(f,0,1), f[0], Atomics.store(f,0,-1), Atomics.load(f,0), Atomics.add(f,1,NaN), Atomics.add(f,1,'7'), f[1]);
      console.log(Atomics.store(f,0,Infinity), Atomics.load(f,0), Atomics.store(f,0,-Infinity), Atomics.load(f,0));
      console.log(Atomics.and(f,0,NaN), Atomics.or(f,0,1.9), f[0], Atomics.compareExchange(f,0,1,'2'), f[0]);
      const g = new Int32Array(new SharedArrayBuffer(8));
      console.log(Atomics.wait(g,0,1,0), Atomics.wait(g,0,0,0), Atomics.wait(g,0,0,5), Atomics.wait(g,0,0,-5), Atomics.notify(g,0,1), Atomics.notify(g,0), Atomics.notify(g,0,-3));
      console.log(Atomics.wait(g,0,0,1), Atomics.isLockFree(1), Atomics.isLockFree(4), Atomics.isLockFree(3));
      try { Atomics.wait(new Int32Array(4),0,0,0) } catch(e){ console.log('nonshared wait', e.constructor.name) }
      try { Atomics.notify(new Int8Array(new SharedArrayBuffer(4)),0,1) } catch(e){ console.log('notify int8', e.constructor.name) }
      console.log(Atomics.notify(new Int32Array(4),0,1));
      try { Atomics.add(new Int32Array(new SharedArrayBuffer(8)), 2, 1) } catch(e){ console.log('oob', e.constructor.name) }
      try { Atomics.add(new Int32Array(new SharedArrayBuffer(8)), -1, 1) } catch(e){ console.log('neg', e.constructor.name) }
      const h = new Int32Array(new SharedArrayBuffer(8)); console.log(Atomics.add(h, 0.7, 2), Atomics.add(h, '1', 4), h[0], h[1]);";
    assert_eq!(
        lines(src, None),
        [
            "5 5 8 6 7 9 9",
            "4294967301 5 0 1099511627779 3",
            "0 250 4 4 240 240",
            "127 127 -128 -128 -128 -1",
            "-1 -56 -56 3",
            "40000 -25536 -25536 64 320 -321",
            "0 65535 65535 1",
            "0 4294967295 -1 4294967295 0 0 7",
            "Infinity 0 -Infinity 0",
            "0 0 1 1 2",
            "not-equal timed-out timed-out timed-out 0 0 0",
            "timed-out true true false",
            "nonshared wait TypeError",
            "notify int8 TypeError",
            "0",
            "oob RangeError",
            "neg RangeError",
            "0 0 2 4",
        ]
    );
}

#[test]
fn atomics_add_from_four_threads_totals_exactly() {
    // 4 Interps x 25000 increments of one word: a lost update anywhere would make the total short.
    let block = new_sab_block(4);
    let workers: Vec<_> = (0..4)
        .map(|_| {
            let block = block.clone();
            std::thread::Builder::new()
                .stack_size(64 << 20)
                .spawn(move || {
                    lines_on_block(&block, "const a = new Int32Array(__sab); for (let i = 0; i < 25000; i++) Atomics.add(a, 0, 1);");
                })
                .expect("thread spawns")
        })
        .collect();
    for w in workers {
        w.join().expect("thread joins");
    }
    assert_eq!(
        lines_on_block(&block, "console.log(new Int32Array(__sab)[0]);"),
        ["100000"]
    );
}

#[test]
fn atomics_compare_exchange_spinlock_protects_a_plain_counter() {
    // word 0 is the lock, word 1 the counter that only the lock holder touches with plain stores
    let block = new_sab_block(8);
    let worker = |block: Arc<SabBlock>| {
        std::thread::Builder::new()
            .stack_size(64 << 20)
            .spawn(move || {
                lines_on_block(
                    &block,
                    "const l = new Int32Array(__sab), c = new Int32Array(__sab, 4, 1);
                     for (let i = 0; i < 20000; i++) {
                       while (Atomics.compareExchange(l, 0, 0, 1) !== 0) {}
                       c[0] = c[0] + 1;
                       Atomics.store(l, 0, 0);
                     }",
                );
            })
            .expect("thread spawns")
    };
    let (a, b) = (worker(block.clone()), worker(block.clone()));
    a.join().expect("thread joins");
    b.join().expect("thread joins");
    assert_eq!(
        lines_on_block(&block, "console.log(new Int32Array(__sab, 4, 1)[0]);"),
        ["40000"]
    );
}

#[test]
fn atomics_wait_is_woken_by_notify_from_another_interp() {
    let block = new_sab_block(8);
    let waiter_block = block.clone();
    let waiter = std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(move || {
            // word 1 announces that the waiter is about to park; the notifier sleeps before it stores
            lines_on_block(
                &waiter_block,
                "const i = new Int32Array(__sab); Atomics.store(i, 1, 1); console.log(Atomics.wait(i, 0, 0));",
            )
        })
        .expect("thread spawns");
    let notifier = lines_on_block(
        &block,
        "const i = new Int32Array(__sab);
         while (Atomics.load(i, 1) !== 1) {}
         __thaw_sleep_ms(100);
         Atomics.store(i, 0, 1);
         console.log(Atomics.notify(i, 0, 1), Atomics.notify(i, 0, 1));",
    );
    assert_eq!(notifier, ["1 0"]);
    assert_eq!(waiter.join().expect("thread joins"), ["ok"]);
}

#[test]
fn atomics_wait_notify_count_limit_wakes_exactly_that_many_waiters() {
    let block = new_sab_block(8);
    let waiters: Vec<_> = (0..2)
        .map(|_| {
            let block = block.clone();
            std::thread::Builder::new()
                .stack_size(64 << 20)
                .spawn(move || {
                    lines_on_block(
                        &block,
                        "const i = new Int32Array(__sab); Atomics.add(i, 1, 1); console.log(Atomics.wait(i, 0, 0, 2000));",
                    )
                })
                .expect("thread spawns")
        })
        .collect();
    let notifier = lines_on_block(
        &block,
        "const i = new Int32Array(__sab);
         while (Atomics.load(i, 1) !== 2) {}
         __thaw_sleep_ms(100);
         Atomics.store(i, 0, 1);
         console.log(Atomics.notify(i, 0, 1));",
    );
    // the count limit wakes one waiter; the other waits out its 2 s timeout
    assert_eq!(notifier, ["1"]);
    let mut results: Vec<String> = waiters
        .into_iter()
        .flat_map(|w| w.join().expect("thread joins"))
        .collect();
    results.sort();
    assert_eq!(results, ["ok", "timed-out"]);
}

#[test]
fn atomics_wait_on_a_terminated_interp_returns_with_exit_code_one() {
    // the worker interrupt (worker.rs terminate) must end an infinite wait; the flag is set after 50 ms
    let block = new_sab_block(4);
    let flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let thread_flag = flag.clone();
    let waiter = std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(move || {
            let mut it = Interp::new();
            it.interrupt = Some(thread_flag);
            let obj = it.new_shared_buffer(sab_store(block));
            it.global.declare("__sab", Value::Object(obj), true);
            // a script-level catch must not see the interrupt: the wait ends as an exit, not a thrown error
            let r = it.run(
                "const i = new Int32Array(__sab); try { Atomics.wait(i, 0, 0); console.log('returned'); } catch (e) { console.log('threw ' + e.name); }",
            );
            (matches!(r, Err(Ctl::Exit(1))), std::mem::take(&mut it.out))
        })
        .expect("thread spawns");
    std::thread::sleep(std::time::Duration::from_millis(50));
    flag.store(true, std::sync::atomic::Ordering::Relaxed);
    assert_eq!(
        waiter.join().expect("thread joins"),
        (true, Vec::<String>::new())
    );
}
