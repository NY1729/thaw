use super::*;

/// Run `src` as the main script, then the event loop; return the stdout lines joined with `|`.
fn run_loop(src: &str) -> String {
    let mut it = Interp::new();
    it.run(src).expect("script runs");
    it.run_event_loop().expect("loop runs");
    it.out.join("|")
}

// RegExp.prototype accessors, flags, source escaping and compile. Expected values recorded with Node v22.23.1.

#[test]
fn regexp_proto_accessors_like_node() {
    assert_eq!(
        run_loop(
            r##"const P = RegExp.prototype;
const keys = ['dotAll','flags','global','hasIndices','ignoreCase','multiline','source','sticky','unicode','unicodeSets'];
console.log(keys.map(k => { const d = Object.getOwnPropertyDescriptor(P, k); return [typeof d.get, d.set, d.enumerable, d.configurable, d.get.name, d.get.length].join(';'); }).join('|'));
const c = Object.getOwnPropertyDescriptor(P, 'compile');
console.log([c.writable, c.enumerable, c.configurable, c.value.length, c.value.name].join(';'));
console.log([P.global, P.source, P.flags, String(P), P.dotAll, P.sticky].map(String).join(';'));"##
        ),
        r##"function;;false;true;get dotAll;0|function;;false;true;get flags;0|function;;false;true;get global;0|function;;false;true;get hasIndices;0|function;;false;true;get ignoreCase;0|function;;false;true;get multiline;0|function;;false;true;get source;0|function;;false;true;get sticky;0|function;;false;true;get unicode;0|function;;false;true;get unicodeSets;0|true;false;true;2;compile|undefined;(?:);;/(?:)/;undefined;undefined"##
    );
}

#[test]
fn regexp_proto_getters_throw_on_non_regexp_like_node() {
    assert_eq!(
        run_loop(
            r##"const P = RegExp.prototype; const out = [];
for (const k of ['global', 'source', 'sticky', 'dotAll', 'unicodeSets']) {
  try { Object.getOwnPropertyDescriptor(P, k).get.call({}); out.push('no'); }
  catch (e) { out.push(e.constructor.name); }
}
try { Object.getOwnPropertyDescriptor(P, 'flags').get.call(1); } catch (e) { out.push(e.constructor.name); }
out.push(Object.getOwnPropertyDescriptor(P, 'flags').get.call({}));
console.log(out.join('|'));"##
        ),
        r##"TypeError|TypeError|TypeError|TypeError|TypeError|TypeError|"##
    );
}

#[test]
fn regexp_sticky_unicode_dotall_flags_work_on_instances() {
    assert_eq!(
        run_loop(
            r##"console.log([/a/y.sticky, /a/d.hasIndices, /a/v.unicodeSets, /a/s.dotAll, /a/u.unicode, /a/.global].join(';'));
console.log([/a/gimsuyd.flags, /a/yg.flags, new RegExp('a', 'vi').flags, /a/i.ignoreCase, /a/m.multiline, /a/g.global].join(';'));
const st = /b/y;
console.log([st.test('ab'), st.lastIndex, /a/y.test('ab')].join(';'));"##
        ),
        r##"true;true;true;true;true;false|dgimsuy;gy;iv;true;true;true|false;0;true"##
    );
}

#[test]
fn regexp_flags_getter_is_generic_like_node() {
    assert_eq!(
        run_loop(
            r##"const get = Object.getOwnPropertyDescriptor(RegExp.prototype, 'flags').get;
console.log([get.call({ global: 1, sticky: 1, hasIndices: 1 }), get.call({ hasIndices: true, global: 0, unicodeSets: true, ignoreCase: 'x' }), get.call({}), RegExp.prototype.flags].join(';'));"##
        ),
        r##"dgy;div;;"##
    );
}

#[test]
fn regexp_source_escapes_like_node() {
    assert_eq!(
        run_loop(
            r##"console.log([new RegExp('/').source, new RegExp('a\nb').source, String(new RegExp('/')), new RegExp('[/]').source, new RegExp('').source, RegExp.prototype.source, String(RegExp.prototype)].join(';'));
console.log([new RegExp('\\/').source, new RegExp('\\\\/').source, new RegExp('a b').source, new RegExp(/a\/b/).source].join(';'));"##
        ),
        r##"\/;a\nb;/\//;[/];(?:);(?:);/(?:)/|\/;\\\/;a\u2028b;a\/b"##
    );
}

#[test]
fn regexp_compile_like_node() {
    assert_eq!(
        run_loop(
            r##"const b = /a/g; const r = b.compile('b', 'g');
console.log([r === b, b.source, b.flags, b.lastIndex, b.global, b.sticky].join(';'));
const z = /a/g; z.lastIndex = 3; z.compile('b');
console.log([z.lastIndex, z.flags, z.source].join(';'));
console.log(/a/.compile(/q/i).flags);
try { /a/.compile(/q/i, 'g'); } catch (e) { console.log(e.constructor.name); }
try { RegExp.prototype.compile.call({}, 'a'); } catch (e) { console.log(e.constructor.name); }
console.log([RegExp.prototype.compile.length, RegExp.prototype.compile.name].join(';'));"##
        ),
        r##"true;b;g;0;true;false|0;;b|i|TypeError|TypeError|2;compile"##
    );
}
