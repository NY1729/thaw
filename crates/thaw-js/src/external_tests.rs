//! A store aliasing memory another owner holds (`new_external_bytes`): reads and writes are the owner's bytes, the
//! owner's `release` runs exactly once (on drop or detach), and the owner's memory is never freed by the store.

use super::*;
use std::cell::Cell;
use std::rc::Rc;

fn run(it: &mut Interp, src: &str) -> Vec<String> {
    match it.run(src) {
        Ok(_) => {}
        Err(Ctl::Throw(v)) => panic!("uncaught {}", it.to_str(&v)),
        Err(_) => panic!("stray completion"),
    }
    std::mem::take(&mut it.out)
}

/// An external ArrayBuffer over `backing` plus a counter of `release` calls.
fn external(it: &Interp, backing: &mut [u8]) -> (Value, Rc<Cell<u32>>) {
    let released = Rc::new(Cell::new(0));
    let counter = released.clone();
    let value = unsafe {
        it.array_buffer_external(
            backing.as_mut_ptr(),
            backing.len(),
            Box::new(move || counter.set(counter.get() + 1)),
        )
    };
    (value, released)
}

#[test]
fn external_store_reads_and_writes_the_owners_bytes() {
    let mut backing = vec![1u8, 2, 3, 4];
    let mut it = Interp::new();
    let (ab, released) = external(&it, &mut backing);
    it.global.declare("ab", ab, true);
    let out = run(
        &mut it,
        "const v = new Uint8Array(ab); console.log(v[0], v.length, ab.byteLength); v[1] = 9;",
    );
    assert_eq!(out, ["1 4 4"]);
    assert_eq!(
        backing,
        [1, 9, 3, 4],
        "a write through the store reaches the owner"
    );
    backing[2] = 7;
    let out = run(
        &mut it,
        "console.log(new Uint8Array(ab)[2], new DataView(ab).getUint8(1));",
    );
    assert_eq!(
        out,
        ["7 9"],
        "a write by the owner is seen through the store"
    );
    assert_eq!(released.get(), 0);
}

#[test]
fn external_store_releases_once_on_drop_and_leaves_the_memory_alone() {
    let mut backing = vec![5u8; 8];
    let released;
    {
        let mut it = Interp::new();
        let (ab, counter) = external(&it, &mut backing);
        released = counter;
        it.global.declare("ab", ab, true);
        run(&mut it, "new Uint8Array(ab)[0] = 6;");
        assert_eq!(released.get(), 0);
    }
    assert_eq!(released.get(), 1, "dropping the last holder releases once");
    assert_eq!(
        backing,
        [6, 5, 5, 5, 5, 5, 5, 5],
        "the owner's memory is intact and still writable"
    );
}

#[test]
fn external_store_detach_releases_once_and_empties_the_store() {
    let mut backing = vec![1u8, 2, 3];
    let mut it = Interp::new();
    let (ab, released) = external(&it, &mut backing);
    it.global.declare("ab", ab, true);
    let out = run(
        &mut it,
        "const t = ab.transfer(); console.log(ab.byteLength, t.byteLength);",
    );
    assert_eq!(out, ["0 3"]);
    assert_eq!(released.get(), 1, "detach releases the owner's pin");
    drop(it);
    assert_eq!(
        released.get(),
        1,
        "dropping afterwards does not release twice"
    );
    assert_eq!(backing, [1, 2, 3], "the owner's memory is untouched");
}

#[test]
fn external_store_of_length_zero_is_safe() {
    let mut backing: Vec<u8> = Vec::new();
    let mut it = Interp::new();
    let (ab, released) = external(&it, &mut backing);
    it.global.declare("ab", ab, true);
    assert_eq!(
        run(
            &mut it,
            "console.log(ab.byteLength, new Uint8Array(ab).length);"
        ),
        ["0 0"]
    );
    drop(it);
    assert_eq!(released.get(), 1);
}
