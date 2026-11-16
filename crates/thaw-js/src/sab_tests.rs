//! SharedArrayBuffer storage and sharing (plan A1/A2): one block that every Interp holding it reads and writes; it
//! crosses threads only as a `{t:'SAB', id}` wire node, and `structuredClone` on one thread returns the same block.
//! Each thread runs its own Interp (Interp is not Send), so every test builds its Interp inside its thread.

use super::*;
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
