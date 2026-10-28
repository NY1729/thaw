// Buffer type-erasure leak matrix: Buffers reached through Promise, async, containers,
// destructuring, closures and generics must keep byte-buffer element access. Expected output from node 22.

#[test]
fn buf_erase_promise_then_write_matches_node() {
    let source = r#"

function main(): void {
  const b = Buffer.from("ab");
  Promise.resolve(b).then((q) => { q[0] = 65; });
  setTimeout(() => console.log(b.toString(), b.length), 0);
}
"#;
    let output = compile_and_run(source, "buf_erase_promise_then_write_matches_node");
    assert_eq!(output, "Ab 2\n");
}

#[test]
fn buf_erase_promise_then_read_matches_node() {
    let source = r#"

function main(): void {
  const b = Buffer.from("ab");
  Promise.resolve(b).then((q) => console.log(q[1], q.length));
}
"#;
    let output = compile_and_run(source, "buf_erase_promise_then_read_matches_node");
    assert_eq!(output, "98 2\n");
}

#[test]
fn buf_erase_promise_then_return_bytes_matches_node() {
    let source = r#"

function main(): void {
  const b = Buffer.from("ab");
  Promise.resolve(b).then((q) => q).then((r) => { r[0] = 67; console.log(b.toString(), r.toString("hex")); });
}
"#;
    let output = compile_and_run(source, "buf_erase_promise_then_return_bytes_matches_node");
    assert_eq!(output, "Cb 4362\n");
}

#[test]
fn buf_erase_async_return_write_matches_node() {
    let source = r#"

function main(): void {
  async function mk(): Promise<Buffer> { return Buffer.from("hi"); }
  (async () => { const x = await mk(); x[0] = 90; console.log(x.toString(), x[1]); })();
}
"#;
    let output = compile_and_run(source, "buf_erase_async_return_write_matches_node");
    assert_eq!(output, "Zi 105\n");
}

#[test]
fn buf_erase_async_param_write_matches_node() {
    let source = r#"
async function w(v: Buffer): Promise<void> { v[0] = 90; }
function main(): void {
  const b = Buffer.from("hi");
  (async () => { await w(b); console.log(b.toString(), b[0]); })();
}
"#;
    let output = compile_and_run(source, "buf_erase_async_param_write_matches_node");
    assert_eq!(output, "Zi 90\n");
}

#[test]
fn buf_erase_array_of_array_write_matches_node() {
    let source = r#"

function main(): void {
  const b = Buffer.from("ab");
  const bb: Buffer[][] = [[b]];
  bb[0][0][0] = 65;
  console.log(b.toString(), bb[0][0].length);
}
"#;
    let output = compile_and_run(source, "buf_erase_array_of_array_write_matches_node");
    assert_eq!(output, "Ab 2\n");
}

#[test]
fn buf_erase_map_value_write_matches_node() {
    let source = r#"

function main(): void {
  const m = new Map<string, Buffer>();
  const b = Buffer.from("ab");
  m.set("k", b);
  const v = m.get("k")!;
  v[0] = 65;
  console.log(b.toString(), v[1]);
}
"#;
    let output = compile_and_run(source, "buf_erase_map_value_write_matches_node");
    assert_eq!(output, "Ab 98\n");
}

#[test]
fn buf_erase_map_array_write_matches_node() {
    let source = r#"

function main(): void {
  const m = new Map<string, Buffer[]>();
  const b = Buffer.from("ab");
  m.set("k", [b]);
  m.get("k")![0][0] = 65;
  console.log(b.toString(), m.get("k")![0][1]);
}
"#;
    let output = compile_and_run(source, "buf_erase_map_array_write_matches_node");
    assert_eq!(output, "Ab 98\n");
}

#[test]
fn buf_erase_array_destructure_write_matches_node() {
    let source = r#"

function main(): void {
  const b = Buffer.from("ab");
  const [x] = [b];
  x[0] = 65;
  console.log(b.toString(), x[1]);
}
"#;
    let output = compile_and_run(source, "buf_erase_array_destructure_write_matches_node");
    assert_eq!(output, "Ab 98\n");
}

#[test]
fn buf_erase_object_destructure_write_matches_node() {
    let source = r#"

function main(): void {
  const b = Buffer.from("ab");
  const { v } = { v: b };
  v[0] = 65;
  console.log(b.toString(), v.length);
}
"#;
    let output = compile_and_run(source, "buf_erase_object_destructure_write_matches_node");
    assert_eq!(output, "Ab 2\n");
}

#[test]
fn buf_erase_closure_capture_write_matches_node() {
    let source = r#"

function main(): void {
  const b = Buffer.from("ab");
  const f = (): number => { b[0] = 65; return b[0]; };
  console.log(f(), b.toString());
}
"#;
    let output = compile_and_run(source, "buf_erase_closure_capture_write_matches_node");
    assert_eq!(output, "65 Ab\n");
}

#[test]
fn buf_erase_object_field_write_matches_node() {
    let source = r#"

function main(): void {
  const o = { buf: Buffer.from("ab") };
  o.buf[0] = 65;
  console.log(o.buf.toString(), o.buf[1]);
}
"#;
    let output = compile_and_run(source, "buf_erase_object_field_write_matches_node");
    assert_eq!(output, "Ab 98\n");
}

#[test]
fn buf_erase_generic_identity_write_matches_node() {
    let source = r#"
function id<T>(x: T): T { return x; }
function main(): void {
  const b = Buffer.from("ab");
  const y = id(b);
  y[0] = 65;
  console.log(b.toString(), y[1]);
}
"#;
    let output = compile_and_run(source, "buf_erase_generic_identity_write_matches_node");
    assert_eq!(output, "Ab 98\n");
}

#[test]
fn buf_erase_ternary_write_matches_node() {
    let source = r#"

function main(): void {
  const b = Buffer.from("ab");
  const t = b.length > 0 ? b : Buffer.from("zz");
  t[0] = 65;
  console.log(b.toString(), t.toString());
}
"#;
    let output = compile_and_run(source, "buf_erase_ternary_write_matches_node");
    assert_eq!(output, "Ab Ab\n");
}

#[test]
fn buf_erase_promise_all_write_matches_node() {
    let source = r#"

function main(): void {
  const b = Buffer.from("ab");
  Promise.all([b]).then(([q]) => { q[0] = 65; console.log(b.toString()); });
}
"#;
    let output = compile_and_run(source, "buf_erase_promise_all_write_matches_node");
    assert_eq!(output, "Ab\n");
}

#[test]
fn buf_erase_tuple_write_matches_node() {
    let source = r#"

function main(): void {
  const b = Buffer.from("ab");
  const t: [Buffer, number] = [b, 1];
  t[0][0] = 65;
  console.log(b.toString(), t[1]);
}
"#;
    let output = compile_and_run(source, "buf_erase_tuple_write_matches_node");
    assert_eq!(output, "Ab 1\n");
}

#[test]
fn buf_erase_set_iter_write_matches_node() {
    let source = r#"

function main(): void {
  const b = Buffer.from("ab");
  const s = new Set<Buffer>([b]);
  for (const x of s) x[0] = 65;
  console.log(b.toString(), s.size);
}
"#;
    let output = compile_and_run(source, "buf_erase_set_iter_write_matches_node");
    assert_eq!(output, "Ab 1\n");
}

#[test]
fn buf_erase_array_push_write_matches_node() {
    let source = r#"

function main(): void {
  const b = Buffer.from("ab");
  const arr: Buffer[] = [];
  arr.push(b);
  arr[0][0] = 65;
  console.log(b.toString(), arr[0][1]);
}
"#;
    let output = compile_and_run(source, "buf_erase_array_push_write_matches_node");
    assert_eq!(output, "Ab 98\n");
}

#[test]
fn buf_erase_promise_nested_array_write_matches_node() {
    let source = r#"

function main(): void {
  const b = Buffer.from("ab");
  Promise.resolve([b]).then((a) => { a[0][0] = 65; console.log(b.toString(), a[0].length); });
}
"#;
    let output = compile_and_run(source, "buf_erase_promise_nested_array_write_matches_node");
    assert_eq!(output, "Ab 2\n");
}

#[test]
fn buf_erase_promise_object_write_matches_node() {
    let source = r#"

function main(): void {
  const b = Buffer.from("ab");
  Promise.resolve({ v: b }).then((o) => { o.v[0] = 65; console.log(b.toString(), o.v[1]); });
}
"#;
    let output = compile_and_run(source, "buf_erase_promise_object_write_matches_node");
    assert_eq!(output, "Ab 98\n");
}

#[test]
fn buf_erase_promise_then_read_bytes_value_matches_node() {
    let source = r#"

function main(): void {
  const b = Buffer.from("ab");
  Promise.resolve(b).then((q) => q[0] + q[1]).then((s) => console.log(s));
}
"#;
    let output = compile_and_run(source, "buf_erase_promise_then_read_bytes_value_matches_node");
    assert_eq!(output, "195\n");
}

#[test]
fn buf_erase_optional_write_matches_node() {
    let source = r#"

function main(): void {
  const b = Buffer.from("ab");
  const ob: Buffer | undefined = b;
  ob![0] = 65;
  console.log(b.toString());
}
"#;
    let output = compile_and_run(source, "buf_erase_optional_write_matches_node");
    assert_eq!(output, "Ab\n");
}

#[test]
fn buf_erase_fn_returns_array_write_matches_node() {
    let source = r#"

function main(): void {
  const b = Buffer.from("ab");
  function arr(): Buffer[] { return [b]; }
  arr()[0][0] = 65;
  console.log(b.toString());
}
"#;
    let output = compile_and_run(source, "buf_erase_fn_returns_array_write_matches_node");
    assert_eq!(output, "Ab\n");
}

#[test]
fn buf_erase_for_of_grid_write_matches_node() {
    let source = r#"

function main(): void {
  const rows: Buffer[] = [Buffer.from("ab"), Buffer.from("cd")];
  for (const row of rows) row[0] = 88;
  console.log(rows[0].toString(), rows[1].toString());
}
"#;
    let output = compile_and_run(source, "buf_erase_for_of_grid_write_matches_node");
    assert_eq!(output, "Xb Xd\n");
}

#[test]
fn buf_erase_copywithin_receiver_matches_node() {
    let source = r#"

function main(): void {
  const b = Buffer.from("abcd");
  const u = new Uint8Array(b.buffer, b.byteOffset, b.length);
  Promise.resolve(b).then((q) => { q.copyWithin(0, 2); console.log(b.toString("hex"), u[0]); });
}
"#;
    let output = compile_and_run(source, "buf_erase_copywithin_receiver_matches_node");
    assert_eq!(output, "63646364 99\n");
}

#[test]
fn buf_erase_tosorted_receiver_matches_node() {
    let source = r#"

function main(): void {
  const b = Buffer.from("cab");
  Promise.resolve(b).then((q) => { const s = q.toSorted(); console.log(s.join(","), b.join(",")); });
}
"#;
    let output = compile_and_run(source, "buf_erase_tosorted_receiver_matches_node");
    assert_eq!(output, "97,98,99 99,97,98\n");
}

#[test]
fn buf_erase_map_of_map_buffer_matches_node() {
    let source = r#"

function main(): void {
  const m = new Map<string, Map<number, Buffer>>();
  const inner = new Map<number, Buffer>();
  const b = Buffer.from("ab");
  inner.set(1, b);
  m.set("x", inner);
  m.get("x")!.get(1)![0] = 65;
  console.log(b.toString(), m.get("x")!.get(1)!.length);
}
"#;
    let output = compile_and_run(source, "buf_erase_map_of_map_buffer_matches_node");
    assert_eq!(output, "Ab 2\n");
}

#[test]
fn buf_erase_await_in_loop_write_matches_node() {
    let source = r#"

function main(): void {
  const b = Buffer.from("ab");
  (async () => { for (let i = 0; i < 2; i++) { const q = await Promise.resolve(b); q[i] = 65 + i; } console.log(b.toString()); })();
}
"#;
    let output = compile_and_run(source, "buf_erase_await_in_loop_write_matches_node");
    assert_eq!(output, "AB\n");
}

#[test]
fn buf_erase_closure_array_element_write_matches_node() {
    let source = r#"

function main(): void {
  const b = Buffer.from("ab");
  const arr: Buffer[] = [b];
  const f = () => { arr[0][0] = 70; };
  f();
  console.log(b.toString());
}
"#;
    let output = compile_and_run(source, "buf_erase_closure_array_element_write_matches_node");
    assert_eq!(output, "Fb\n");
}

#[test]
fn buf_erase_map_values_iter_write_matches_node() {
    let source = r#"

function main(): void {
  const m = new Map<string, Buffer>();
  const b = Buffer.from("ab");
  m.set("k", b);
  for (const v of m.values()) v[0] = 65;
  console.log(b.toString(), m.size);
}
"#;
    let output = compile_and_run(source, "buf_erase_map_values_iter_write_matches_node");
    assert_eq!(output, "Ab 1\n");
}

#[test]
fn buf_erase_spread_array_write_matches_node() {
    let source = r#"

function main(): void {
  const b = Buffer.from("ab");
  const arr: Buffer[] = [b];
  const copy = [...arr];
  copy[0][0] = 65;
  console.log(b.toString(), copy.length);
}
"#;
    let output = compile_and_run(source, "buf_erase_spread_array_write_matches_node");
    assert_eq!(output, "Ab 1\n");
}

#[test]
fn buf_erase_async_arrow_return_write_matches_node() {
    let source = r#"

function main(): void {
  const b = Buffer.from("ab");
  const f = async () => b;
  f().then((q) => { q[0] = 65; console.log(b.toString(), q[1]); });
}
"#;
    let output = compile_and_run(source, "buf_erase_async_arrow_return_write_matches_node");
    assert_eq!(output, "Ab 98\n");
}

#[test]
fn buf_erase_promise_race_write_matches_node() {
    let source = r#"

function main(): void {
  const b = Buffer.from("ab");
  Promise.race([Promise.resolve(b)]).then((q) => { q[0] = 65; console.log(b.toString()); });
}
"#;
    let output = compile_and_run(source, "buf_erase_promise_race_write_matches_node");
    assert_eq!(output, "Ab\n");
}

#[test]
fn buf_erase_generic_array_identity_write_matches_node() {
    let source = r#"
function idArr<T>(xs: T[]): T[] { return xs; }
function main(): void {
  const b = Buffer.from("ab");
  const xs = idArr([b]);
  xs[0][0] = 65;
  console.log(b.toString(), xs.length);
}
"#;
    let output = compile_and_run(source, "buf_erase_generic_array_identity_write_matches_node");
    assert_eq!(output, "Ab 1\n");
}

#[test]
fn buf_erase_closure_return_buffer_write_matches_node() {
    let source = r#"

function main(): void {
  const b = Buffer.from("ab");
  const g = (): Buffer => b;
  g()[0] = 65;
  console.log(b.toString());
}
"#;
    let output = compile_and_run(source, "buf_erase_closure_return_buffer_write_matches_node");
    assert_eq!(output, "Ab\n");
}

#[test]
fn buf_erase_object_array_field_write_matches_node() {
    let source = r#"

function main(): void {
  const b = Buffer.from("ab");
  const o: { list: Buffer[] } = { list: [b] };
  o.list[0][0] = 65;
  console.log(b.toString(), o.list.length);
}
"#;
    let output = compile_and_run(source, "buf_erase_object_array_field_write_matches_node");
    assert_eq!(output, "Ab 1\n");
}

#[test]
fn buf_erase_generic_fn_return_write_matches_node() {
    let source = r#"
function wrap<T>(x: T): { v: T } { return { v: x }; }
function main(): void {
  const b = Buffer.from("ab");
  const y = wrap(b).v;
  y[0] = 65;
  console.log(b.toString());
}
"#;
    let output = compile_and_run(source, "buf_erase_generic_fn_return_write_matches_node");
    assert_eq!(output, "Ab\n");
}
