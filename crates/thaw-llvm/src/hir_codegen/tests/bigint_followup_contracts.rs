// Follow-ups to arbitrary-precision bigint: value equality for heap-backed handles, exact
// bigint/number ordering, Map/Set keys, array search/join/sort, FFI struct fields. Expected
// output recorded from node 22.

#[test]
fn bigint_followup_contract_equality_search_and_collection_keys_compare_by_value() {
    let source = r##"function main(): void {
  const a = 2n ** 70n;
  const b = 2n ** 70n;
  console.log(a === b, a !== b, a == b, a != b);
  const arr: bigint[] = [a, 3n, 2n ** 80n];
  console.log(arr.includes(b), arr.indexOf(b), arr.lastIndexOf(b), arr.includes(2n ** 80n), arr.indexOf(5n), arr.includes(3n));
  switch (b) { case 1n: console.log("one"); break; case a: console.log("a"); break; default: console.log("def"); }
  console.log(Object.is(a, b), Object.is(a, a + 1n));
  const s = new Set<bigint>([a, b, 2n ** 71n, 2n ** 71n]);
  console.log(s.size, s.has(2n ** 70n), s.has(2n ** 72n));
  const m = new Map<bigint, string>();
  m.set(a, "x"); m.set(b, "y"); m.set(5n, "z");
  console.log(m.size, m.get(2n ** 70n), m.has(2n ** 70n), m.get(5n));
  m.delete(2n ** 70n);
  console.log(m.size, [...s.values()].length, [...m.keys()]);
}
"##;
    assert_eq!(
        compile_and_run(source, "bigint_followup_equality"),
        "true false true false\ntrue 0 0 true -1 true\na\ntrue false\n2 true false\n2 y true z\n1 2 [ 5n ]\n"
    );
}

#[test]
fn bigint_followup_contract_union_and_nullable_members_compare_by_value() {
    let source = r##"function main(): void {
  const t = 2n ** 70n;
  const v: bigint | string = 2n ** 70n;
  console.log(v === t, v === 2n ** 70n, v !== t, typeof v);
  const n: bigint | null = 2n ** 70n;
  console.log(n === t, n !== t, n === 2n ** 71n);
  const u: bigint | undefined = 2n ** 70n;
  console.log(u === t, u === 2n ** 71n);
  const arr: (bigint | null)[] = [2n ** 70n, null, 5n];
  console.log(arr[0] === t, arr[1] === t, arr[2] === 5n, arr[1] === null);
  let m: bigint | string = "x";
  console.log(m === t);
  m = 2n ** 70n;
  console.log(m === t);
}
"##;
    assert_eq!(
        compile_and_run(source, "bigint_followup_union_equality"),
        "true true false bigint\ntrue false false\ntrue false\ntrue false true true\nfalse\ntrue\n"
    );
}

#[test]
fn bigint_followup_contract_number_comparison_is_exact_above_2_pow_53() {
    let source = r##"function main(): void {
  const big = 2n ** 60n + 1n;
  const f = 2 ** 60;
  console.log(big > f, big < f, big == f, big >= f, big <= f);
  console.log(9007199254740993n > 9007199254740992, 9007199254740993n == 9007199254740992, 9007199254740993n < 9007199254740994);
  const h = 2n ** 80n;
  console.log(h > 1e24, h < 1.3e24, h == 2 ** 80, h > 2 ** 80, 2 ** 80 < h, 2 ** 80 == h, 1e24 < h);
  console.log(1n < NaN, 1n > NaN, 1n == NaN, 1n < Infinity, -(2n ** 70n) > -Infinity);
  console.log(9007199254740993n === BigInt(9007199254740993), 5n == 5.5, 5n < 5.5, 10n >= 10, 10n <= 10.0001);
}
"##;
    assert_eq!(
        compile_and_run(source, "bigint_followup_number_compare"),
        "true false false true false\ntrue false true\ntrue true true false false true true\nfalse false false true true\nfalse false true true true\n"
    );
}

#[test]
fn bigint_followup_contract_join_sort_valueof_format_and_range_error_text() {
    let source = r##"function main(): void {
  const a = 2n ** 70n;
  console.log([a, 3n, -a].join(), [a, 3n].join("|"), String([a, 3n]), `${[a, 1n]}`, [a] + "");
  console.log([a, 3n, 10n, -a, 2n].sort(), [10n, 9n, 1n].toSorted(), [a].valueOf(), a.valueOf() === a);
  console.log("%d %i %s", a, a, a);
  for (const bad of [1.5, NaN, Infinity, -0.5]) {
    try { BigInt(bad); } catch (e) { console.log((e as Error).name, (e as Error).message); }
  }
}
"##;
    assert_eq!(
        compile_and_run(source, "bigint_followup_join_sort"),
        "1180591620717411303424,3,-1180591620717411303424 1180591620717411303424|3 1180591620717411303424,3 1180591620717411303424,1 1180591620717411303424\n[ -1180591620717411303424n, 10n, 1180591620717411303424n, 2n, 3n ] [ 1n, 10n, 9n ] [ 1180591620717411303424n ] true\n1180591620717411303424n 1180591620717411303424n 1180591620717411303424n\nRangeError The number 1.5 cannot be converted to a BigInt because it is not an integer\nRangeError The number NaN cannot be converted to a BigInt because it is not an integer\nRangeError The number Infinity cannot be converted to a BigInt because it is not an integer\nRangeError The number -0.5 cannot be converted to a BigInt because it is not an integer\n"
    );
}

#[test]
fn bigint_followup_contract_ffi_struct_int64_fields_round_trip_raw_integers() {
    let source = r#"
        declare function native_pair(): { id: bigint; w: number };
        declare function native_pair_sum(p: { id: bigint; w: number }): bigint;
        function main(): void {
            const p = native_pair();
            console.log(p.id, p.id === 9223372036854775807n, p.id + 1n, p.w);
            const q = native_pair_sum({ id: 4611686018427387904n, w: 2 });
            console.log(q, q === 4611686018427387906n);
            console.log(native_pair_sum({ id: -5n, w: 1 }));
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let mut program = thaw_hir::lower_module(&module).unwrap();
    for (name, params) in [
        ("native_pair", vec![]),
        ("native_pair_sum", vec![thaw_hir::FfiStringAbi::NullTerminated]),
    ] {
        thaw_hir::set_ffi_string_abi(
            &mut program,
            name,
            params,
            thaw_hir::FfiStringAbi::NullTerminated,
            thaw_hir::FfiCallingConvention::C,
            thaw_hir::FfiAggregateAbi::Portable,
        )
        .unwrap();
    }
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "ffi_bigint_struct");
    compiler.compile_program(&program).unwrap();
    let dir = std::env::temp_dir().join(format!("thaw-ffi-bigint-struct-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let obj_path = dir.join("out.o");
    let exe_path = dir.join("out");
    let native_c_path = dir.join("native.c");
    let native_obj_path = dir.join("native.o");
    compiler.write_object_file(&obj_path).unwrap();
    std::fs::write(
        &native_c_path,
        "#include <stdint.h>\n\
         typedef struct { int64_t id; double w; } Pair;\n\
         Pair native_pair(void) { return (Pair){INT64_MAX, 1.5}; }\n\
         int64_t native_pair_sum(Pair p) { return p.id + (int64_t)p.w; }\n",
    )
    .unwrap();
    assert!(cc_command().arg("-c").arg(&native_c_path).arg("-o").arg(&native_obj_path).status().unwrap().success());
    let arena_lib = build_staticlib("thaw-arena");
    let std_lib = build_staticlib("thaw-std");
    let runtime_lib = build_staticlib("thaw-runtime");
    assert!(cc_command()
        .arg(&obj_path)
        .arg(&native_obj_path)
        .arg(&arena_lib)
        .arg(&std_lib)
        .arg(&runtime_lib)
        .arg(&std_lib)
        .arg(&runtime_lib)
        .arg(build_staticlib("thaw-quickjs"))
        .arg("-lm")
        .arg("-o")
        .arg(&exe_path)
        .status()
        .unwrap()
        .success());
    let output = Command::new(&exe_path).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "9223372036854775807n true 9223372036854775808n 1.5\n4611686018427387906n true\n-4n\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn bigint_followup_contract_uncaught_heap_backed_bigint_reports_its_digits() {
    let source = r##"function main(): void {
  throw 2n ** 70n;
}
"##;
    let output = compile_and_run_raw(source, "bigint_followup_uncaught", &[]);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("1180591620717411303424"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
