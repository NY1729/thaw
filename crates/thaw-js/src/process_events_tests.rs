use super::*;

/// Run `src` as the main script, then the event loop; return the stdout lines joined with `|`.
fn run_loop(src: &str) -> String {
    let mut it = Interp::new();
    it.run(src).expect("script runs");
    it.run_event_loop().expect("loop runs");
    it.out.join("|")
}

// `process` is an EventEmitter instance as in Node 22: `process` -> `process.prototype` ->
// `EventEmitter.prototype` -> `Object.prototype`. Expected values were recorded with Node v22.23.1.

#[test]
fn process_proto_chain_matches_node() {
    assert_eq!(
        run_loop(
            "const p = Object.getPrototypeOf(process);
console.log([p.constructor.name, Object.getPrototypeOf(p).constructor.name, Object.getPrototypeOf(Object.getPrototypeOf(p)) === Object.prototype].join(';'));"
        ),
        "process;EventEmitter;true"
    );
}

#[test]
fn process_own_event_state_like_node() {
    assert_eq!(
        run_loop(
            "const keys = Object.keys(process);
console.log(['_events', '_eventsCount', '_maxListeners'].map(k => keys.includes(k)).join(';'));
console.log(['on', 'emit', 'once', 'listeners', 'removeAllListeners'].map(k => Object.prototype.hasOwnProperty.call(process, k)).join(';'));
console.log([typeof process._eventsCount, String(process._maxListeners)].join(';'));"
        ),
        "true;true;true|false;false;false;false;false|number;undefined"
    );
}

#[test]
fn emitter_methods_typeof_name_length_and_attributes_like_node() {
    assert_eq!(
        run_loop(
            "const names = ['addListener', 'on', 'once', 'prependListener', 'prependOnceListener', 'removeListener', 'off', 'removeAllListeners', 'emit', 'listeners', 'rawListeners', 'listenerCount', 'eventNames', 'setMaxListeners', 'getMaxListeners'];
const base = Object.getPrototypeOf(Object.getPrototypeOf(process));
for (const n of names) console.log([n, typeof process[n], Object.prototype.hasOwnProperty.call(process, n), base.propertyIsEnumerable(n), process[n].name, process[n].length].join(';'));"
        ),
        "addListener;function;false;true;addListener;2|on;function;false;true;addListener;2|once;function;false;true;once;2|prependListener;function;false;true;prependListener;2|prependOnceListener;function;false;true;prependOnceListener;2|removeListener;function;false;true;removeListener;2|off;function;false;true;removeListener;2|removeAllListeners;function;false;true;removeAllListeners;1|emit;function;false;true;emit;1|listeners;function;false;true;listeners;1|rawListeners;function;false;true;rawListeners;1|listenerCount;function;false;true;listenerCount;2|eventNames;function;false;true;eventNames;0|setMaxListeners;function;false;true;setMaxListeners;1|getMaxListeners;function;false;true;getMaxListeners;0"
    );
}

#[test]
fn on_once_off_emit_listeners_roundtrip_like_node() {
    assert_eq!(
        run_loop(
            "const log = [];
const f = (x) => log.push('f' + x);
const g = () => log.push('g');
console.log(process.on('t', f) === process);
process.once('t', g);
console.log(process.listenerCount('t'));
console.log([process.emit('t', 1), process.emit('t', 2)].join(';'));
console.log(log.join(','));
console.log(process.listenerCount('t'));
const raw = process.rawListeners('t');
console.log([raw.length, raw[0] === f].join(';'));
process.once('u', g);
console.log([process.rawListeners('u')[0].listener === g, process.listeners('u')[0] === g].join(';'));
process.off('t', f);
console.log(process.listenerCount('t'));
console.log(process.emit('t'));"
        ),
        "true|2|true;true|f1,g,f2|1|1;true|true;true|0|false"
    );
}

#[test]
fn new_and_remove_listener_events_fire_around_the_change_like_node() {
    assert_eq!(
        run_loop(
            "const seen = [];
const h = () => {};
process.on('newListener', (ev, fn) => seen.push('new:' + String(ev) + ':' + (fn === h)));
process.on('removeListener', (ev, fn) => seen.push('rm:' + String(ev) + ':' + (fn === h)));
process.on('x', h);
process.off('x', h);
console.log(seen.join(','));"
        ),
        "new:removeListener:false,new:x:true,rm:x:true"
    );
}

#[test]
fn emit_error_without_listener_throws_like_node() {
    assert_eq!(
        run_loop(
            "try { process.emit('error', new TypeError('boom')); } catch (e) { console.log(e.name + ':' + e.message); }
try { process.emit('error', 'x'); } catch (e) { console.log(e.name + ':' + e.code + ':' + e.message); }"
        ),
        "TypeError:boom|Error:ERR_UNHANDLED_ERROR:Unhandled error. ('x')"
    );
}

#[test]
fn max_listeners_get_set_validate_and_warn_like_node() {
    assert_eq!(
        run_loop(
            "console.log(process.getMaxListeners());
console.log([process.setMaxListeners(2) === process, process.getMaxListeners()].join(';'));
try { process.setMaxListeners(-1); } catch (e) { console.log([e.name, e.code].join(';')); }
try { process.setMaxListeners('3'); } catch (e) { console.log([e.name, e.code].join(';')); }
process.on('warning', (w) => console.log([w.name, w.count, w.message].join(':')));
process.on('ml', () => {});
process.on('ml', () => {});
process.on('ml', () => {});"
        ),
        "10|true;2|RangeError;ERR_OUT_OF_RANGE|TypeError;ERR_INVALID_ARG_TYPE|MaxListenersExceededWarning:3:Possible EventEmitter memory leak detected. 3 ml listeners added to [process]. MaxListeners is 2. Use emitter.setMaxListeners() to increase limit"
    );
}

#[test]
fn event_names_and_listener_count_with_a_listener_argument_like_node() {
    assert_eq!(
        run_loop(
            "process.on('b', () => {}); process.on('a', () => {}); process.prependListener('p', () => {});
const names = process.eventNames().filter((n) => ['a', 'b', 'p'].includes(n));
console.log(names.join(','));
const fn = () => {};
process.on('c', fn); process.on('c', fn);
console.log([process.listenerCount('c'), process.listenerCount('c', fn), process.listenerCount('c', () => {})].join(';'));"
        ),
        "b,a,p|2;2;0"
    );
}

#[test]
fn exit_listener_calling_process_exit_propagates_and_stops_the_emit() {
    let mut it = Interp::new();
    it.run(
        "process.on('exit', () => { process.exit(7); });
process.on('exit', () => console.log('second-should-not-run'));",
    )
    .expect("script runs");
    assert_eq!(it.run_event_loop().expect("loop returns the code"), 7);
    assert_eq!(it.out.join("|"), "");
}

#[test]
fn patched_process_emit_is_seen_by_exit_like_node() {
    assert_eq!(
        run_loop(
            "const orig = process.emit;
let saw = 'none';
process.emit = function (name, ...args) { if (name === 'exit') saw = args[0]; return orig.call(this, name, ...args); };
let ran = 'none';
process.on('exit', (c) => { ran = c; });
process.on('exit', () => console.log(['exit-ran', saw, ran].join(';')));"
        ),
        "exit-ran;0;0"
    );
}
