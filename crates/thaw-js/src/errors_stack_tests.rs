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

/// `call_frames` across suspensions (#684). A coroutine keeps its own frames while suspended and gets
/// them back on resume; the frames of whoever resumes it sit below them. Node also lists native frames
/// (`next`) and async-chain frames, which this engine does not record, so the expected text is thaw's.
const CALL_FRAMES_PROBE: &str = r##"Error.prepareStackTrace = (e, s) => s.map((c) => c.getFunctionName()).filter(Boolean).join(',');
const out = [];
async function co() { await null; return new Error('a').stack; }
async function inner() { await null; return new Error('b').stack; }
async function outer() { await null; const s = await inner(); return [s, new Error('c').stack].join(' | '); }
function* g() { yield 1; yield new Error('d').stack; }
function* gen4() { yield 0; yield new Error('e').stack; }
function pull(it) { return it.next().value; }
function leaf() { return new Error('f').stack; }
async function callsLeaf() { await null; return leaf(); }
async function cap() { await null; const o = {}; Error.captureStackTrace(o); return o.stack; }
function hostAfter(it) { pull(it); return new Error('h').stack; }
(async () => {
  out.push('P1 ' + await co());
  out.push('P2 ' + await outer());
  const it = g(); it.next(); out.push('P3 ' + it.next().value);
  const gi = gen4(); gi.next(); out.push('P4 ' + pull(gi));
  out.push('P5 ' + await callsLeaf());
  out.push('P6 ' + await cap());
  out.push('P7 ' + hostAfter(gen4()));
  console.log(out.join('\n'));
})();
"##;

/// Thaw's output for `CALL_FRAMES_PROBE`: every coroutine's own frame is in its stack after each resume.
const CALL_FRAMES_EXPECTED: &str = "P1 co
P2 inner | outer
P3 g
P4 gen4,pull
P5 leaf,callsLeaf
P6 cap
P7 hostAfter";

#[test]
fn call_frames_stay_with_their_coroutine_across_awaits_and_yields() {
    let mut it = Interp::new();
    it.run_main(CALL_FRAMES_PROBE).expect("probe runs");
    it.flush_partial();
    assert_eq!(it.out.join("\n"), CALL_FRAMES_EXPECTED);
}
