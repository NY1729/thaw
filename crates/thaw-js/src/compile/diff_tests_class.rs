//! Compiled base-class constructors: programs run under `off` (the reference), `on` and `check` with
//! `compile_after = 1`; output must match and functions must really be compiled.

use super::*;
use crate::Interp;

fn run_mode(src: &str, mode: CompileMode) -> (Vec<String>, bool, Interp) {
    let mut it = Interp::with_compile_mode(mode);
    it.compile_after = 1;
    let ok = it.run_sloppy(src).is_ok();
    (it.out.clone(), ok, it)
}

fn same(src: &str, min_compiled: u64) -> Vec<String> {
    let (want, ok_want, _) = run_mode(src, CompileMode::Off);
    for mode in [CompileMode::On, CompileMode::Check] {
        let (got, ok, it) = run_mode(src, mode);
        assert_eq!(ok, ok_want, "{mode:?}: {src}");
        assert_eq!(got, want, "{mode:?}: {src}");
        assert!(
            it.cstats.compiled() >= min_compiled,
            "{mode:?}: compiled {} < {min_compiled}: {src}",
            it.cstats.compiled()
        );
    }
    want
}

#[test]
fn base_class_constructors() {
    let out = same(
        r#"
        class P { x = 1; y; constructor(a, { b = 2 } = {}, ...r) { this.a = a; this.b = b; this.n = r.length; this.sum = this.x + a } }
        class Q { constructor() { return { replaced: true } } }
        class R1 { constructor(v) { if (v < 0) throw new RangeError('neg ' + v); this.v = v } }
        class S { constructor() { try { null.x } catch (e) { this.caught = e.name } finally { this.fin = 1 } } }
        class T { static count = 0; constructor() { T.count++; this.id = T.count } }
        class U { constructor(items) { this.items = []; for (const i of items) this.items.push(i * 2) } }
        class V { #p = 1; constructor() { this.q = this.#p } }
        const arrowIn = class { constructor(o) { const f = () => o; this.v = f() } };
        const p = new P(5, { b: 9 }, 1, 2);
        console.log(JSON.stringify(p), p instanceof P, p.constructor === P, Object.getPrototypeOf(p) === P.prototype);
        console.log(JSON.stringify(new P(1)), JSON.stringify(new Q()), new T().id, new T().id, T.count);
        console.log(JSON.stringify(new S()), JSON.stringify(new U([1, 2])), new V().q, new arrowIn(3).v);
        for (const v of [1, -2, 3]) { try { console.log(new R1(v).v) } catch (e) { console.log(e.name + ': ' + e.message) } }
        try { P() } catch (e) { console.log(e.name) }
        class D extends P { constructor() { super(1); this.d = 2 } }
        class E extends R1 {}
        console.log(JSON.stringify(new D()), new E(4).v);
        try { new E(-1) } catch (e) { console.log(e.message) }
        function nt() { return new.target }
        class W { constructor() { this.t = typeof new.target } }
        console.log(new W().t);
        "#,
        6,
    );
    assert!(
        out[0].starts_with("{\"x\":1,\"a\":5,\"b\":9,\"n\":2,\"sum\":6}"),
        "{}",
        out[0]
    );
}
