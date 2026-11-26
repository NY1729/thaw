//! Pins `size_of` of the hot types, so a later layout change (item 4 of the perf plan) is deliberate:
//! a shrink makes the matching assertion fail and the new size is written here in the same PR.
//! Run: `cargo test -p thaw-js --lib size_tests -- --nocapture` prints the table.
use super::*;
use std::mem::size_of;

#[test]
fn hot_type_sizes_are_pinned() {
    let table: [(&str, usize, usize); 8] = [
        ("Value", size_of::<Value>(), VALUE),
        ("Slot", size_of::<Slot>(), SLOT),
        ("Prop", size_of::<Prop>(), PROP),
        ("Kind", size_of::<Kind>(), KIND),
        ("ObjData", size_of::<ObjData>(), OBJDATA),
        ("Scope", size_of::<Scope>(), SCOPE),
        ("Binding", size_of::<Binding>(), BINDING),
        ("FuncDef", size_of::<FuncDef>(), FUNCDEF),
    ];
    let mut bad = Vec::new();
    for (n, got, want) in table {
        println!("size_of::<{n}>() = {got}");
        if got != want {
            bad.push(format!("{n}: {got} (pinned {want})"));
        }
    }
    assert!(bad.is_empty(), "layout changed, update the pins: {bad:?}");
}

const VALUE: usize = 24;
const SLOT: usize = 48;
const PROP: usize = 72;
const KIND: usize = 80;
const OBJDATA: usize = 216;
const SCOPE: usize = 136;
const BINDING: usize = 32;
const FUNCDEF: usize = 232;
