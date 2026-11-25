//! Step-wise iteration with IteratorClose in the Map/Set/WeakMap/WeakSet constructors, `Array.from` and the
//! Promise combinators, in the AST interpreter and the closure compiler.

use crate::compile::CompileMode;
use crate::Interp;

fn run(src: &str) -> String {
    let mut want: Option<Vec<String>> = None;
    for mode in [CompileMode::Off, CompileMode::On] {
        let mut it = Interp::with_compile_mode(mode);
        it.compile_after = 1;
        it.run_sloppy(src)
            .unwrap_or_else(|_| panic!("{mode:?}: must run: {src}"));
        it.run_jobs().unwrap();
        let got = std::mem::take(&mut it.out);
        match &want {
            None => want = Some(got),
            Some(w) => assert_eq!(&got, w, "{mode:?} differs: {src}"),
        }
    }
    want.unwrap().join("|")
}

/// An infinite generator that counts `return()` calls in `closed`.
const GEN: &str = "var closed = 0; function* inf() { try { var i = 0; while (true) yield i++; } finally { closed++; } }\n";

#[test]
fn collection_ctors_close_an_infinite_iterator_when_the_adder_throws() {
    for (ctor, adder, item) in [
        ("Map", "set", "[1, 2]"),
        ("Set", "add", "1"),
        ("WeakMap", "set", "[{}, 2]"),
        ("WeakSet", "add", "{}"),
    ] {
        let src = format!(
            "{GEN} var it = (function* () {{ try {{ while (true) yield {item}; }} finally {{ closed++; }} }})();\n\
             var orig = {ctor}.prototype.{adder}; {ctor}.prototype.{adder} = function () {{ throw new RangeError('x'); }};\n\
             try {{ new {ctor}(it); }} catch (e) {{ console.log(e.name, closed); }}"
        );
        assert_eq!(run(&src), "RangeError 1", "{ctor}");
    }
}

#[test]
fn map_ctor_closes_on_a_non_entry_value_and_reads_the_adder_once_before_iterating() {
    let src = "var closed = 0, log = [];
        var it = { [Symbol.iterator]() { return this; }, next() { return { value: 1, done: false }; }, return() { closed++; return {}; } };
        try { new Map(it); } catch (e) { log.push(e.name, closed); }
        var m = Map.prototype.set; var n = 0;
        Object.defineProperty(Map.prototype, 'set', { get() { n++; return m; }, configurable: true });
        new Map([[1, 2], [3, 4]]); log.push(n);
        console.log(log.join(','));";
    assert_eq!(run(src), "TypeError,1,1");
}

#[test]
fn array_from_maps_while_stepping_and_closes_on_a_throwing_map_fn() {
    let src = format!(
        "{GEN} try {{ Array.from(inf(), function (v) {{ if (v === 3) throw new Error('m'); return v; }}); }} catch (e) {{ console.log(e.message, closed); }}\n\
         console.log(Array.from({{ length: 2, 0: 'a', 1: 'b' }}, function (v, i) {{ return v + i + this.s; }}, {{ s: '!' }}).join());"
    );
    assert_eq!(run(&src), "m 1|a0!,b1!");
}

#[test]
fn array_from_and_ctors_honour_a_replaced_array_iterator() {
    let src = "var orig = Array.prototype[Symbol.iterator];
        Array.prototype[Symbol.iterator] = function* () { yield 'patched'; };
        console.log(Array.from([1, 2]).join(), new Set([1, 2]).size);
        Array.prototype[Symbol.iterator] = orig;
        console.log(Array.from([1, 2]).join(), new Set([1, 2]).size);";
    assert_eq!(run(src), "patched 1|1,2 2");
}

#[test]
fn promise_combinators_close_when_resolve_throws_and_reject_on_iteration_errors() {
    for name in ["all", "allSettled", "any", "race"] {
        let src = format!(
            "{GEN} var r = Promise.resolve; Promise.resolve = function () {{ throw new TypeError('boom'); }};\n\
             Promise.{name}(inf()).then(null, function (e) {{ Promise.resolve = r; console.log(e.message, closed); }});"
        );
        assert_eq!(run(&src), "boom 1", "{name}");
    }
}

#[test]
fn promise_combinators_reject_on_a_non_callable_resolve_before_iterating_and_on_non_iterables() {
    let src = "var r = Promise.resolve, touched = 0;
        Promise.resolve = 1;
        var it = { [Symbol.iterator]() { touched++; return { next() { return { done: true }; } }; } };
        var a = Promise.all(it).then(null, function (e) { Promise.resolve = r; return e.name + touched; });
        var b = Promise.race(5).then(null, function (e) { return e.name; });
        Promise.resolve = r;
        var c = Promise.all([1, Promise.resolve(2)]).then(function (v) { return v.join(); });
        var d = Promise.any([]).then(null, function (e) { return e.name; });
        var e = Promise.allSettled([1, Promise.reject(2)]).then(function (v) { return v.map(function (x) { return x.status; }).join(); });
        Promise.all([a, b, c, d, e]).then(function (v) { console.log(v.join('|')); });";
    assert_eq!(
        run(src),
        "TypeError0|TypeError|1,2|AggregateError|fulfilled,rejected"
    );
}
