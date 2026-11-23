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

/// Runs `src` as a main program (its event loop included) on a fresh Interp over `block`; returns its console lines.
fn main_lines_on_block(block: &Arc<SabBlock>, src: &str) -> Vec<String> {
    let mut it = Interp::new();
    let obj = it.new_shared_buffer(sab_store(block.clone()));
    it.global.declare("__sab", Value::Object(obj), true);
    match it.run_main(src) {
        Ok(0) => {}
        Ok(code) => panic!("exit code {code}"),
        Err(Ctl::Throw(v)) => panic!("uncaught {}", it.to_str(&v)),
        Err(_) => panic!("stray completion"),
    }
    std::mem::take(&mut it.out)
}

/// `main_lines_on_block` for a program that makes its own SharedArrayBuffer.
fn main_lines(src: &str) -> Vec<String> {
    main_lines_on_block(&new_sab_block(4), src)
}

#[test]
fn atomics_wait_async_sync_results_shapes_and_argument_errors() {
    let src = "const ia = new Int32Array(new SharedArrayBuffer(8));
      const r1 = Atomics.waitAsync(ia, 0, 1);
      console.log(r1.async, r1.value, Object.keys(r1).join(','));
      const r2 = Atomics.waitAsync(ia, 0, 0, 0);
      console.log(r2.async, r2.value);
      const r3 = Atomics.waitAsync(ia, 0, 0, -5);
      console.log(r3.async, r3.value);
      console.log(Atomics.waitAsync.length, Atomics.waitAsync.name);
      try { Atomics.waitAsync(new Int32Array(4), 0, 0, 0); } catch (e) { console.log('nonshared', e.constructor.name); }
      try { Atomics.waitAsync(new Int16Array(new SharedArrayBuffer(8)), 0, 0, 0); } catch (e) { console.log('int16', e.constructor.name); }
      try { Atomics.waitAsync(new Float64Array(new SharedArrayBuffer(16)), 0, 0, 0); } catch (e) { console.log('f64', e.constructor.name); }
      try { Atomics.waitAsync(ia, 2, 0, 0); } catch (e) { console.log('range', e.constructor.name); }
      const p = Atomics.waitAsync(ia, 0, 0, 10);
      console.log(p.async, typeof p.value.then);";
    assert_eq!(
        lines(src, None),
        [
            "false not-equal async,value",
            "false timed-out",
            "false timed-out",
            "4 waitAsync",
            "nonshared TypeError",
            "int16 TypeError",
            "f64 TypeError",
            "range RangeError",
            "true function",
        ]
    );
}

#[test]
fn atomics_wait_async_fixture_n66_timeout_then_notify_resolves_ok() {
    // retired-engine contract n66, run as a program: the setInterval keeps the loop alive while the promises settle
    let src = "(async () => {
      const keep = setInterval(() => {}, 50);
      const ia = new Int32Array(new SharedArrayBuffer(16)); const out = [];
      const r = Atomics.waitAsync(ia, 0, 0, 5);
      out.push(await r.value);
      out.push(Atomics.notify(ia, 0));
      const r2 = Atomics.waitAsync(ia, 0, 0, 1000);
      out.push(Atomics.notify(ia, 0, 1), await r2.value);
      clearInterval(keep);
      console.log(JSON.stringify(out));
    })();";
    assert_eq!(main_lines(src), ["[\"timed-out\",0,1,\"ok\"]"]);
}

#[test]
fn atomics_wait_async_fixture_n67_index_coercion_and_range() {
    let src = "(async () => {
      const ia = new Int32Array(new SharedArrayBuffer(16)); const out = [];
      for (const idx of [1.5, '1', -0.5, undefined, NaN, null, true, -1, 4, 2**53, Infinity, {valueOf(){return 2}}]) {
        try { const r = Atomics.waitAsync(ia, idx, 1, 0); out.push(r.value); } catch (e) { out.push(e.constructor.name); }
      }
      console.log(JSON.stringify(out));
    })();";
    assert_eq!(
        main_lines(src),
        ["[\"not-equal\",\"not-equal\",\"not-equal\",\"not-equal\",\"not-equal\",\"not-equal\",\"not-equal\",\"RangeError\",\"RangeError\",\"RangeError\",\"RangeError\",\"not-equal\"]"]
    );
}

#[test]
fn atomics_wait_async_fixture_n68_coerces_value_then_timeout_then_count() {
    let src = "(async () => {
      const ia = new Int32Array(new SharedArrayBuffer(16)); const log = [];
      const t = { valueOf() { log.push('timeout'); return 0; } }, v = { valueOf() { log.push('value'); return 5; } };
      const r = Atomics.waitAsync(ia, 0, v, t);
      const c = { valueOf() { log.push('count'); return 1; } };
      Atomics.notify(ia, 0, c);
      const r2 = Atomics.waitAsync(ia, 0, '0', 0);
      console.log(JSON.stringify([log, r.value, r2.value]));
    })();";
    assert_eq!(
        main_lines(src),
        ["[[\"value\",\"timeout\",\"count\"],\"not-equal\",\"timed-out\"]"]
    );
}

#[test]
fn atomics_wait_async_fixture_n70_int32_value_coercion() {
    // retired-engine contract n70 without its BigInt64Array rows (BigInt64 is not supported yet; see atomics_wait_async note)
    let src = "(async () => {
      const ia = new Int32Array(new SharedArrayBuffer(16)); const out = [];
      out.push(Atomics.waitAsync(ia, 0, '0', 0).value, Atomics.waitAsync(ia, 0, 2**32, 0).value, Atomics.waitAsync(ia, 0, 1.9, 0).value);
      console.log(JSON.stringify(out));
    })();";
    assert_eq!(
        main_lines(src),
        ["[\"timed-out\",\"timed-out\",\"not-equal\"]"]
    );
}

#[test]
fn atomics_wait_async_shares_one_fifo_with_sync_waiters() {
    // thread A parks in a sync wait first; the main program then queues an async waiter behind it. The first
    // notify must wake A (so the promise is still pending in the next timer turn); the second wakes the async waiter.
    let block = new_sab_block(8);
    let sync_block = block.clone();
    let sync = std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(move || {
            lines_on_block(
                &sync_block,
                "const i = new Int32Array(__sab); Atomics.store(i, 1, 1); console.log('sync', Atomics.wait(i, 0, 0, 2000));",
            )
        })
        .expect("thread spawns");
    let program = main_lines_on_block(
        &block,
        "const i = new Int32Array(__sab);
         while (Atomics.load(i, 1) !== 1) {}
         __thaw_sleep_ms(30);
         let got = null;
         Atomics.waitAsync(i, 0, 0, 2000).value.then((v) => { got = v; console.log('async', v); });
         console.log('notify1', Atomics.notify(i, 0, 1));
         setTimeout(() => {
           console.log('mid', got);
           console.log('notify2', Atomics.notify(i, 0, 1));
         }, 20);",
    );
    assert_eq!(program, ["notify1 1", "mid null", "notify2 1", "async ok"]);
    assert_eq!(sync.join().expect("thread joins"), ["sync ok"]);
}

#[test]
fn atomics_wait_async_is_woken_by_notify_from_another_interp() {
    // the waiter queues before it announces itself (word 1), so the notifier never races the queueing
    let block = new_sab_block(8);
    let waiter_block = block.clone();
    let waiter = std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(move || {
            main_lines_on_block(
                &waiter_block,
                "const i = new Int32Array(__sab);
                 const keep = setInterval(() => {}, 50);
                 Atomics.waitAsync(i, 0, 0, 5000).value.then((v) => { console.log('woke', v); clearInterval(keep); });
                 Atomics.store(i, 1, 1);",
            )
        })
        .expect("thread spawns");
    let notifier = lines_on_block(
        &block,
        "const i = new Int32Array(__sab);
         for (let n = 0; n < 2e6 && Atomics.load(i, 1) !== 1; n++) {}
         Atomics.store(i, 0, 1);
         console.log(Atomics.notify(i, 0, 1));",
    );
    assert_eq!(notifier, ["1"]);
    assert_eq!(waiter.join().expect("thread joins"), ["woke ok"]);
}

#[test]
fn atomics_wait_async_timeout_settles_timed_out_while_the_loop_is_kept_alive() {
    // the 10 ms timeout fires before the 60 ms referenced timer that keeps the program alive
    let src = "const ia = new Int32Array(new SharedArrayBuffer(8));
      Atomics.waitAsync(ia, 0, 0, 10).value.then((v) => console.log('t', v));
      setTimeout(() => console.log('keep'), 60);";
    assert_eq!(main_lines(src), ["t timed-out", "keep"]);
}

#[test]
fn atomics_wait_async_does_not_keep_the_process_alive() {
    // Node 22: a pending waitAsync (finite or infinite timeout) alone lets the program exit; nothing settles
    let src = "const ia = new Int32Array(new SharedArrayBuffer(8));
      Atomics.waitAsync(ia, 0, 0, 200).value.then(() => console.log('fired'));
      Atomics.waitAsync(ia, 0, 0).value.then(() => console.log('fired inf'));
      console.log('end');";
    assert_eq!(main_lines(src), ["end"]);
}

#[test]
fn atomics_wait_async_dropped_realm_leaves_no_waiter_for_notify() {
    // a realm that drops with a pending waitAsync must not count as a waiter (notify returns 0)
    let block = new_sab_block(8);
    let dropped_block = block.clone();
    std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(move || {
            let mut it = Interp::new();
            let obj = it.new_shared_buffer(sab_store(dropped_block));
            it.global.declare("__sab", Value::Object(obj), true);
            assert!(it
                .run("const i = new Int32Array(__sab); Atomics.waitAsync(i, 0, 0);")
                .is_ok());
        })
        .expect("thread spawns")
        .join()
        .expect("thread joins");
    assert_eq!(
        lines_on_block(
            &block,
            "const i = new Int32Array(__sab); console.log(Atomics.notify(i, 0, 1));"
        ),
        ["0"]
    );
}

/// Runs `src` as a main program with its event loop, on its own thread (Interp is built inside it); returns its lines.
fn port_main_lines(src: &'static str) -> Vec<String> {
    in_thread(move || {
        let mut it = Interp::new();
        match it.run_main(src) {
            Ok(0) => {}
            Ok(code) => panic!("exit code {code}"),
            Err(Ctl::Throw(v)) => panic!("uncaught {}", it.to_str(&v)),
            Err(_) => panic!("stray completion"),
        }
        std::mem::take(&mut it.out)
    })
}

#[test]
fn receive_message_on_port_takes_a_message_a_worker_posted_while_main_waited() {
    // main blocks in Atomics.wait; the worker posts on the port it was handed, then notifies. Main's sync
    // receiveMessageOnPort must see that post (Node: it dequeues what the other thread already sent).
    let lines = port_main_lines(
        "const { Worker, receiveMessageOnPort, MessageChannel } = globalThis.__thaw_worker_threads_exports;
         const { port1, port2 } = new MessageChannel();
         const sab = new SharedArrayBuffer(4);
         const w = new Worker(
           \"const { workerData } = require('node:worker_threads'); const i = new Int32Array(workerData.sab);\" +
           \"workerData.port.postMessage({ hi: 1 }); Atomics.store(i, 0, 1); Atomics.notify(i, 0, 1);\",
           { eval: true, workerData: { port: port2, sab }, transferList: [port2] });
         const i = new Int32Array(sab);
         console.log(Atomics.wait(i, 0, 0, 5000));
         console.log(JSON.stringify(receiveMessageOnPort(port1)));
         w.terminate();",
    );
    assert_eq!(lines, ["ok", r#"{"message":{"hi":1}}"#]);
}

#[test]
fn receive_message_on_port_returns_queued_worker_messages_in_order_one_per_call() {
    let lines = port_main_lines(
        "const { Worker, receiveMessageOnPort, MessageChannel } = globalThis.__thaw_worker_threads_exports;
         const { port1, port2 } = new MessageChannel();
         const sab = new SharedArrayBuffer(4);
         const w = new Worker(
           \"const { workerData } = require('node:worker_threads'); const i = new Int32Array(workerData.sab);\" +
           \"for (let n = 1; n <= 3; n++) workerData.port.postMessage(n); Atomics.store(i, 0, 1); Atomics.notify(i, 0, 1);\",
           { eval: true, workerData: { port: port2, sab }, transferList: [port2] });
         const i = new Int32Array(sab);
         console.log(Atomics.wait(i, 0, 0, 5000));
         for (let k = 0; k < 4; k++) console.log(JSON.stringify(receiveMessageOnPort(port1)));
         w.terminate();",
    );
    assert_eq!(
        lines,
        [
            "ok",
            r#"{"message":1}"#,
            r#"{"message":2}"#,
            r#"{"message":3}"#,
            "undefined",
        ]
    );
}

#[test]
fn receive_message_on_port_pump_leaves_other_worker_events_for_the_loop() {
    // the worker's `online` and `exit` are not port deliveries: the sync pump must not take them, so the loop
    // still fires them after the synchronous read, in their order
    let lines = port_main_lines(
        "const { Worker, receiveMessageOnPort, MessageChannel } = globalThis.__thaw_worker_threads_exports;
         const { port1, port2 } = new MessageChannel();
         const sab = new SharedArrayBuffer(4);
         const w = new Worker(
           \"const { workerData } = require('node:worker_threads'); const i = new Int32Array(workerData.sab);\" +
           \"workerData.port.postMessage('m'); Atomics.store(i, 0, 1); Atomics.notify(i, 0, 1);\",
           { eval: true, workerData: { port: port2, sab }, transferList: [port2] });
         w.on('online', () => console.log('online'));
         w.on('exit', (code) => console.log('exit', code));
         const i = new Int32Array(sab);
         console.log(Atomics.wait(i, 0, 0, 5000));
         console.log(JSON.stringify(receiveMessageOnPort(port1)));",
    );
    assert_eq!(lines, ["ok", r#"{"message":"m"}"#, "online", "exit 0"]);
}

#[test]
fn a_growable_shared_buffer_grows_and_length_tracking_views_follow_it() {
    // expected text is node 22's output for the same program
    let src = "const s = new SharedArrayBuffer(8, { maxByteLength: 16 });
      console.log(s.growable, s.maxByteLength, s.byteLength);
      const u = new Uint8Array(s); const t = new Uint8Array(s, 4); const f = new Uint8Array(s, 0, 4);
      s.grow(12);
      console.log(s.byteLength, u.length, t.length, f.length, new Int32Array(s).length);
      for (const n of [4, 17]) { try { s.grow(n); } catch (e) { console.log(e.name + ': ' + e.message); } }
      const fixed = new SharedArrayBuffer(8);
      console.log(fixed.growable, fixed.maxByteLength);
      try { fixed.grow(9); } catch (e) { console.log(e.name + ': ' + e.message); }
      try { new SharedArrayBuffer(8, { maxByteLength: 4 }); } catch (e) { console.log(e.name + ': ' + e.message); }
      const sl = s.slice(2, 6); console.log(sl.byteLength, sl.growable);
      u[11] = 5; console.log(new Uint8Array(s)[11]);
      s.grow(16); console.log(u.length, t.length);";
    assert_eq!(
        lines(src, None),
        [
            "true 16 8",
            "12 12 8 4 3",
            "RangeError: SharedArrayBuffer.prototype.grow: Invalid length parameter",
            "RangeError: SharedArrayBuffer.prototype.grow: Invalid length parameter",
            "false 8",
            "TypeError: Method SharedArrayBuffer.prototype.grow called on incompatible receiver #<SharedArrayBuffer>",
            "RangeError: Invalid array buffer max length",
            "4 false",
            "5",
            "16 12"
        ]
    );
}

#[test]
fn a_growth_through_one_store_is_seen_by_the_other_stores_of_the_block() {
    // the block arrives as a wire node; a structured clone is a second store over it (as another thread's is)
    let wire = in_thread(|| {
        lines(
            "const s = new SharedArrayBuffer(4, { maxByteLength: 8 });
             console.log(__thaw_worker_encode(s));",
            None,
        )
        .remove(0)
    });
    let src = "const g = __thaw_worker_decode(__wire_in); const v = new Uint8Array(g);
      console.log(g.byteLength, g.growable, g.maxByteLength, v.length);
      const c = structuredClone(g); const cv = new Uint8Array(c); g.grow(8);
      console.log(g.byteLength, v.length, c === g, c.byteLength, cv.length);";
    assert_eq!(
        lines(src, Some(wire.as_str())),
        ["4 true 8 4", "8 8 false 8 8"]
    );
}
