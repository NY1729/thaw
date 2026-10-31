// Audit-2 leftovers: compound field writes compiled in place, and native
// `process.hrtime*` that never creates the JS realm. Expected output is from node 22.

#[test]
fn compound_field_and_element_writes_run_in_place_and_match_node() {
    let source = r#"class B { x: number; vx: number; m: number; constructor(x: number, vx: number, m: number) { this.x = x; this.vx = vx; this.m = m; } }
function main(): void {
  const b = [new B(1, 2, 3), new B(4, 5, 6)];
  let order = "";
  const idx = (i: number): number => { order += "i" + i; return i; };
  const val = (v: number): number => { order += "v" + v; return v; };
  b[idx(0)].vx -= val(1) * b[1].m;
  b[idx(1)].x += 2;
  b[0].m *= 3;
  b[1].m /= 2;
  const r = (b[0].vx -= 1);
  const arr = [1, 2, 3];
  arr[idx(2)] += val(10);
  arr[0] -= arr[1] * 2;
  console.log(b[0].x, b[0].vx, b[0].m, b[1].x, b[1].vx, b[1].m, r, arr.join(","), order);
  let e = 0;
  for (let step = 0; step < 1000; step++) {
    for (let i = 0; i < 2; i++) for (let j = 0; j < 2; j++) {
      const d = b[i].x - b[j].x;
      b[i].vx -= d * 0.001;
      b[j].vx += d * 0.001;
    }
    e += b[0].vx;
  }
  console.log(b[0].vx.toFixed(6), b[1].vx.toFixed(6), e.toFixed(6));
}
"#;
    let (ir, _) = compile_to_ir_and_quickjs_use(source, "perf_audit2_compound_ir");
    let body = function_ir(&ir, "thaw_user_main");
    // `d` is read by in-place compound-write bindings only, so it stays a stack slot.
    assert!(
        !body.lines().any(|line| line.contains("%d_cell") && line.contains("thaw_arena_alloc")),
        "loop-local const captured only by in-place bindings must not live in the arena: {body}"
    );
    assert_eq!(
        compile_and_run(source, "perf_audit2_compound_run"),
        "1 -5 9 6 5 3 -5 -3,2,13 i0v1i1i2v10\n5.000000 -5.000000 5.000000\n"
    );
}

#[test]
fn process_hrtime_is_native_and_does_not_create_the_js_realm() {
    let source = r#"function main(): void {
  const t0: number[] = process.hrtime();
  const b0 = process.hrtime.bigint();
  let x = 0;
  for (let i = 0; i < 100000; i++) x += i;
  const d = process.hrtime(t0);
  const b1 = process.hrtime.bigint();
  console.log(typeof b0, b1 >= b0, b0 > 0n, t0.length, d.length, d[0] >= 0, d[1] >= 0 && d[1] < 1e9, Number.isInteger(t0[1]), x);
  const fixed: number[] = [1, 900000000];
  const r = process.hrtime(fixed);
  console.log(r[1] >= 0 && r[1] < 1e9);
}
"#;
    let (_, uses_quickjs) = compile_to_ir_and_quickjs_use(source, "perf_audit2_hrtime_ir");
    assert!(!uses_quickjs, "process.hrtime* must not need the JS realm");
    assert_eq!(
        compile_and_run(source, "perf_audit2_hrtime_run"),
        "bigint true true 2 2 true true true 4999950000\ntrue\n"
    );
}
