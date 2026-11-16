//! `Error.prepareStackTrace` and CallSite objects (PR D2 of the Q9 parity round 3 plan).
//! Generated with node 22.23.1: `PROBE` is the script saved as `callsite_probe.js` and run with `node`; `EXPECTED`
//! is its stdout. Frames carry function names and the constructing flag only (no file, line or column), so
//! CallSite position getters return null.
#[cfg(test)]
use super::*;

const PROBE: &str = r##"const out = [];
const T = (f) => { try { out.push(String(f())); } catch (e) { out.push('THROW ' + e.name + ' ' + e.message); } };
const names = (s) => s.slice(0, 2).map((c) => c.getFunctionName()).join(',');
function inner() { return new Error('z'); }
function outer() { return inner(); }
function g() { return f(); }
function f() { const o = {}; Error.captureStackTrace(o, f); return o.stack; }
class K { constructor() { this.e = new Error('k'); } }
T(() => { Error.prepareStackTrace = (e, s) => 'PREP:' + e.message; return new Error('q').stack; });
T(() => { Error.prepareStackTrace = (e, s) => Array.isArray(s); return new Error('q').stack; });
T(() => { Error.prepareStackTrace = (e, s) => typeof s[0].getFunctionName; return new Error('q').stack; });
T(() => { Error.prepareStackTrace = (e, s) => names(s); return outer().stack; });
T(() => { Error.prepareStackTrace = (e, s) => s[0].isConstructor(); return new K().e.stack; });
T(() => { Error.prepareStackTrace = (e, s) => s[0].getFunctionName(); return f(); });
T(() => { Error.prepareStackTrace = (e, s) => s[0].getFunctionName(); return g(); });
T(() => { Error.prepareStackTrace = (e, s) => s.length; Error.stackTraceLimit = 2; const r = outer().stack; Error.stackTraceLimit = 10; return r; });
T(() => { Error.prepareStackTrace = (e, s) => s.length; Error.stackTraceLimit = 0; const r = new Error('n').stack; Error.stackTraceLimit = 10; return r; });
T(() => { Error.prepareStackTrace = (e, s) => { throw new Error('boom'); }; try { return new Error('x').stack; } catch (err) { return 'caught ' + err.message; } });
T(() => { Error.prepareStackTrace = undefined; return new Error('plain').stack.split('\n')[0]; });
T(() => { Error.prepareStackTrace = (e, s) => typeof s[0].toString(); return outer().stack; });
Error.prepareStackTrace = undefined;
console.log(out.join('\n'));
"##;

/// Node 22 output for `PROBE`.
const EXPECTED: &str = r##"PREP:q
true
function
inner,outer
true
null
g
2
0
caught boom
Error: plain
string"##;

#[test]
fn prepare_stack_trace_and_callsites_match_node() {
    let mut it = Interp::new();
    it.run(PROBE).expect("probe runs");
    assert_eq!(it.out.join("\n"), EXPECTED);
}
