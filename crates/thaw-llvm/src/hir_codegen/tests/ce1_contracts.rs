// `new Map()`/`Set()`/`WeakMap()`/`WeakSet()` without type arguments, and the console methods that
// were missing (group indentation, count, table, dir, trace) incl. `console.log` passed as a value.
// Expected output recorded from node 22.

#[test]
fn collections_without_type_arguments_default_to_any() {
    let source = r####"function main(): void {
  const m = new Map();
  m.set("a", 1);
  m.set("b", "x");
  console.log(m.get("a"), m.size, m);
  const s = new Set();
  s.add(1);
  s.add("a");
  console.log(s.size, s.has(1), s.has("zz"), s);
  const k = {};
  const w = new WeakMap();
  w.set(k, 1);
  console.log(w.get(k), w.has(k), w.has({}));
  const ws = new WeakSet();
  ws.add(k);
  console.log(ws.has(k), ws.has({}));
  const typed: Map<string, number> = new Map();
  typed.set("x", 2);
  console.log(typed.get("x"), typed);
  const tset: Set<number> = new Set();
  tset.add(3);
  console.log([...tset]);
}
"####;
    assert_eq!(compile_and_run(source, "collections_without_type_arguments_default_to_any"), r####"1 2 Map(2) { 'a' => 1, 'b' => 'x' }
2 true false Set(2) { 1, 'a' }
1 true false
true false
2 Map(1) { 'x' => 2 }
[ 3 ]
"####);
}

#[test]
fn console_group_count_table_dir_match_node() {
    let (stdout, stderr) = compile_and_run_output(r####"function main(): void {
  console.group("G");
  console.log("inside");
  console.group();
  console.log("deeper\nmultiline");
  console.error("to stderr");
  console.groupEnd();
  console.groupEnd();
  console.groupEnd();
  console.log("out");
  console.group("A", "b");
  console.log();
  console.log("a", "b");
  console.groupEnd();
  console.count();
  console.count("x");
  console.count();
  console.countReset();
  console.count();
  console.countReset("never");
  console.table([{ a: 1, b: "x" }, { a: 2, c: true }]);
  console.table([[1, 2], [3, 4]]);
  console.table([1, 2]);
  console.table("str");
  console.table([{ a: 1, b: { x: 1 } }, { a: "long string here", b: [1, 2, 3, 4, 5] }], ["a"]);
  console.table({ r1: { c: 1 }, r2: { c: 2 } });
  console.dir({ a: { b: { c: { d: 1 } } } }, { depth: 0 });
  console.dir({ a: { b: { c: { d: 1 } } } }, { depth: null });
  console.dir("s");
  console.dir([1, 2]);
  console.timeEnd("nolabel");
  console.log("after");
}
"####, "console_group_count_table_dir_match_node");
    assert_eq!(stdout, r####"G
  inside
    deeper
    multiline
out
A b
  
  a b
default: 1
x: 1
default: 2
default: 1
┌─────────┬───┬─────┬──────┐
│ (index) │ a │ b   │ c    │
├─────────┼───┼─────┼──────┤
│ 0       │ 1 │ 'x' │      │
│ 1       │ 2 │     │ true │
└─────────┴───┴─────┴──────┘
┌─────────┬───┬───┐
│ (index) │ 0 │ 1 │
├─────────┼───┼───┤
│ 0       │ 1 │ 2 │
│ 1       │ 3 │ 4 │
└─────────┴───┴───┘
┌─────────┬────────┐
│ (index) │ Values │
├─────────┼────────┤
│ 0       │ 1      │
│ 1       │ 2      │
└─────────┴────────┘
str
┌─────────┬────────────────────┐
│ (index) │ a                  │
├─────────┼────────────────────┤
│ 0       │ 1                  │
│ 1       │ 'long string here' │
└─────────┴────────────────────┘
┌─────────┬───┐
│ (index) │ c │
├─────────┼───┤
│ r1      │ 1 │
│ r2      │ 2 │
└─────────┴───┘
{ a: [Object] }
{
  a: { b: { c: { d: 1 } } }
}
's'
[ 1, 2 ]
after
"####);
    assert_eq!(stderr, "    to stderr\n");
}

#[test]
fn console_methods_passed_as_values_forward_their_arguments() {
    let (stdout, stderr) = compile_and_run_output(r####"async function f(): Promise<string> {
  return "r";
}
function g(): Promise<number> {
  return Promise.resolve(42);
}
function main(): void {
  f().then(console.log);
  g().then(console.log);
  g().then(console.error);
}
"####, "console_methods_passed_as_values_forward_their_arguments");
    assert_eq!(stdout, r####"r
42
"####);
    assert_eq!(stderr, "42\n");
}

#[test]
fn console_trace_writes_to_stderr_with_group_indent() {
    let (stdout, stderr) = compile_and_run_output(r####"function main(): void {
  console.group("G");
  console.trace("tr", 1);
  console.groupEnd();
  console.trace();
  console.log("after");
}
"####, "console_trace_writes_to_stderr_with_group_indent");
    assert_eq!(stdout, "G\nafter\n");
    // Node appends stack frames after each header line; only the headers are reproduced.
    let headers: Vec<&str> = stderr.lines().filter(|line| line.trim_start().starts_with("Trace")).collect();
    assert_eq!(headers, ["  Trace: tr 1", "Trace"]);
}

#[test]
fn console_time_labels_follow_node() {
    let source = r####"function main(): void {
  console.time("t");
  console.timeEnd("missing");
  console.timeLog("missing");
  console.timeEnd("t");
  console.timeEnd("t");
  console.log("done");
}
"####;
    let (stdout, _) = compile_and_run_output(source, "console_time_labels_follow_node");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2, "{stdout}");
    assert!(lines[0].starts_with("t: ") && lines[0].ends_with("ms"), "{stdout}");
    assert_eq!(lines[1], "done");
}
