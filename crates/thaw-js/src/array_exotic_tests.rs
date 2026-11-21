use super::*;

/// Run `src` and join its console.log lines with `|` (or the uncaught error text).
fn one(src: &str) -> String {
    let mut it = Interp::new();
    match it.run(src) {
        Ok(_) => it.out.join("|"),
        Err(Ctl::Throw(v)) => format!("Uncaught {}", it.to_str(&v)),
        Err(_) => "stray completion".into(),
    }
}

// Array length and indices past the dense ceiling (sparse tail). Expectations are Node 22 (v22.23.1)
// output, each run three times with identical results (probe files kept in thaw-tmp/ar2probe).

#[test]
fn array_length_set_past_dense_ceiling() {
    assert_eq!(
        one("const e = []; e.length = 2 ** 32 - 1; console.log(e.length, Object.keys(e).length);"),
        "4294967295 0"
    );
}

#[test]
fn array_length_past_uint32_is_range_error() {
    assert_eq!(
        one("const m = []; try { m.length = 2 ** 32; } catch (x) { console.log(x.constructor.name, x.message); }"),
        "RangeError Invalid array length"
    );
    assert_eq!(
        one("const m = []; try { m.length = -1; } catch (x) { console.log(x.constructor.name, x.message); }"),
        "RangeError Invalid array length"
    );
}

#[test]
fn array_push_at_max_length_is_range_error() {
    assert_eq!(
        one("const e = []; e.length = 2 ** 32 - 1; try { e.push(1); } catch (x) { console.log(x.constructor.name, x.message); }"),
        "RangeError Invalid array length"
    );
}

#[test]
fn array_index_max_minus_one_extends_length() {
    assert_eq!(
        one("const f = []; f[4294967294] = 1; console.log(f.length, JSON.stringify(Object.keys(f)), f[4294967294], 4294967294 in f);"),
        "4294967295 [\"4294967294\"] 1 true"
    );
}

#[test]
fn array_index_u32_max_is_a_plain_key() {
    assert_eq!(
        one("const a = []; a[4294967295] = 1; console.log(a.length, JSON.stringify(Object.keys(a)));"),
        "0 [\"4294967295\"]"
    );
}

#[test]
fn array_huge_index_is_sparse() {
    assert_eq!(
        one("const g = []; g[1e9] = 1; console.log(g.length, JSON.stringify(Object.keys(g)), 1e9 in g, 5 in g);"),
        "1000000001 [\"1000000000\"] true false"
    );
}

#[test]
fn array_sparse_keys_follow_dense_keys_in_order() {
    assert_eq!(
        one("const d = [1, 2]; d[1e9] = 3; d[5] = 4; console.log(JSON.stringify(Object.keys(d)), d.length);"),
        "[\"0\",\"1\",\"5\",\"1000000000\"] 1000000001"
    );
}

#[test]
fn array_new_with_huge_length_allocates_nothing() {
    assert_eq!(one("console.log(new Array(2 ** 31).length);"), "2147483648");
}

#[test]
fn array_new_with_bad_length_is_range_error() {
    assert_eq!(
        one("try { new Array(-1); } catch (x) { console.log(x.constructor.name, x.message); }"),
        "RangeError Invalid array length"
    );
    assert_eq!(
        one("try { new Array(1.5); } catch (x) { console.log(x.constructor.name, x.message); }"),
        "RangeError Invalid array length"
    );
}

#[test]
fn array_delete_sparse_element() {
    assert_eq!(
        one("const q = []; q[1e9] = 1; console.log(delete q[1e9], q.length, 1e9 in q);"),
        "true 1000000001 false"
    );
}

#[test]
fn array_shrinking_length_drops_sparse_elements() {
    assert_eq!(
        one("const h = [1, 2, 3]; h[1e9] = 5; h.length = 0; console.log(h.length, JSON.stringify(Object.keys(h)), h[1e9]);"),
        "0 [] undefined"
    );
    assert_eq!(
        one("const g = []; g[1e9] = 1; g.length = 3; console.log(g.length, JSON.stringify(Object.keys(g)));"),
        "3 []"
    );
}

#[test]
fn array_pop_sparse_element() {
    assert_eq!(
        one("const g = []; g[1e9] = 1; console.log(g.length, g.pop(), g.length);"),
        "1000000001 1 1000000000"
    );
}

#[test]
fn array_from_array_like_past_uint32_is_range_error() {
    assert_eq!(
        one("try { Array.from({ length: 2 ** 32 }); } catch (x) { console.log(x.constructor.name, x.message); }"),
        "RangeError Invalid array length"
    );
}

// Generic reads and own-key queries that work on the sparse tail (expected values from Node 22, three runs).

#[test]
fn array_sparse_at_and_inspect() {
    assert_eq!(
        one("const a = [1, 2]; a[1e9] = 'x'; console.log([a.at(-1), a.at(0), a.at(1)].join('|'));"),
        "x|1|2"
    );
    assert_eq!(
        one("const a = [1, 2]; a[1e9] = 'x'; console.log(a);"),
        "[ 1, 2, <999999998 empty items>, 'x' ]"
    );
    assert_eq!(
        one("const a = []; a.length = 2 ** 32 - 1; console.log(a);"),
        "[ <4294967295 empty items> ]"
    );
    assert_eq!(
        one("const a = [1, , 3]; a[1e9] = 5; a[1e9 + 2] = 7; console.log(a);"),
        "[ 1, <1 empty item>, 3, <999999997 empty items>, 5, <1 empty item>, 7 ]"
    );
}

#[test]
fn array_sparse_own_property_queries() {
    assert_eq!(
        one("const a = [1]; a[1e9] = 2; console.log(Object.prototype.hasOwnProperty.call(a, 1e9), Object.hasOwn(a, 1e9 - 1), 1e9 in a);"),
        "true false true"
    );
    assert_eq!(
        one("const a = [1]; a[1e9] = 2; a[1e9 + 1] = 3; console.log(Object.keys(a).join(','));"),
        "0,1000000000,1000000001"
    );
    assert_eq!(
        one("const a = []; a[1e9] = 'v'; console.log(JSON.stringify(Object.entries(a)));"),
        "[[\"1000000000\",\"v\"]]"
    );
    assert_eq!(
        one("const a = []; a[1e9] = 'v'; const d = Object.getOwnPropertyDescriptor(a, 1e9); console.log(d.value, d.writable, d.enumerable, d.configurable);"),
        "v true true true"
    );
    assert_eq!(
        one("const a = []; a[1e9] = 1; a.p = 2; console.log(Reflect.ownKeys(a).join(','));"),
        "1000000000,length,p"
    );
}

#[test]
fn array_sparse_pop_delete_and_shrink() {
    assert_eq!(
        one("const a = []; a[1e9] = 'z'; console.log(a.pop(), a.length, 1e9 in a);"),
        "z 1000000000 false"
    );
    assert_eq!(
        one("const a = [1]; a[1e9] = 2; delete a[1e9]; console.log(a.length, Object.keys(a).length);"),
        "1000000001 1"
    );
    assert_eq!(
        one("const a = []; a[1e9] = 1; a.length = 1e9; console.log(a.length, Object.keys(a).length);"),
        "1000000000 0"
    );
    assert_eq!(
        one("const a = []; a.length = 2 ** 32 - 2; a.push('p'); console.log(a.length, a[2 ** 32 - 2]);"),
        "4294967295 p"
    );
}

// Builtins that still read the dense slots throw the stopgap RangeError on a sparse tail (`sparse_unsupported`).
#[test]
fn array_sparse_tail_guards_dense_only_builtins() {
    let calls = [
        "a.shift()",
        "a.unshift(0)",
        "a.reverse()",
        "a.splice(0, 1)",
        "a.join('-')",
        "String(a)",
        "a.includes('x')",
        "a.slice(0, 1)",
        "a.concat([9])",
        "[9].concat(a)",
        "Math.max.apply(null, a)",
        "Array.from(a)",
        "JSON.stringify(a)",
        "Object.freeze(a)",
        "Object.seal(a)",
    ];
    for call in calls {
        assert_eq!(
            one(&format!("const a = [1, 2]; a[1e9] = 'x'; try {{ {call}; }} catch (x) {{ console.log(x.constructor.name, x.message); }}")),
            "RangeError Invalid array length",
            "{call}"
        );
    }
}

#[test]
fn array_sparse_index_of_skips_holes() {
    assert_eq!(
        one("const a = [1, 2]; a[1e9] = 'x'; console.log(a.indexOf('x'), a.lastIndexOf(1), a.indexOf(2, 1), a.lastIndexOf('x', 5), a.indexOf(1, -1e9), a.lastIndexOf(1, -2), a.indexOf('x', 1e9 + 1));"),
        "1000000000 0 1 -1 -1 0 -1"
    );
    assert_eq!(
        one("const u = []; u[1e9] = undefined; console.log(u.indexOf(undefined), u.lastIndexOf(undefined), u.indexOf(undefined, 1e9 + 1));"),
        "1000000000 1000000000 -1"
    );
}

// Length-only tails: Node's join/slice would run for 2**32-1 steps (its own errors differ), so this pins the stopgap.
#[test]
fn array_length_only_tail_guards_join_and_slice() {
    for call in ["e.join()", "e.slice(0)"] {
        assert_eq!(
            one(&format!("const e = []; e.length = 2 ** 32 - 1; try {{ {call}; }} catch (x) {{ console.log(x.constructor.name, x.message); }}")),
            "RangeError Invalid array length",
            "{call}"
        );
    }
}

#[test]
fn array_dense_builtins_unaffected_by_guard() {
    assert_eq!(
        one("const a = [1, 2, 3]; a.length = 2; console.log(a.shift(), a.join('-'), [4].concat(a).length);"),
        "1 2 2"
    );
}

// Integrity levels on an exotic array whose dense slots are empty (the sparse element is at 2 ** 26). Expectations
// are Node 22.23.1 output (three identical runs). `isFrozen` on a non-extensible array with a sparse element is the
// case the sparse tail must answer (it used to read only the dense slots and said true).
#[test]
fn array_sparse_integrity_levels_match_node() {
    assert_eq!(
        one(r#"
const N = 2 ** 26;
const ext = []; ext[N] = 1;
console.log('T1', Object.isSealed(ext), Object.isFrozen(ext));
const pe = []; pe[N] = 1; Object.preventExtensions(pe);
console.log('T2', Object.isSealed(pe), Object.isFrozen(pe));
const emptied = []; emptied[N] = 1; Object.preventExtensions(emptied); delete emptied[N];
console.log('T4', Object.isSealed(emptied), Object.isFrozen(emptied));
"#),
        "T1 false false|T2 false false|T4 true true"
    );
}

// `Object.seal` / `Object.freeze` on an exotic array are the stopgap guard (their per-element attribute work is not
// done for the sparse tail); Node returns the array.
#[test]
fn array_sparse_seal_and_freeze_refuse() {
    for call in ["Object.seal(a)", "Object.freeze(a)"] {
        assert_eq!(
            one(&format!("const a = []; a[2 ** 26] = 1; try {{ {call}; console.log('no-throw'); }} catch (x) {{ console.log(x.constructor.name, x.message); }}")),
            "RangeError Invalid array length",
            "{call}"
        );
    }
}

#[test]
fn array_sparse_prevent_extensions_refuses_new_index() {
    assert_eq!(
        one(r#"
const N = 2 ** 26;
const wr = [1]; wr[N] = 'x'; Object.preventExtensions(wr); wr[N + 5] = 'z'; wr[N] = 'q';
console.log('T5', wr.length, wr[N + 5], wr[N], Object.keys(wr).join(','));
const st = () => { 'use strict'; const a = [1]; a[N] = 'x'; Object.preventExtensions(a); try { a[N + 5] = 1; return 'no'; } catch (e) { return e.constructor.name + ' ' + e.message; } };
console.log('T6', st());
"#),
        "T5 67108865 undefined q 0,67108864|T6 TypeError Cannot add property 67108869, object is not extensible"
    );
}

#[test]
fn array_sparse_at_find_last_and_last_index_of() {
    assert_eq!(
        one(r#"
const N = 2 ** 26;
const at = [1]; at[N] = 'x'; at[N + 3] = 'y';
console.log('T9', at.at(N), at.at(-4), at.at(-1), at.at(N + 1), at.findLast(v => v !== undefined), at.findLastIndex(v => v === 'x'));
console.log('T10', at.lastIndexOf('y'), at.lastIndexOf(undefined), at.indexOf('x'));
"#),
        "T9 x x y undefined y 67108864|T10 67108867 -1 67108864"
    );
}

#[test]
fn array_sparse_fill_and_lazy_iterators() {
    assert_eq!(
        one(r#"
const N = 2 ** 26;
const fl = [1]; fl[N] = 'x'; fl[N + 3] = 'y'; fl.fill(0, N, N + 2);
console.log('T11', fl.length, Object.keys(fl).join(','), fl[N], fl[N + 1]);
const ent = [1]; ent[N] = 'x'; const it = ent.entries();
console.log('T12', JSON.stringify(it.next()), JSON.stringify(ent.keys().next()), JSON.stringify(ent.values().next()));
"#),
        "T11 67108868 0,67108864,67108865,67108867 0 0|T12 {\"value\":[0,1],\"done\":false} {\"value\":0,\"done\":false} {\"value\":1,\"done\":false}"
    );
}

// Copying methods on holes in a dense array: the result has no holes (`toSorted`, `toReversed`, `with`,
// `toSpliced` read undefined for a hole); `copyWithin` deletes its target for a hole in the source.
#[test]
fn array_dense_holes_copying_methods_match_node() {
    assert_eq!(
        one(r#"
const dh = [1, , 3, , 5];
console.log('T13', JSON.stringify([dh.toSorted(), dh.toReversed(), dh.with(0, 9), dh.toSpliced(0, 1)]), Object.keys(dh.toSorted()).length, Object.keys(dh.toReversed()).length, Object.keys(dh.with(0, 9)).length, Object.keys(dh.toSpliced(0, 1)).length);
const cw = [1, , 3, , 5]; cw.copyWithin(0, 1, 3);
console.log('T14', cw.length, Object.keys(cw).join(','), JSON.stringify(cw));
"#),
        "T13 [[1,3,5,null,null],[5,null,3,null,1],[9,null,3,null,5],[null,3,null,5]] 5 5 5 4|T14 5 1,2,4 [null,3,3,null,5]"
    );
}

#[test]
fn array_dense_copy_within_sealed_hole_throws() {
    assert_eq!(
        one(r#"
const sealDense = [1, , 3, 4]; Object.seal(sealDense);
try { sealDense.copyWithin(0, 1, 2); console.log('no-throw'); } catch (e) { console.log('T16', e.constructor.name, e.message); }
"#),
        "T16 TypeError Cannot delete property '0' of [object Array]"
    );
}

#[test]
fn array_dense_flat_flat_map_and_sort_skip_holes() {
    assert_eq!(
        one(r#"
const dfl = [1, , [3, , 4], , 5];
console.log('T15', JSON.stringify(dfl.flat()), Object.keys(dfl.flat()).join(','), JSON.stringify([1, , 3].flatMap(v => [v])), Object.keys([1, , 3].flatMap(v => [v])).join(','));
const sortD = [3, , 1]; sortD.sort();
console.log('T17', JSON.stringify(sortD), Object.keys(sortD).join(','));
"#),
        "T15 [1,3,4,5] 0,1,2,3 [1,3] 0,1|T17 [1,3,null] 0,1"
    );
}

// The prelude methods above take array-like receivers too (`Array.prototype.sort.call(obj)`), as Node does.
#[test]
fn array_like_receivers_of_prelude_array_methods_match_node() {
    assert_eq!(
        one(r#"
const al = () => ({ length: 3, 0: 'b', 2: 'a' });
console.log('sort', JSON.stringify(((o) => { Array.prototype.sort.call(o); return [o[0], o[1], o[2], o.length]; })(al())));
console.log('toSorted', JSON.stringify(Array.prototype.toSorted.call(al())));
console.log('toReversed', JSON.stringify(Array.prototype.toReversed.call(al())));
console.log('with', JSON.stringify(Array.prototype.with.call(al(), 0, 'z')));
console.log('toSpliced', JSON.stringify(Array.prototype.toSpliced.call(al(), 0, 1)));
console.log('flat', JSON.stringify(Array.prototype.flat.call(al())));
console.log('flatMap', JSON.stringify(Array.prototype.flatMap.call(al(), v => [v])));
console.log('copyWithin', JSON.stringify(((o) => { Array.prototype.copyWithin.call(o, 0, 2); return [o[0], o[1], o[2]]; })(al())));
console.log('slice', JSON.stringify(Array.prototype.slice.call(al())));
console.log('fill', JSON.stringify(((o) => { Array.prototype.fill.call(o, 'f'); return o; })(al())));
console.log('sort-cmp', JSON.stringify(((o) => { Array.prototype.sort.call(o, (x, y) => x < y ? 1 : -1); return [o[0], o[1], o[2]]; })(al())));
"#),
        r#"sort ["a","b",null,3]|toSorted ["a","b",null]|toReversed ["a",null,"b"]|with ["z",null,"a"]|toSpliced [null,"a"]|flat ["b","a"]|flatMap ["b","a"]|copyWithin ["a",null,"a"]|slice ["b",null,"a"]|fill {"0":"f","1":"f","2":"f","length":3}|sort-cmp ["b","a",null]"#
    );
}

// The copying and sorting builtins refuse an exotic array with a sparse tail: their work is O(length), so the
// stopgap `RangeError` stands in. Node returns a value for each of these; that difference is deliberate.
#[test]
fn array_sparse_copying_and_sorting_methods_refuse_sparse_tail() {
    for call in [
        "big.sort()",
        "big.toSorted()",
        "big.toReversed()",
        "big.with(0, 1)",
        "big.toSpliced(0, 1)",
        "big.copyWithin(0, 2 ** 26, 2 ** 26 + 4)",
        "big.flat()",
        "big.flatMap(v => [v])",
    ] {
        assert_eq!(
            one(&format!("const N = 2 ** 26; const big = [1]; big[N] = 'x'; big[N + 3] = 'y'; try {{ {call}; console.log('no-throw'); }} catch (x) {{ console.log(x.constructor.name, x.message); }}")),
            "RangeError Invalid array length",
            "{call}"
        );
    }
}
