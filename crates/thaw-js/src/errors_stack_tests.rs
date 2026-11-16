//! `Error.captureStackTrace` with a string `stack` and the `Error.prepareStackTrace` default (PR D1 of the Q9
//! parity round 3 plan). The expected text is Node 22 output for `PROBE`; frames are not compared (a string
//! stack has no frames, only the header and one `at <anonymous>` line).
#[cfg(test)]
use super::*;

const PROBE: &str = r##"const out = [];
const T = (f) => { try { out.push(String(f())); } catch (e) { out.push('THROW ' + e.name + ' ' + e.message); } };
const dsc = (o, k) => { const d = Object.getOwnPropertyDescriptor(o, k); if (!d) return 'NONE'; return (d.writable ? 'w' : '-') + (d.enumerable ? 'e' : '-') + (d.configurable ? 'c' : '-') + ' ' + typeof d.value; };
// Error.prepareStackTrace default (plain node 22: a named function, not undefined)
out.push(typeof Error.prepareStackTrace, dsc(Error, 'prepareStackTrace'), Error.prepareStackTrace.name, Error.prepareStackTrace.length);
out.push(typeof Error.prepareStackTrace.prototype, Error.prepareStackTrace.prototype.constructor === Error.prepareStackTrace);
out.push(dsc(Error, 'captureStackTrace'), Error.captureStackTrace.length, Error.captureStackTrace.name);
// captureStackTrace on plain objects: header is `${name}: ${message}` with Error.prototype.toString-style defaults
const o = { name: 'Custom', message: 'hi' };
Error.captureStackTrace(o);
out.push(typeof o.stack, o.stack.split('\n')[0], Object.getOwnPropertyNames(o).join(','), Object.keys(o).length);
o.stack = 'x';
out.push(o.stack);
const p = {};
Error.captureStackTrace(p);
out.push(Object.getOwnPropertyNames(p).join(','), Object.keys(p).length, p.stack.split('\n')[0]);
out.push(({ message: 'm' }, (() => { const e = { message: 'm' }; Error.captureStackTrace(e); return e.stack.split('\n')[0]; })()));
out.push((() => { const e = { name: 'N' }; Error.captureStackTrace(e); return e.stack.split('\n')[0]; })());
out.push((() => { const e = { name: '', message: 'only' }; Error.captureStackTrace(e); return e.stack.split('\n')[0]; })());
out.push((() => { const e = new TypeError('t'); Error.captureStackTrace(e); return e.stack.split('\n')[0]; })());
function f() { const x = {}; Error.captureStackTrace(x, f); return typeof x.stack; }
out.push(f());
const fo = function Named() {};
Error.captureStackTrace(fo);
out.push(typeof fo.stack, fo.stack.split('\n')[0]);
T(() => Error.captureStackTrace(undefined));
T(() => Error.captureStackTrace(null));
T(() => Error.captureStackTrace(1));
T(() => Error.captureStackTrace('s'));
T(() => Error.captureStackTrace());
console.log(out.join('\n'));
"##;

/// Node 22 output for `PROBE`.
const EXPECTED: &str = r##"function
w-c function
ErrorPrepareStackTrace
2
object
true
w-c function
2
captureStackTrace
string
Custom: hi
name,message,stack
2
x
stack
0
Error
Error: m
N
only
TypeError: t
string
string
Named
THROW TypeError invalid_argument
THROW TypeError invalid_argument
THROW TypeError invalid_argument
THROW TypeError invalid_argument
THROW TypeError invalid_argument"##;

#[test]
fn error_capture_stack_trace_and_prepare_stack_trace_match_node() {
    let mut it = Interp::new();
    it.run(PROBE).expect("probe runs");
    assert_eq!(it.out.join("\n"), EXPECTED);
}

/// Not in the Node probe: Node installs `stack` as an accessor, this engine as a data property (the
/// writable/configurable/non-enumerable data shape is the contract chosen for PR D1).
#[test]
fn error_capture_stack_trace_defines_writable_data_stack() {
    let mut it = Interp::new();
    it.run("const o = {}; Error.captureStackTrace(o); const d = Object.getOwnPropertyDescriptor(o, 'stack'); console.log([d.writable, d.enumerable, d.configurable, typeof d.value].join(' '));").expect("runs");
    assert_eq!(it.out.join("\n"), "true false true string");
}
