// Node-verified contracts for inspect/format fidelity: JSON.parse/stringify error wording,
// subclass tags, promise state and circular markers. Expected text recorded from Node v22.

#[test]
fn audit_inspect_f1_json_parse() {
    let source = r####"function main(): void { const bad = ["", "{", "[", "{a:1}", "[1,]", "{\"a\":1,}", "undefined", "'x'", "01", "-", "1.", "1e", "\"abc", "tru", "trux", "{\"a\" 1}", "[1 2]", "1 2", "xxxxxxxxxxxxxxxxxxxxxxxxxxxxx", "[1,2,3,4,5,6,7,8,9,10,x]", "{\"a\":\"\\x\"}", "{\n  \"a\": 1,\n  b\n}"]; for (const text of bad) { try { JSON.parse(text); } catch (e) { console.log((e as Error).name + ": " + (e as Error).message); } } }
"####;
    assert_eq!(compile_and_run(source, "audit_inspect_f1_json_parse"), r####"SyntaxError: Unexpected end of JSON input
SyntaxError: Expected property name or '}' in JSON at position 1 (line 1 column 2)
SyntaxError: Unexpected end of JSON input
SyntaxError: Expected property name or '}' in JSON at position 1 (line 1 column 2)
SyntaxError: Unexpected token ']', "[1,]" is not valid JSON
SyntaxError: Expected double-quoted property name in JSON at position 7 (line 1 column 8)
SyntaxError: "undefined" is not valid JSON
SyntaxError: Unexpected token ''', "'x'" is not valid JSON
SyntaxError: Unexpected number in JSON at position 1 (line 1 column 2)
SyntaxError: No number after minus sign in JSON at position 1 (line 1 column 2)
SyntaxError: Unterminated fractional number in JSON at position 2 (line 1 column 3)
SyntaxError: Exponent part is missing a number in JSON at position 2 (line 1 column 3)
SyntaxError: Unterminated string in JSON at position 4 (line 1 column 5)
SyntaxError: Unexpected end of JSON input
SyntaxError: Unexpected token 'x', "trux" is not valid JSON
SyntaxError: Expected ':' after property name in JSON at position 5 (line 1 column 6)
SyntaxError: Expected ',' or ']' after array element in JSON at position 3 (line 1 column 4)
SyntaxError: Unexpected non-whitespace character after JSON at position 2 (line 1 column 3)
SyntaxError: Unexpected token 'x', "xxxxxxxxxx"... is not valid JSON
SyntaxError: Unexpected token 'x', ...",7,8,9,10,x]" is not valid JSON
SyntaxError: Bad escaped character in JSON at position 7 (line 1 column 8)
SyntaxError: Expected double-quoted property name in JSON at position 14 (line 3 column 3)
"####);
}

#[test]
fn audit_inspect_f2_json_circular() {
    let source = r####"function show(v: any): void {
  try {
    JSON.stringify(v);
  } catch (e) {
    console.log((e as Error).name);
    console.log((e as Error).message);
  }
}
function main(): void {
  const a: Record<string, any> = {};
  a["self"] = a;
  show(a);
  const b: any = { inner: { list: [1, { back: null }] } };
  b.inner.list[1].back = b;
  show(b);
  const c: any[] = [1];
  c.push({ c });
  show(c);
  const d: Record<string, any> = { x: { y: { z: { w: {} } } } };
  d.x.y.z.w.loop = d.x;
  show(d);
  const o: any = {};
  o.self = o;
  show(o);
}
"####;
    assert_eq!(compile_and_run(source, "audit_inspect_f2_json_circular"), r####"TypeError
Converting circular structure to JSON
    --> starting at object with constructor 'Object'
    --- property 'self' closes the circle
TypeError
Converting circular structure to JSON
    --> starting at object with constructor 'Object'
    |     property 'inner' -> object with constructor 'Object'
    |     property 'list' -> object with constructor 'Array'
    |     index 1 -> object with constructor 'Object'
    --- property 'back' closes the circle
TypeError
Converting circular structure to JSON
    --> starting at object with constructor 'Array'
    |     index 1 -> object with constructor 'Object'
    --- property 'c' closes the circle
TypeError
Converting circular structure to JSON
    --> starting at object with constructor 'Object'
    |     property 'y' -> object with constructor 'Object'
    |     property 'z' -> object with constructor 'Object'
    |     property 'w' -> object with constructor 'Object'
    --- property 'loop' closes the circle
TypeError
Converting circular structure to JSON
    --> starting at object with constructor 'Object'
    --- property 'self' closes the circle
"####);
}

#[test]
fn audit_inspect_f3_subclass_tag() {
    let source = r####"class DefMap extends Map<string, number> { get(k: string): number { return super.get(k) ?? 0; } } class SetThing extends Set<number> {} class Setx extends Set<number> {} class XMapper extends Map<string, number> {} class Plain extends Map<string, number> {} function main(): void { console.log(new DefMap([["a", 1]]), new SetThing([1]), new Setx([2]), new XMapper(), new Plain([["k", 3]])); }
"####;
    assert_eq!(compile_and_run(source, "audit_inspect_f3_subclass_tag"), r####"DefMap(1) { 'a' => 1 } SetThing(1) { 1 } Setx(1) [Set] { 2 } XMapper(0) [Map] {} Plain(1) [Map] { 'k' => 3 }
"####);
}

#[test]
fn audit_inspect_f4_promise_state() {
    let source = r####"async function later(): Promise<number> {
  return 5;
}
function main(): void {
  const r = Promise.reject(3);
  r.catch(() => {});
  console.log(Promise.resolve(1), new Promise<void>(() => {}), Promise.resolve("s"), Promise.resolve({ a: [1, 2] }), r, later());
  const e = Promise.reject("why");
  e.catch(() => {});
  console.log(e);
  const t = Promise.reject(true);
  t.catch(() => {});
  console.log(t, Promise.resolve([1, 2, 3]), Promise.resolve(null as string | null), Promise.resolve(undefined));
}
"####;
    assert_eq!(compile_and_run(source, "audit_inspect_f4_promise_state"), r####"Promise { 1 } Promise { <pending> } Promise { 's' } Promise { { a: [ 1, 2 ] } } Promise { <rejected> 3 } Promise { 5 }
Promise { <rejected> 'why' }
Promise { <rejected> true } Promise { [ 1, 2, 3 ] } Promise { null } Promise { undefined }
"####);
}

#[test]
fn audit_inspect_f5_circular_console() {
    let source = r####"function main(): void {
  const o: any = {};
  o.self = o;
  o.arr = [o];
  console.log(o);
  const x: any = { name: "x", child: { up: null } };
  x.child.up = x;
  console.log(x);
  console.log("%s", { a: 1 }, "%s", null, undefined, 5n);
}
"####;
    assert_eq!(compile_and_run(source, "audit_inspect_f5_circular_console"), r####"<ref *1> { self: [Circular *1], arr: [ [Circular *1] ] }
<ref *1> { name: 'x', child: { up: [Circular *1] } }
{ a: 1 } %s null undefined 5n
"####);
}

