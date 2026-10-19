
/// docs/design/bridge.md's first vertical slice: an ambient `declare
/// function` call actually links against and calls a real native
/// implementation -- here a tiny hand-written C function, standing in
/// for what would eventually be a thaw-registry-fetched library.
#[test]
fn compiles_ambient_declaration_and_links_a_real_native_function() {
    let source = r#"
        declare function native_add(a: number, b: number): number;

        function main(): void {
            console.log(native_add(2, 3));
        }
    "#;

    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    assert_eq!(
        program.extern_functions.len(),
        1,
        "sanity: this is really an FFI call"
    );

    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "ffi_ambient");
    compiler.compile_program(&program).unwrap();

    let dir = std::env::temp_dir().join(format!(
        "thaw-hir-codegen-test-ffi_ambient-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let obj_path = dir.join("out.o");
    let exe_path = dir.join("out");
    let native_c_path = dir.join("native.c");
    let native_obj_path = dir.join("native.o");

    compiler.write_object_file(&obj_path).unwrap();

    std::fs::write(
        &native_c_path,
        "double native_add(double a, double b) { return a + b; }\n",
    )
    .unwrap();
    let cc_status = cc_command()
        .arg("-c")
        .arg(&native_c_path)
        .arg("-o")
        .arg(&native_obj_path)
        .status()
        .expect("failed to invoke `cc` to build the native stand-in library");
    assert!(cc_status.success(), "compiling native.c failed");

    let arena_lib = build_staticlib("thaw-arena");
    let std_lib = build_staticlib("thaw-std");
    let runtime_lib = build_staticlib("thaw-runtime");
    let link_status = cc_command()
        .arg(&obj_path)
        .arg(&native_obj_path)
        .arg(&arena_lib)
        .arg(&std_lib)
        .arg(&runtime_lib)
        .arg(&std_lib)
        .arg(&runtime_lib)
        // A caught-error `console.log` reaches the JsValue console path, which lives in thaw-quickjs.
        .arg(build_staticlib("thaw-quickjs"))
        .arg("-o")
        .arg(&exe_path)
        .status()
        .expect("failed to invoke system `cc` linker");
    assert!(link_status.success(), "linking failed");

    let output = Command::new(&exe_path)
        .output()
        .expect("failed to execute compiled binary");
    assert!(output.status.success(), "binary exited non-zero");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "5\n");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ffi_variadic_number_rest_calls_real_c_varargs() {
    let source = r#"
        declare function native_sum(count: number, ...values: number[]): number;
        declare function native_true_count(count: number, ...values: boolean[]): number;
        declare function native_total_length(count: number, ...values: string[]): number;
        declare function native_handle(): JsValue;
        declare function native_handle_sum(count: number, ...values: JsValue[]): number;
        declare function native_array_total(count: number, ...values: number[][]): number;
        declare function native_string_array_total(count: number, ...values: string[][]): number;
        declare function native_bool_array_total(count: number, ...values: boolean[][]): number;
        declare function native_handle_array_total(count: number, ...values: JsValue[][]): number;
        declare function native_object_total(
            count: number,
            ...values: {
                value: number;
                meta: { enabled: boolean; label: string };
                samples: number[];
            }[]
        ): number;
        declare function native_optional_total(
            count: number, ...values: (number | undefined)[]
        ): number;
        declare function native_nullable_total(
            count: number, ...values: (number | null)[]
        ): number;
        declare function native_nullish_total(
            count: number, ...values: (number | null | undefined)[]
        ): number;
        declare function native_optional_object_total(
            count: number, ...values: ({ value: number } | undefined)[]
        ): number;
        declare function native_sum_i32(count: number, ...values: number[]): number;
        declare function native_sum_i64(count: number, ...values: number[]): number;
        declare function native_sum_u32(count: number, ...values: number[]): number;
        declare function native_sum_u64(count: number, ...values: number[]): number;
        declare function native_integer_call_count(): number;

        async function invalidBeforeAwait(): Promise<void> {
            native_sum_i32(1, 0 / 0);
            await sleep(1);
        }

        async function invalidAfterAwait(): Promise<void> {
            await sleep(1);
            native_sum_i32(1, 0 / 0);
        }

        async function catchBeforeAwait(): Promise<void> {
            try {
                native_sum_i32(1, 0 / 0);
                console.log("unreachable-before");
                await sleep(1);
            } catch (error) {
                console.log(error instanceof RangeError);
            } finally {
                console.log("finally-before");
            }
        }

        async function catchAfterAwait(): Promise<void> {
            try {
                await sleep(1);
                native_sum_i32(1, 0 / 0);
                console.log("unreachable-after");
            } catch (error) {
                console.log(error instanceof RangeError);
            } finally {
                console.log("finally-after");
            }
        }

        async function main(): Promise<void> {
            console.log(native_sum(0));
            console.log(native_sum(3, 2, 3, 5));
            console.log(native_true_count(4, true, false, true, true));
            console.log(native_total_length(3, "thaw", "ffi", "ok"));
            console.log(native_handle_sum(2, native_handle(), native_handle()));
            console.log(native_array_total(2, [1, 2], [3, 4, 5]));
            console.log(native_string_array_total(2, ["a", "bc"], ["def"]));
            console.log(native_bool_array_total(2, [true, false, true], [false, true]));
            console.log(native_handle_array_total(
                2,
                [native_handle()],
                [native_handle(), native_handle()]
            ));
            console.log(native_object_total(
                2,
                { value: 2, meta: { enabled: true, label: "abc" }, samples: [1, 2] },
                { value: 4, meta: { enabled: false, label: "x" }, samples: [3, 4, 5] }
            ));
            console.log(native_optional_total(2, 2, undefined));
            console.log(native_nullable_total(2, 3, null));
            console.log(native_nullish_total(3, 4, null, undefined));
            console.log(native_optional_object_total(2, { value: 5 }, undefined));
            console.log(native_sum_i32(2, 0 - 2, 5));
            console.log(native_sum_i64(2, 0 - 4, 10));
            console.log(native_sum_u32(2, 20, 22));
            console.log(native_sum_u64(2, 40, 2));
            try { native_sum_i32(1, 2147483648); } catch (error) { console.log(error instanceof RangeError); }
            try { native_sum_i64(1, 9223372036854775808); } catch (error) { console.log(error instanceof RangeError); }
            try { native_sum_u32(1, 4294967296); } catch (error) { console.log(error instanceof RangeError); }
            try { native_sum_u64(1, 18446744073709551616); } catch (error) { console.log(error instanceof RangeError); }
            try { native_sum_i32(1, 0 / 0); } catch (error) { console.log(error instanceof RangeError); }
            try { native_sum_i64(1, 1 / 0); } catch (error) { console.log(error instanceof RangeError); }
            try { native_sum_i32(1, -2147483649); } catch (error) { console.log(error instanceof RangeError); }
            let laterArgument: number = 0;
            try { native_sum_i32(2, 2147483648, laterArgument = 1); } catch (error) { console.log(error instanceof RangeError); }
            console.log(laterArgument);
            console.log(native_sum_u32(1, -0.5));
            console.log(native_integer_call_count());
            try { await invalidBeforeAwait(); } catch (error) { console.log(error instanceof RangeError); }
            try { await invalidAfterAwait(); } catch (error) { console.log(error instanceof RangeError); }
            await catchBeforeAwait();
            await catchAfterAwait();
            console.log(native_integer_call_count());
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let mut program = thaw_hir::lower_module(&module).unwrap();
    assert!(program
        .extern_functions
        .iter()
        .any(|signature| signature.variadic == Some(HirType::F64)));
    assert!(program
        .extern_functions
        .iter()
        .any(|signature| signature.variadic == Some(HirType::Bool)));
    assert!(program
        .extern_functions
        .iter()
        .any(|signature| signature.variadic == Some(HirType::Str)));
    assert!(program
        .extern_functions
        .iter()
        .any(|signature| signature.variadic == Some(HirType::JsValue)));
    assert!(program.extern_functions.iter().any(|signature| {
        signature.variadic == Some(HirType::Array(Box::new(HirType::F64)))
    }));
    assert!(program.extern_functions.iter().any(|signature| {
        signature.variadic == Some(HirType::Array(Box::new(HirType::Str)))
    }));
    assert!(program.extern_functions.iter().any(|signature| {
        signature.variadic == Some(HirType::Array(Box::new(HirType::Bool)))
    }));
    assert!(program.extern_functions.iter().any(|signature| {
        signature.variadic == Some(HirType::Array(Box::new(HirType::JsValue)))
    }));
    assert!(program.extern_functions.iter().any(|signature| {
        matches!(&signature.variadic, Some(HirType::Object(fields)) if fields.len() == 3)
    }));
    assert!(program.extern_functions.iter().any(|signature| {
        signature.variadic == Some(HirType::Optional(Box::new(HirType::F64)))
    }));
    assert!(program.extern_functions.iter().any(|signature| {
        signature.variadic == Some(HirType::Nullable(Box::new(HirType::F64)))
    }));
    assert!(program.extern_functions.iter().any(|signature| {
        signature.variadic == Some(HirType::Nullish(Box::new(HirType::F64)))
    }));
    for (symbol, abi) in [
        ("native_sum_i32", thaw_hir::FfiVariadicAbi::I32),
        ("native_sum_i64", thaw_hir::FfiVariadicAbi::I64),
        ("native_sum_u32", thaw_hir::FfiVariadicAbi::U32),
        ("native_sum_u64", thaw_hir::FfiVariadicAbi::U64),
    ] {
        thaw_hir::set_ffi_variadic_abi(&mut program, symbol, abi).unwrap();
    }
    assert!(thaw_hir::set_ffi_variadic_abi(
        &mut program,
        "native_true_count",
        thaw_hir::FfiVariadicAbi::I32,
    )
    .unwrap_err()
    .contains("requires a number[] rest parameter"));

    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "ffi_variadic");
    compiler.compile_program(&program).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "thaw-hir-codegen-test-ffi-variadic-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let obj_path = dir.join("out.o");
    let exe_path = dir.join("out");
    let native_c_path = dir.join("native.c");
    let native_obj_path = dir.join("native.o");
    compiler.write_object_file(&obj_path).unwrap();
    std::fs::write(
        &native_c_path,
        "#include <stdarg.h>\n#include <stdint.h>\n#include <string.h>\n\
         static int integer_calls = 0;\n\
         double native_integer_call_count(void) { return (double)integer_calls; }\n\
         double native_sum(double raw_count, ...) {\n\
           int count = (int)raw_count; double sum = 0; va_list args;\n\
           va_start(args, raw_count);\n\
           for (int i = 0; i < count; ++i) sum += va_arg(args, double);\n\
           va_end(args); return sum;\n\
         }\n\
         double native_true_count(double raw_count, ...) {\n\
           int count = (int)raw_count; int found = 0; va_list args;\n\
           va_start(args, raw_count);\n\
           for (int i = 0; i < count; ++i) found += va_arg(args, int) != 0;\n\
           va_end(args); return found;\n\
         }\n\
         double native_total_length(double raw_count, ...) {\n\
           int count = (int)raw_count; size_t length = 0; va_list args;\n\
           va_start(args, raw_count);\n\
           for (int i = 0; i < count; ++i) length += strlen(va_arg(args, const char *));\n\
           va_end(args); return (double)length;\n\
         }\n\
         uint64_t native_handle(void) { return 20; }\n\
         double native_handle_sum(double raw_count, ...) {\n\
           int count = (int)raw_count; uint64_t sum = 0; va_list args;\n\
           va_start(args, raw_count);\n\
           for (int i = 0; i < count; ++i) sum += va_arg(args, uint64_t);\n\
           va_end(args); return (double)sum;\n\
         }\n\
         double native_array_total(double raw_count, ...) {\n\
           int count = (int)raw_count; double sum = 0; va_list args;\n\
           va_start(args, raw_count);\n\
           for (int i = 0; i < count; ++i) {\n\
             const double *data = va_arg(args, const double *);\n\
             long long length = va_arg(args, long long);\n\
             for (long long j = 0; j < length; ++j) sum += data[j];\n\
           }\n\
           va_end(args); return sum;\n\
         }\n\
         double native_string_array_total(double raw_count, ...) {\n\
           int count = (int)raw_count; double sum = 0; va_list args;\n\
           va_start(args, raw_count);\n\
           for (int i = 0; i < count; ++i) {\n\
             const char *const *data = va_arg(args, const char *const *);\n\
             long long length = va_arg(args, long long);\n\
             for (long long j = 0; j < length; ++j) sum += strlen(data[j]);\n\
           }\n\
           va_end(args); return sum;\n\
         }\n\
         double native_bool_array_total(double raw_count, ...) {\n\
           int count = (int)raw_count; double sum = 0; va_list args;\n\
           va_start(args, raw_count);\n\
           for (int i = 0; i < count; ++i) {\n\
             const int *data = va_arg(args, const int *);\n\
             long long length = va_arg(args, long long);\n\
             for (long long j = 0; j < length; ++j) sum += data[j] != 0;\n\
           }\n\
           va_end(args); return sum;\n\
         }\n\
         double native_handle_array_total(double raw_count, ...) {\n\
           int count = (int)raw_count; uint64_t sum = 0; va_list args;\n\
           va_start(args, raw_count);\n\
           for (int i = 0; i < count; ++i) {\n\
             const uint64_t *data = va_arg(args, const uint64_t *);\n\
             long long length = va_arg(args, long long);\n\
             for (long long j = 0; j < length; ++j) sum += data[j];\n\
           }\n\
           va_end(args); return (double)sum;\n\
         }\n\
         double native_object_total(double raw_count, ...) {\n\
           int count = (int)raw_count; double sum = 0; va_list args;\n\
           va_start(args, raw_count);\n\
           for (int i = 0; i < count; ++i) {\n\
             sum += va_arg(args, double);\n\
             sum += va_arg(args, int) != 0;\n\
             sum += (double)strlen(va_arg(args, const char *));\n\
             const double *data = va_arg(args, const double *);\n\
             long long length = va_arg(args, long long);\n\
             for (long long j = 0; j < length; ++j) sum += data[j];\n\
           }\n\
           va_end(args); return sum;\n\
         }\n\
         double native_optional_total(double raw_count, ...) {\n\
           int count = (int)raw_count; double sum = 0; va_list args;\n\
           va_start(args, raw_count);\n\
           for (int i = 0; i < count; ++i) {\n\
             sum += va_arg(args, int); sum += va_arg(args, double);\n\
           }\n\
           va_end(args); return sum;\n\
         }\n\
         double native_nullable_total(double raw_count, ...) {\n\
           int count = (int)raw_count; double sum = 0; va_list args;\n\
           va_start(args, raw_count);\n\
           for (int i = 0; i < count; ++i) {\n\
             sum += va_arg(args, int); sum += va_arg(args, double);\n\
           }\n\
           va_end(args); return sum;\n\
         }\n\
         double native_nullish_total(double raw_count, ...) {\n\
           int count = (int)raw_count; double sum = 0; va_list args;\n\
           va_start(args, raw_count);\n\
           for (int i = 0; i < count; ++i) {\n\
             sum += va_arg(args, int); sum += va_arg(args, double);\n\
           }\n\
           va_end(args); return sum;\n\
         }\n\
         double native_optional_object_total(double raw_count, ...) {\n\
           int count = (int)raw_count; double sum = 0; va_list args;\n\
           va_start(args, raw_count);\n\
           for (int i = 0; i < count; ++i) {\n\
             sum += va_arg(args, int); sum += va_arg(args, double);\n\
           }\n\
           va_end(args); return sum;\n\
         }\n\
         double native_sum_i32(double raw_count, ...) {\n\
           integer_calls++;\n\
           int count = (int)raw_count; int sum = 0; va_list args;\n\
           va_start(args, raw_count);\n\
           for (int i = 0; i < count; ++i) sum += va_arg(args, int);\n\
           va_end(args); return (double)sum;\n\
         }\n\
         double native_sum_i64(double raw_count, ...) {\n\
           integer_calls++;\n\
           int count = (int)raw_count; long long sum = 0; va_list args;\n\
           va_start(args, raw_count);\n\
           for (int i = 0; i < count; ++i) sum += va_arg(args, long long);\n\
           va_end(args); return (double)sum;\n\
         }\n\
         double native_sum_u32(double raw_count, ...) {\n\
           integer_calls++;\n\
           int count = (int)raw_count; unsigned int sum = 0; va_list args;\n\
           va_start(args, raw_count);\n\
           for (int i = 0; i < count; ++i) sum += va_arg(args, unsigned int);\n\
           va_end(args); return (double)sum;\n\
         }\n\
         double native_sum_u64(double raw_count, ...) {\n\
           integer_calls++;\n\
           int count = (int)raw_count; unsigned long long sum = 0; va_list args;\n\
           va_start(args, raw_count);\n\
           for (int i = 0; i < count; ++i) sum += va_arg(args, unsigned long long);\n\
           va_end(args); return (double)sum;\n\
         }\n",
    )
    .unwrap();
    assert!(cc_command()
        .arg("-c")
        .arg(&native_c_path)
        .arg("-o")
        .arg(&native_obj_path)
        .status()
        .unwrap()
        .success());
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
        // A caught-error `console.log` reaches the JsValue console path, which lives in thaw-quickjs.
        .arg(build_staticlib("thaw-quickjs"))
        .arg("-o")
        .arg(&exe_path)
        .status()
        .unwrap()
        .success());
    let output = Command::new(&exe_path).output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "0\n10\n3\n9\n40\n15\n6\n3\n60\n26\n3\n4\n7\n6\n3\n6\n42\n42\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\n1\n0\n5\ntrue\ntrue\ntrue\nfinally-before\ntrue\nfinally-after\n5\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn ffi_thaw_result_abi_propagates_native_errors_into_try_catch() {
    let source = r#"
        declare function native_read(value: number): number;

        function main(): void {
            console.log(native_read(2));
            try {
                console.log(native_read(0 - 1));
                console.log("unreachable");
            } catch (error) {
                console.log(error);
            } finally {
                console.log("cleanup");
            }
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let mut program = thaw_hir::lower_module(&module).unwrap();
    thaw_hir::set_ffi_error_abi(
        &mut program,
        "native_read",
        thaw_hir::FfiErrorAbi::ThawResult,
    )
    .unwrap();

    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "ffi_result_abi");
    compiler.compile_program(&program).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "thaw-hir-codegen-test-ffi-result-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let obj_path = dir.join("out.o");
    let exe_path = dir.join("out");
    let native_c_path = dir.join("native.c");
    let native_obj_path = dir.join("native.o");
    compiler.write_object_file(&obj_path).unwrap();
    std::fs::write(
        &native_c_path,
        "typedef struct { double value; const char *error; } ThawF64Result;\n\
         ThawF64Result native_read(double value) {\n\
           if (value < 0) return (ThawF64Result){0, \"native read failed\"};\n\
           return (ThawF64Result){value * 10, 0};\n\
         }\n",
    )
    .unwrap();
    assert!(cc_command()
        .arg("-c")
        .arg(&native_c_path)
        .arg("-o")
        .arg(&native_obj_path)
        .status()
        .unwrap()
        .success());
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
        // A caught-error `console.log` reaches the JsValue console path, which lives in thaw-quickjs.
        .arg(build_staticlib("thaw-quickjs"))
        .arg("-o")
        .arg(&exe_path)
        .status()
        .unwrap()
        .success());
    let output = Command::new(&exe_path).output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "20\nnative read failed\ncleanup\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn ffi_void_calls_support_direct_and_thaw_result_error_abis() {
    let source = r#"
        declare function native_mark(value: number): void;
        declare function mark_count(): number;
        declare function native_check(value: number): void;
        declare function error_destroy_count(): number;

        function main(): void {
            native_mark(2);
            console.log(mark_count());
            native_check(1);
            console.log("ok");
            try {
                native_check(0 - 1);
                console.log("unreachable");
            } catch (error) {
                console.log(error);
                console.log(error_destroy_count());
            }
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let mut program = thaw_hir::lower_module(&module).unwrap();
    thaw_hir::set_ffi_error_abi(
        &mut program,
        "native_check",
        thaw_hir::FfiErrorAbi::ThawResult,
    )
    .unwrap();
    thaw_hir::set_ffi_ownership(
        &mut program,
        "native_check",
        thaw_hir::FfiOwnership::Borrowed,
        thaw_hir::FfiOwnership::Owned {
            destroy: "destroy_error".into(),
        },
    )
    .unwrap();

    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "ffi_void_result");
    compiler.compile_program(&program).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "thaw-hir-codegen-test-ffi-void-result-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let obj_path = dir.join("out.o");
    let exe_path = dir.join("out");
    let native_c_path = dir.join("native.c");
    let native_obj_path = dir.join("native.o");
    compiler.write_object_file(&obj_path).unwrap();
    std::fs::write(
        &native_c_path,
        "#include <stdlib.h>\n#include <string.h>\n\
         typedef struct { char *error; } ThawVoidResult;\n\
         static int marks; static int error_destroys;\n\
         static char *copy(const char *s) { size_t n = strlen(s) + 1; char *p = malloc(n); memcpy(p, s, n); return p; }\n\
         void native_mark(double value) { marks += (int)value; }\n\
         double mark_count(void) { return marks; }\n\
         ThawVoidResult native_check(double value) {\n\
           if (value < 0) return (ThawVoidResult){copy(\"void check failed\")};\n\
           return (ThawVoidResult){0};\n\
         }\n\
         void destroy_error(char *p) { ++error_destroys; free(p); }\n\
         double error_destroy_count(void) { return error_destroys; }\n",
    )
    .unwrap();
    assert!(cc_command()
        .arg("-c")
        .arg(&native_c_path)
        .arg("-o")
        .arg(&native_obj_path)
        .status()
        .unwrap()
        .success());
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
        // A caught-error `console.log` reaches the JsValue console path, which lives in thaw-quickjs.
        .arg(build_staticlib("thaw-quickjs"))
        .arg("-o")
        .arg(&exe_path)
        .status()
        .unwrap()
        .success());
    let output = Command::new(&exe_path).output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "2\nok\nvoid check failed\n1\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn ffi_owned_result_strings_are_copied_and_destroyed_once() {
    let source = r#"
        declare function native_read(value: number): string;
        declare function return_destroy_count(): number;
        declare function error_destroy_count(): number;

        function main(): void {
            console.log(native_read(1));
            console.log(return_destroy_count());
            try {
                console.log(native_read(0 - 1));
            } catch (error) {
                console.log(error);
                console.log(error_destroy_count());
            }
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let mut program = thaw_hir::lower_module(&module).unwrap();
    thaw_hir::set_ffi_error_abi(
        &mut program,
        "native_read",
        thaw_hir::FfiErrorAbi::ThawResult,
    )
    .unwrap();
    thaw_hir::set_ffi_ownership(
        &mut program,
        "native_read",
        thaw_hir::FfiOwnership::Owned {
            destroy: "destroy_return".into(),
        },
        thaw_hir::FfiOwnership::Owned {
            destroy: "destroy_error".into(),
        },
    )
    .unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "ffi_owned_result");
    compiler.compile_program(&program).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "thaw-hir-codegen-test-ffi-owned-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let obj_path = dir.join("out.o");
    let exe_path = dir.join("out");
    let native_c_path = dir.join("native.c");
    let native_obj_path = dir.join("native.o");
    compiler.write_object_file(&obj_path).unwrap();
    std::fs::write(
        &native_c_path,
        "#include <stdlib.h>\n#include <string.h>\n\
         typedef struct { char *value; char *error; } ThawStringResult;\n\
         static int return_destroys; static int error_destroys;\n\
         static char *copy(const char *s) { size_t n = strlen(s) + 1; char *p = malloc(n); memcpy(p, s, n); return p; }\n\
         ThawStringResult native_read(double value) {\n\
           if (value < 0) return (ThawStringResult){0, copy(\"owned error\")};\n\
           return (ThawStringResult){copy(\"owned value\"), 0};\n\
         }\n\
         void destroy_return(char *p) { ++return_destroys; free(p); }\n\
         void destroy_error(char *p) { ++error_destroys; free(p); }\n\
         double return_destroy_count(void) { return return_destroys; }\n\
         double error_destroy_count(void) { return error_destroys; }\n",
    )
    .unwrap();
    assert!(cc_command()
        .arg("-c")
        .arg(&native_c_path)
        .arg("-o")
        .arg(&native_obj_path)
        .status()
        .unwrap()
        .success());
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
        // A caught-error `console.log` reaches the JsValue console path, which lives in thaw-quickjs.
        .arg(build_staticlib("thaw-quickjs"))
        .arg("-o")
        .arg(&exe_path)
        .status()
        .unwrap()
        .success());
    let output = Command::new(&exe_path).output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "owned value\n1\nowned error\n1\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn ffi_call_marshals_array_and_object_returns_from_portable_c_structs() {
    let source = r#"
        declare function native_values(): number[];
        declare function native_point(): { x: number; y: number };
        declare function native_vector(): { x: number; y: number; z: number };
        declare function array_destroy_count(): number;

        function main(): void {
            const values: number[] = native_values();
            const point: { x: number; y: number } = native_point();
            const vector: { x: number; y: number; z: number } = native_vector();
            console.log(values[0] + values[1] + values[2]);
            console.log(point.x + point.y);
            console.log(vector.x + vector.y + vector.z);
            console.log(array_destroy_count());
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let mut program = thaw_hir::lower_module(&module).unwrap();
    thaw_hir::set_ffi_ownership(
        &mut program,
        "native_values",
        thaw_hir::FfiOwnership::Owned {
            destroy: "destroy_values".into(),
        },
        thaw_hir::FfiOwnership::Borrowed,
    )
    .unwrap();
    thaw_hir::set_ffi_string_abi(
        &mut program,
        "native_values",
        vec![],
        thaw_hir::FfiStringAbi::NullTerminated,
        thaw_hir::FfiCallingConvention::C,
        thaw_hir::FfiAggregateAbi::Portable,
    )
    .unwrap();
    thaw_hir::set_ffi_string_abi(
        &mut program,
        "native_point",
        vec![],
        thaw_hir::FfiStringAbi::NullTerminated,
        thaw_hir::FfiCallingConvention::C,
        thaw_hir::FfiAggregateAbi::Portable,
    )
    .unwrap();
    thaw_hir::set_ffi_string_abi(
        &mut program,
        "native_vector",
        vec![],
        thaw_hir::FfiStringAbi::NullTerminated,
        thaw_hir::FfiCallingConvention::C,
        thaw_hir::FfiAggregateAbi::Portable,
    )
    .unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "ffi_aggregate_returns");
    compiler.compile_program(&program).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "thaw-hir-codegen-test-ffi-aggregate-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let obj_path = dir.join("out.o");
    let exe_path = dir.join("out");
    let native_c_path = dir.join("native.c");
    let native_obj_path = dir.join("native.o");
    compiler.write_object_file(&obj_path).unwrap();
    std::fs::write(
        &native_c_path,
        "#include <stdint.h>\n#include <stdlib.h>\n\
         typedef struct { double *data; int64_t len; } ThawF64Array;\n\
         typedef struct { double x; double y; } Point;\n\
         typedef struct { double x; double y; double z; } Vector;\n\
         static int destroys;\n\
         ThawF64Array native_values(void) { double *p = malloc(3 * sizeof(double)); p[0] = 2; p[1] = 3; p[2] = 5; return (ThawF64Array){p, 3}; }\n\
         void destroy_values(void *p) { ++destroys; free(p); }\n\
         double array_destroy_count(void) { return destroys; }\n\
         Point native_point(void) { return (Point){7, 11}; }\n\
         Vector native_vector(void) { return (Vector){13, 17, 19}; }\n",
    )
    .unwrap();
    assert!(cc_command()
        .arg("-c")
        .arg(&native_c_path)
        .arg("-o")
        .arg(&native_obj_path)
        .status()
        .unwrap()
        .success());
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
        // A caught-error `console.log` reaches the JsValue console path, which lives in thaw-quickjs.
        .arg(build_staticlib("thaw-quickjs"))
        .arg("-o")
        .arg(&exe_path)
        .status()
        .unwrap()
        .success());
    let output = Command::new(&exe_path).output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "10\n18\n49\n1\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn ffi_call_marshals_object_returns_from_packed_c_structs() {
    let source = r#"
        declare function native_record(): { active: boolean; value: number };
        declare function native_checked_record(value: number): { active: boolean; value: number };

        function main(): void {
            const record: { active: boolean; value: number } = native_record();
            console.log(record.active);
            console.log(record.value);
            const checked: { active: boolean; value: number } = native_checked_record(7);
            console.log(checked.value);
            try {
                native_checked_record(0 - 1);
            } catch (error) {
                console.log(error);
            }
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let mut program = thaw_hir::lower_module(&module).unwrap();
    thaw_hir::set_ffi_string_abi(
        &mut program,
        "native_record",
        vec![],
        thaw_hir::FfiStringAbi::NullTerminated,
        thaw_hir::FfiCallingConvention::C,
        thaw_hir::FfiAggregateAbi::Packed,
    )
    .unwrap();
    thaw_hir::set_ffi_error_abi(
        &mut program,
        "native_checked_record",
        thaw_hir::FfiErrorAbi::ThawResult,
    )
    .unwrap();
    thaw_hir::set_ffi_string_abi(
        &mut program,
        "native_checked_record",
        vec![thaw_hir::FfiStringAbi::NullTerminated],
        thaw_hir::FfiStringAbi::NullTerminated,
        thaw_hir::FfiCallingConvention::C,
        thaw_hir::FfiAggregateAbi::Packed,
    )
    .unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "ffi_packed_return");
    compiler.compile_program(&program).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "thaw-hir-codegen-test-ffi-packed-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let obj_path = dir.join("out.o");
    let exe_path = dir.join("out");
    let native_c_path = dir.join("native.c");
    let native_obj_path = dir.join("native.o");
    compiler.write_object_file(&obj_path).unwrap();
    std::fs::write(
        &native_c_path,
        "typedef struct __attribute__((packed)) { _Bool active; double value; } PackedRecord;\n\
         typedef struct { PackedRecord value; const char *error; } PackedRecordResult;\n\
         PackedRecord native_record(void) { return (PackedRecord){1, 42}; }\n\
         PackedRecordResult native_checked_record(double value) {\n\
           if (value < 0) return (PackedRecordResult){{0, 0}, \"packed check failed\"};\n\
           return (PackedRecordResult){{1, value * 2}, 0};\n\
         }\n",
    )
    .unwrap();
    assert!(cc_command()
        .arg("-c")
        .arg(&native_c_path)
        .arg("-o")
        .arg(&native_obj_path)
        .status()
        .unwrap()
        .success());
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
        // A caught-error `console.log` reaches the JsValue console path, which lives in thaw-quickjs.
        .arg(build_staticlib("thaw-quickjs"))
        .arg("-o")
        .arg(&exe_path)
        .status()
        .unwrap()
        .success());
    let output = Command::new(&exe_path).output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "true\n42\n14\npacked check failed\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn ffi_call_uses_explicit_c_aggregate_offsets_and_alignment() {
    let source = r#"
        declare function native_aligned_record(): { active: boolean; value: number };
        declare function native_checked_aligned_record(value: number): { active: boolean; value: number };
        declare function native_nested_aligned_record(): { meta: { active: boolean; value: number }; total: number };
        declare function native_bit_record(): { active: boolean; ready: boolean; count: number; delta: number; value: number };
        declare function native_register_record(): { active: boolean; value: number };
        declare function native_checked_register_record(value: number): { active: boolean; value: number };

        function main(): void {
            const record: { active: boolean; value: number } = native_aligned_record();
            console.log(record.active);
            console.log(record.value);
            const checked: { active: boolean; value: number } = native_checked_aligned_record(7);
            console.log(checked.value);
            try {
                native_checked_aligned_record(0 - 1);
            } catch (error) {
                console.log(error);
            }
            const nested = native_nested_aligned_record();
            console.log(nested.meta.active);
            console.log(nested.meta.value + nested.total);
            const bits = native_bit_record();
            console.log(bits.active);
            console.log(bits.ready);
            console.log(bits.count);
            console.log(bits.delta);
            console.log(bits.value);
            const register = native_register_record();
            console.log(register.active);
            console.log(register.value);
            const checkedRegister = native_checked_register_record(9);
            console.log(checkedRegister.value);
            try {
                native_checked_register_record(0 - 1);
            } catch (error) {
                console.log(error);
            }
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let mut program = thaw_hir::lower_module(&module).unwrap();
    thaw_hir::set_ffi_string_abi(
        &mut program,
        "native_aligned_record",
        vec![],
        thaw_hir::FfiStringAbi::NullTerminated,
        thaw_hir::FfiCallingConvention::C,
        thaw_hir::FfiAggregateAbi::Portable,
    )
    .unwrap();
    let mut invalid_program = program.clone();
    assert!(thaw_hir::set_ffi_aggregate_layout(
        &mut invalid_program,
        "native_aligned_record",
        thaw_hir::FfiAggregateLayout {
            field_offsets: vec![0, 0],
            field_layouts: vec![None, None],
            field_bitfields: vec![None, None],
            register_classes: vec![],
            size: 16,
            alignment: 8,
            indirect: true,
        },
    )
    .unwrap_err()
    .contains("outside or overlaps"));
    thaw_hir::set_ffi_aggregate_layout(
        &mut program,
        "native_aligned_record",
        thaw_hir::FfiAggregateLayout {
            field_offsets: vec![0, 16],
            field_layouts: vec![None, None],
            field_bitfields: vec![None, None],
            register_classes: vec![],
            size: 32,
            alignment: 32,
            indirect: true,
        },
    )
    .unwrap();
    for symbol in ["native_register_record", "native_checked_register_record"] {
        if symbol == "native_checked_register_record" {
            thaw_hir::set_ffi_error_abi(
                &mut program,
                symbol,
                thaw_hir::FfiErrorAbi::ThawResult,
            )
            .unwrap();
        }
        thaw_hir::set_ffi_string_abi(
            &mut program,
            symbol,
            if symbol == "native_register_record" {
                vec![]
            } else {
                vec![thaw_hir::FfiStringAbi::NullTerminated]
            },
            thaw_hir::FfiStringAbi::NullTerminated,
            thaw_hir::FfiCallingConvention::C,
            thaw_hir::FfiAggregateAbi::Portable,
        )
        .unwrap();
        thaw_hir::set_ffi_aggregate_layout(
            &mut program,
            symbol,
            thaw_hir::FfiAggregateLayout {
                field_offsets: vec![0, 8],
                field_layouts: vec![None, None],
                field_bitfields: vec![None, None],
                register_classes: vec![
                    thaw_hir::FfiRegisterClass::Integer,
                    thaw_hir::FfiRegisterClass::Sse,
                ],
                size: 16,
                alignment: 8,
                indirect: false,
            },
        )
        .unwrap();
    }
    thaw_hir::set_ffi_string_abi(
        &mut program,
        "native_bit_record",
        vec![],
        thaw_hir::FfiStringAbi::NullTerminated,
        thaw_hir::FfiCallingConvention::C,
        thaw_hir::FfiAggregateAbi::Portable,
    )
    .unwrap();
    let mut invalid_bits = program.clone();
    assert!(thaw_hir::set_ffi_aggregate_layout(
        &mut invalid_bits,
        "native_bit_record",
        thaw_hir::FfiAggregateLayout {
            field_offsets: vec![0, 0, 0, 0, 8],
            field_layouts: vec![None, None, None, None, None],
            field_bitfields: vec![
                Some(thaw_hir::FfiBitFieldLayout {
                    bit_offset: 32,
                    bit_width: 1,
                    storage_bytes: 4,
                    signed: false,
                }),
                Some(thaw_hir::FfiBitFieldLayout {
                    bit_offset: 1,
                    bit_width: 1,
                    storage_bytes: 4,
                    signed: false,
                }),
                Some(thaw_hir::FfiBitFieldLayout {
                    bit_offset: 2,
                    bit_width: 5,
                    storage_bytes: 4,
                    signed: false,
                }),
                Some(thaw_hir::FfiBitFieldLayout {
                    bit_offset: 7,
                    bit_width: 6,
                    storage_bytes: 4,
                    signed: true,
                }),
                None,
            ],
            register_classes: vec![],
            size: 32,
            alignment: 32,
            indirect: true,
        },
    )
    .unwrap_err()
    .contains("invalid bit offset"));
    thaw_hir::set_ffi_aggregate_layout(
        &mut program,
        "native_bit_record",
        thaw_hir::FfiAggregateLayout {
            field_offsets: vec![0, 0, 0, 0, 8],
            field_layouts: vec![None, None, None, None, None],
            field_bitfields: vec![
                Some(thaw_hir::FfiBitFieldLayout {
                    bit_offset: 0,
                    bit_width: 1,
                    storage_bytes: 4,
                    signed: false,
                }),
                Some(thaw_hir::FfiBitFieldLayout {
                    bit_offset: 1,
                    bit_width: 1,
                    storage_bytes: 4,
                    signed: false,
                }),
                Some(thaw_hir::FfiBitFieldLayout {
                    bit_offset: 2,
                    bit_width: 5,
                    storage_bytes: 4,
                    signed: false,
                }),
                Some(thaw_hir::FfiBitFieldLayout {
                    bit_offset: 7,
                    bit_width: 6,
                    storage_bytes: 4,
                    signed: true,
                }),
                None,
            ],
            register_classes: vec![],
            size: 32,
            alignment: 32,
            indirect: true,
        },
    )
    .unwrap();
    thaw_hir::set_ffi_string_abi(
        &mut program,
        "native_nested_aligned_record",
        vec![],
        thaw_hir::FfiStringAbi::NullTerminated,
        thaw_hir::FfiCallingConvention::C,
        thaw_hir::FfiAggregateAbi::Portable,
    )
    .unwrap();
    assert!(thaw_hir::set_ffi_aggregate_layout(
        &mut program.clone(),
        "native_nested_aligned_record",
        thaw_hir::FfiAggregateLayout {
            field_offsets: vec![0, 48],
            field_layouts: vec![None, None],
            field_bitfields: vec![None, None],
            register_classes: vec![],
            size: 64,
            alignment: 32,
            indirect: true,
        },
    )
    .unwrap_err()
    .contains("needs a nested field layout"));
    thaw_hir::set_ffi_aggregate_layout(
        &mut program,
        "native_nested_aligned_record",
        thaw_hir::FfiAggregateLayout {
            field_offsets: vec![0, 48],
            field_layouts: vec![
                Some(Box::new(thaw_hir::FfiAggregateLayout {
                    field_offsets: vec![0, 16],
                    field_layouts: vec![None, None],
                    field_bitfields: vec![None, None],
                    register_classes: vec![],
                    size: 32,
                    alignment: 32,
                    indirect: false,
                })),
                None,
            ],
            field_bitfields: vec![None, None],
            register_classes: vec![],
            size: 64,
            alignment: 32,
            indirect: true,
        },
    )
    .unwrap();
    thaw_hir::set_ffi_error_abi(
        &mut program,
        "native_checked_aligned_record",
        thaw_hir::FfiErrorAbi::ThawResult,
    )
    .unwrap();
    thaw_hir::set_ffi_string_abi(
        &mut program,
        "native_checked_aligned_record",
        vec![thaw_hir::FfiStringAbi::NullTerminated],
        thaw_hir::FfiStringAbi::NullTerminated,
        thaw_hir::FfiCallingConvention::C,
        thaw_hir::FfiAggregateAbi::Portable,
    )
    .unwrap();
    thaw_hir::set_ffi_aggregate_layout(
        &mut program,
        "native_checked_aligned_record",
        thaw_hir::FfiAggregateLayout {
            field_offsets: vec![0, 16],
            field_layouts: vec![None, None],
            field_bitfields: vec![None, None],
            register_classes: vec![],
            size: 32,
            alignment: 32,
            indirect: true,
        },
    )
    .unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "ffi_explicit_aggregate_layout");
    compiler.compile_program(&program).unwrap();
    let ir = compiler.module.print_to_string().to_string();
    assert!(ir.contains("alloca <{ i8, [15 x i8], double, [8 x i8] }>, align 32"));
    assert!(ir.contains("declare { i64, double } @native_register_record()"));

    let dir = std::env::temp_dir().join(format!(
        "thaw-hir-codegen-test-ffi-explicit-layout-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let obj_path = dir.join("out.o");
    let exe_path = dir.join("out");
    let native_c_path = dir.join("native.c");
    let native_obj_path = dir.join("native.o");
    compiler.write_object_file(&obj_path).unwrap();
    std::fs::write(
        &native_c_path,
        "#include <stddef.h>\n\
         typedef struct __attribute__((aligned(32))) { _Bool active; char padding[15]; double value; char tail[8]; } AlignedRecord;\n\
         typedef struct { AlignedRecord value; const char *error; } AlignedRecordResult;\n\
         typedef struct __attribute__((aligned(32))) { AlignedRecord meta; char padding[16]; double total; char tail[8]; } NestedAlignedRecord;\n\
         typedef struct __attribute__((aligned(32))) { unsigned active:1; unsigned ready:1; unsigned count:5; signed delta:6; double value; } BitRecord;\n\
         typedef struct { _Bool active; double value; } RegisterRecord;\n\
         typedef struct { RegisterRecord value; const char *error; } RegisterRecordResult;\n\
         _Static_assert(sizeof(BitRecord) == 32, \"unexpected BitRecord size\");\n\
         _Static_assert(offsetof(BitRecord, value) == 8, \"unexpected BitRecord value offset\");\n\
         AlignedRecord native_aligned_record(void) { return (AlignedRecord){1, {0}, 42, {0}}; }\n\
         AlignedRecordResult native_checked_aligned_record(double value) {\n\
           if (value < 0) return (AlignedRecordResult){{0, {0}, 0, {0}}, \"aligned check failed\"};\n\
           return (AlignedRecordResult){{1, {0}, value * 3, {0}}, 0};\n\
         }\n\
         NestedAlignedRecord native_nested_aligned_record(void) {\n\
           return (NestedAlignedRecord){{1, {0}, 20, {0}}, {0}, 22, {0}};\n\
         }\n\
         BitRecord native_bit_record(void) { return (BitRecord){1, 0, 17, -7, 42}; }\n\
         RegisterRecord native_register_record(void) { return (RegisterRecord){1, 55}; }\n\
         RegisterRecordResult native_checked_register_record(double value) {\n\
           if (value < 0) return (RegisterRecordResult){{0, 0}, \"register check failed\"};\n\
           return (RegisterRecordResult){{1, value * 2}, 0};\n\
         }\n",
    )
    .unwrap();
    assert!(cc_command()
        .arg("-c")
        .arg(&native_c_path)
        .arg("-o")
        .arg(&native_obj_path)
        .status()
        .unwrap()
        .success());
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
        // A caught-error `console.log` reaches the JsValue console path, which lives in thaw-quickjs.
        .arg(build_staticlib("thaw-quickjs"))
        .arg("-o")
        .arg(&exe_path)
        .status()
        .unwrap()
        .success());
    let output = Command::new(&exe_path).output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "true\n42\n21\naligned check failed\ntrue\n42\ntrue\nfalse\n17\n-7\n42\ntrue\n55\n18\nregister check failed\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn ffi_object_return_copies_and_destroys_owned_string_fields() {
    let source = r#"
        declare function native_message(): { code: number; message: string };
        declare function native_checked_message(value: number): { code: number; message: string };
        declare function message_destroy_count(): number;

        function main(): void {
            const direct: { code: number; message: string } = native_message();
            const checked: { code: number; message: string } = native_checked_message(2);
            console.log(direct.code);
            console.log(direct.message);
            console.log(checked.message);
            console.log(message_destroy_count());
            try {
                native_checked_message(0 - 1);
            } catch (error) {
                console.log(error);
            }
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let mut program = thaw_hir::lower_module(&module).unwrap();
    for symbol in ["native_message", "native_checked_message"] {
        thaw_hir::set_ffi_ownership(
            &mut program,
            symbol,
            thaw_hir::FfiOwnership::Owned {
                destroy: "destroy_message".into(),
            },
            thaw_hir::FfiOwnership::Borrowed,
        )
        .unwrap();
        thaw_hir::set_ffi_string_abi(
            &mut program,
            symbol,
            if symbol == "native_message" {
                vec![]
            } else {
                vec![thaw_hir::FfiStringAbi::NullTerminated]
            },
            thaw_hir::FfiStringAbi::NullTerminated,
            thaw_hir::FfiCallingConvention::C,
            thaw_hir::FfiAggregateAbi::Portable,
        )
        .unwrap();
    }
    thaw_hir::set_ffi_error_abi(
        &mut program,
        "native_checked_message",
        thaw_hir::FfiErrorAbi::ThawResult,
    )
    .unwrap();

    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "ffi_owned_object_fields");
    compiler.compile_program(&program).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "thaw-hir-codegen-test-ffi-owned-object-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let obj_path = dir.join("out.o");
    let exe_path = dir.join("out");
    let native_c_path = dir.join("native.c");
    let native_obj_path = dir.join("native.o");
    compiler.write_object_file(&obj_path).unwrap();
    std::fs::write(
        &native_c_path,
        "#include <stdlib.h>\n#include <string.h>\n\
         typedef struct { double code; char *message; } Message;\n\
         typedef struct { Message value; const char *error; } MessageResult;\n\
         static int destroys;\n\
         static char *copy(const char *s) { size_t n = strlen(s) + 1; char *p = malloc(n); memcpy(p, s, n); return p; }\n\
         Message native_message(void) { return (Message){1, copy(\"direct message\")}; }\n\
         MessageResult native_checked_message(double value) {\n\
           if (value < 0) return (MessageResult){{0, 0}, \"checked message failed\"};\n\
           return (MessageResult){{value, copy(\"checked message\")}, 0};\n\
         }\n\
         void destroy_message(char *p) { ++destroys; free(p); }\n\
         double message_destroy_count(void) { return destroys; }\n",
    )
    .unwrap();
    assert!(cc_command()
        .arg("-c")
        .arg(&native_c_path)
        .arg("-o")
        .arg(&native_obj_path)
        .status()
        .unwrap()
        .success());
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
        // A caught-error `console.log` reaches the JsValue console path, which lives in thaw-quickjs.
        .arg(build_staticlib("thaw-quickjs"))
        .arg("-o")
        .arg(&exe_path)
        .status()
        .unwrap()
        .success());
    let output = Command::new(&exe_path).output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "1\ndirect message\nchecked message\n2\nchecked message failed\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn ffi_nested_aggregate_returns_are_rebuilt_and_owned_recursively() {
    let source = r#"
        declare function native_nested(): { meta: { label: string; ok: boolean }; values: number[] };
        declare function native_checked_nested(value: number): { meta: { label: string; ok: boolean }; values: number[] };
        declare function nested_destroy_count(): number;

        function main(): void {
            const direct = native_nested();
            console.log(direct.meta.label);
            console.log(direct.meta.ok);
            console.log(direct.values[0] + direct.values[1]);
            console.log(nested_destroy_count());
            const checked = native_checked_nested(3);
            console.log(checked.meta.label);
            console.log(checked.values[0]);
            console.log(nested_destroy_count());
            try {
                native_checked_nested(0 - 1);
            } catch (error) {
                console.log(error);
            }
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let mut program = thaw_hir::lower_module(&module).unwrap();
    for symbol in ["native_nested", "native_checked_nested"] {
        thaw_hir::set_ffi_ownership(
            &mut program,
            symbol,
            thaw_hir::FfiOwnership::Owned {
                destroy: "destroy_nested_leaf".into(),
            },
            thaw_hir::FfiOwnership::Borrowed,
        )
        .unwrap();
        thaw_hir::set_ffi_string_abi(
            &mut program,
            symbol,
            if symbol == "native_nested" {
                vec![]
            } else {
                vec![thaw_hir::FfiStringAbi::NullTerminated]
            },
            thaw_hir::FfiStringAbi::NullTerminated,
            thaw_hir::FfiCallingConvention::C,
            thaw_hir::FfiAggregateAbi::Portable,
        )
        .unwrap();
    }
    thaw_hir::set_ffi_error_abi(
        &mut program,
        "native_checked_nested",
        thaw_hir::FfiErrorAbi::ThawResult,
    )
    .unwrap();

    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "ffi_nested_aggregate");
    compiler.compile_program(&program).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "thaw-hir-codegen-test-ffi-nested-aggregate-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let obj_path = dir.join("out.o");
    let exe_path = dir.join("out");
    let native_c_path = dir.join("native.c");
    let native_obj_path = dir.join("native.o");
    compiler.write_object_file(&obj_path).unwrap();
    std::fs::write(
        &native_c_path,
        "#include <stdint.h>\n#include <stdlib.h>\n#include <string.h>\n\
         typedef struct { double *data; int64_t len; } NumberArray;\n\
         typedef struct { char *label; _Bool ok; } Meta;\n\
         typedef struct { Meta meta; NumberArray values; } Nested;\n\
         typedef struct { Nested value; const char *error; } NestedResult;\n\
         static int destroys;\n\
         static char *copy(const char *s) { size_t n = strlen(s) + 1; char *p = malloc(n); memcpy(p, s, n); return p; }\n\
         static Nested make_nested(const char *label, double first) {\n\
           double *values = malloc(2 * sizeof(double)); values[0] = first; values[1] = 5;\n\
           return (Nested){{copy(label), 1}, {values, 2}};\n\
         }\n\
         Nested native_nested(void) { return make_nested(\"direct nested\", 2); }\n\
         NestedResult native_checked_nested(double value) {\n\
           if (value < 0) return (NestedResult){{{0, 0}, {0, 0}}, \"nested check failed\"};\n\
           return (NestedResult){make_nested(\"checked nested\", value), 0};\n\
         }\n\
         void destroy_nested_leaf(void *p) { ++destroys; free(p); }\n\
         double nested_destroy_count(void) { return destroys; }\n",
    )
    .unwrap();
    assert!(cc_command()
        .arg("-c")
        .arg(&native_c_path)
        .arg("-o")
        .arg(&native_obj_path)
        .status()
        .unwrap()
        .success());
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
        // A caught-error `console.log` reaches the JsValue console path, which lives in thaw-quickjs.
        .arg(build_staticlib("thaw-quickjs"))
        .arg("-o")
        .arg(&exe_path)
        .status()
        .unwrap()
        .success());
    let output = Command::new(&exe_path).output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "direct nested\ntrue\n7\n2\nchecked nested\n3\n4\nnested check failed\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn ffi_call_supports_pointer_length_string_parameters_and_returns() {
    let source = r#"
        declare function native_slice(value: string): string;

        function main(): void {
            console.log(native_slice("hello"));
            const embedded: string = native_slice("a\u0000b");
            console.log(embedded.length, embedded.charCodeAt(1), embedded.charCodeAt(2));
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let mut program = thaw_hir::lower_module(&module).unwrap();
    thaw_hir::set_ffi_string_abi(
        &mut program,
        "native_slice",
        vec![thaw_hir::FfiStringAbi::PointerLength],
        thaw_hir::FfiStringAbi::PointerLength,
        thaw_hir::FfiCallingConvention::C,
        thaw_hir::FfiAggregateAbi::Internal,
    )
    .unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "ffi_string_slice");
    compiler.compile_program(&program).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "thaw-hir-codegen-test-ffi-string-slice-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let obj_path = dir.join("out.o");
    let exe_path = dir.join("out");
    let native_c_path = dir.join("native.c");
    let native_obj_path = dir.join("native.o");
    compiler.write_object_file(&obj_path).unwrap();
    std::fs::write(
        &native_c_path,
        "#include <stdint.h>\n\
         typedef struct { const char *data; int64_t len; } ThawStringSlice;\n\
         static const char result[] = {'O', 'K', '!'};\n\
         static const char embedded[] = {'A', 0, 'B'};\n\
         ThawStringSlice native_slice(const char *value, int64_t len) {\n\
           if (len == 3 && value[0] == 'a' && value[1] == 0 && value[2] == 'b') return (ThawStringSlice){embedded, 3};\n\
           return value[0] == 'h' && len == 5 ? (ThawStringSlice){result, 3} : (ThawStringSlice){result, 0};\n\
         }\n",
    )
    .unwrap();
    assert!(cc_command()
        .arg("-c")
        .arg(&native_c_path)
        .arg("-o")
        .arg(&native_obj_path)
        .status()
        .unwrap()
        .success());
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
        // A caught-error `console.log` reaches the JsValue console path, which lives in thaw-quickjs.
        .arg(build_staticlib("thaw-quickjs"))
        .arg("-o")
        .arg(&exe_path)
        .status()
        .unwrap()
        .success());
    let output = Command::new(&exe_path).output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "OK!\n3 0 66\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// docs/design/bridge.md section 5's marshal-adapter gap, closed:
/// unlike the plain-`number` case above, a real C function taking an
/// array almost never expects Thaw's own internal `[i64 len][f64
/// elements...]` buffer -- it expects the near-universal `(const
/// double*, int64_t len)` two-argument convention instead, exactly
/// what this real (not Thaw-authored) C function below declares. If
/// `ffi_param_types`/`compile_ffi_call` still passed Thaw's raw buffer
/// pointer as a single argument, this would either fail to link
/// (arity mismatch caught by `cc`) or silently misread memory as a
/// `(double*, int64_t)` pair -- it does neither: the sum comes back
/// correct.
#[test]
fn ffi_call_marshals_a_number_array_into_pointer_plus_length() {
    let source = r#"
        declare function native_sum(xs: number[]): number;

        function main(): void {
            const xs: number[] = [1, 2, 3, 4];
            console.log(native_sum(xs));
        }
    "#;

    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    assert_eq!(
        program.extern_functions.len(),
        1,
        "sanity: this is really an FFI call"
    );

    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "ffi_array_marshal");
    compiler.compile_program(&program).unwrap();

    let dir = std::env::temp_dir().join(format!(
        "thaw-hir-codegen-test-ffi_array_marshal-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let obj_path = dir.join("out.o");
    let exe_path = dir.join("out");
    let native_c_path = dir.join("native.c");
    let native_obj_path = dir.join("native.o");

    compiler.write_object_file(&obj_path).unwrap();

    // A real, independently-written C function -- not one shaped
    // around Thaw's own array layout. `len` is genuinely used (not
    // just accepted and ignored), so a wrong length would also
    // produce a wrong sum, not just "happen to work".
    std::fs::write(
        &native_c_path,
        "#include <stdint.h>\n\
         double native_sum(const double* xs, int64_t len) {\n\
         \x20\x20double total = 0;\n\
         \x20\x20for (int64_t i = 0; i < len; i++) { total += xs[i]; }\n\
         \x20\x20return total;\n\
         }\n",
    )
    .unwrap();
    let cc_status = cc_command()
        .arg("-c")
        .arg(&native_c_path)
        .arg("-o")
        .arg(&native_obj_path)
        .status()
        .expect("failed to invoke `cc` to build the native stand-in library");
    assert!(cc_status.success(), "compiling native.c failed");

    let arena_lib = build_staticlib("thaw-arena");
    let std_lib = build_staticlib("thaw-std");
    let runtime_lib = build_staticlib("thaw-runtime");
    let link_status = cc_command()
        .arg(&obj_path)
        .arg(&native_obj_path)
        .arg(&arena_lib)
        .arg(&std_lib)
        .arg(&runtime_lib)
        .arg(&std_lib)
        .arg(&runtime_lib)
        // A caught-error `console.log` reaches the JsValue console path, which lives in thaw-quickjs.
        .arg(build_staticlib("thaw-quickjs"))
        .arg("-o")
        .arg(&exe_path)
        .status()
        .expect("failed to invoke system `cc` linker");
    assert!(link_status.success(), "linking failed");

    let output = Command::new(&exe_path)
        .output()
        .expect("failed to execute compiled binary");
    assert!(output.status.success(), "binary exited non-zero");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "10\n");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ffi_call_marshals_string_arrays_in_both_directions() {
    let source = r#"
        declare function native_labels(values: string[]): string[];

        function main(): void {
            const result: string[] = native_labels(["first", "second"]);
            console.log(result[0], result[1], result.length);
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let mut program = thaw_hir::lower_module(&module).unwrap();
    thaw_hir::set_ffi_string_abi(
        &mut program,
        "native_labels",
        vec![thaw_hir::FfiStringAbi::NullTerminated],
        thaw_hir::FfiStringAbi::NullTerminated,
        thaw_hir::FfiCallingConvention::C,
        thaw_hir::FfiAggregateAbi::Portable,
    )
    .unwrap();

    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "ffi_string_arrays");
    compiler.compile_program(&program).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "thaw-hir-codegen-test-ffi-string-arrays-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let obj_path = dir.join("out.o");
    let exe_path = dir.join("out");
    let native_c_path = dir.join("native.c");
    let native_obj_path = dir.join("native.o");
    compiler.write_object_file(&obj_path).unwrap();
    std::fs::write(
        &native_c_path,
        "#include <stdint.h>\n\
         #include <string.h>\n\
         typedef struct { const char **data; int64_t len; } ThawStringArray;\n\
         static const char *result[] = {\"accepted\", \"strings\"};\n\
         ThawStringArray native_labels(const char **values, int64_t len) {\n\
           int valid = len == 2 && strcmp(values[0], \"first\") == 0 && strcmp(values[1], \"second\") == 0;\n\
           return valid ? (ThawStringArray){result, 2} : (ThawStringArray){result, 0};\n\
         }\n",
    )
    .unwrap();
    assert!(cc_command()
        .arg("-c")
        .arg(&native_c_path)
        .arg("-o")
        .arg(&native_obj_path)
        .status()
        .unwrap()
        .success());
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
        // A caught-error `console.log` reaches the JsValue console path, which lives in thaw-quickjs.
        .arg(build_staticlib("thaw-quickjs"))
        .arg("-o")
        .arg(&exe_path)
        .status()
        .unwrap()
        .success());
    let output = Command::new(&exe_path).output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "accepted strings 2\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn ffi_call_marshals_boolean_arrays_in_both_directions() {
    let source = r#"
        declare function native_flags(values: boolean[]): boolean[];

        function main(): void {
            const result: boolean[] = native_flags([true, false, true]);
            console.log(result[0], result[1], result[2], result.length);
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let mut program = thaw_hir::lower_module(&module).unwrap();
    thaw_hir::set_ffi_string_abi(
        &mut program,
        "native_flags",
        vec![thaw_hir::FfiStringAbi::NullTerminated],
        thaw_hir::FfiStringAbi::NullTerminated,
        thaw_hir::FfiCallingConvention::C,
        thaw_hir::FfiAggregateAbi::Portable,
    )
    .unwrap();

    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "ffi_boolean_arrays");
    compiler.compile_program(&program).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "thaw-hir-codegen-test-ffi-boolean-arrays-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let obj_path = dir.join("out.o");
    let exe_path = dir.join("out");
    let native_c_path = dir.join("native.c");
    let native_obj_path = dir.join("native.o");
    compiler.write_object_file(&obj_path).unwrap();
    std::fs::write(
        &native_c_path,
        "#include <stdint.h>\n\
         typedef struct { const uint8_t *data; int64_t len; } ThawBoolArray;\n\
         static const uint8_t result[] = {0, 1, 0};\n\
         ThawBoolArray native_flags(const uint8_t *values, int64_t len) {\n\
           int valid = len == 3 && values[0] == 1 && values[1] == 0 && values[2] == 1;\n\
           return valid ? (ThawBoolArray){result, 3} : (ThawBoolArray){result, 0};\n\
         }\n",
    )
    .unwrap();
    assert!(cc_command()
        .arg("-c")
        .arg(&native_c_path)
        .arg("-o")
        .arg(&native_obj_path)
        .status()
        .unwrap()
        .success());
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
        // A caught-error `console.log` reaches the JsValue console path, which lives in thaw-quickjs.
        .arg(build_staticlib("thaw-quickjs"))
        .arg("-o")
        .arg(&exe_path)
        .status()
        .unwrap()
        .success());
    let output = Command::new(&exe_path).output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "false true false 3\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn ffi_owned_boolean_array_return_is_copied_and_destroyed() {
    let source = r#"
        declare function native_owned_flags(): boolean[];
        declare function flag_destroy_count(): number;

        function main(): void {
            const flags: boolean[] = native_owned_flags();
            console.log(flags[0], flags[1], flags.length, flag_destroy_count());
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let mut program = thaw_hir::lower_module(&module).unwrap();
    thaw_hir::set_ffi_ownership(
        &mut program,
        "native_owned_flags",
        thaw_hir::FfiOwnership::Owned {
            destroy: "destroy_flags".into(),
        },
        thaw_hir::FfiOwnership::Borrowed,
    )
    .unwrap();
    thaw_hir::set_ffi_string_abi(
        &mut program,
        "native_owned_flags",
        vec![],
        thaw_hir::FfiStringAbi::NullTerminated,
        thaw_hir::FfiCallingConvention::C,
        thaw_hir::FfiAggregateAbi::Portable,
    )
    .unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "ffi_owned_boolean_array");
    compiler.compile_program(&program).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "thaw-hir-codegen-test-ffi-owned-bool-array-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let obj_path = dir.join("out.o");
    let exe_path = dir.join("out");
    let native_c_path = dir.join("native.c");
    let native_obj_path = dir.join("native.o");
    compiler.write_object_file(&obj_path).unwrap();
    std::fs::write(
        &native_c_path,
        r#"#include <stdint.h>
#include <stdlib.h>
typedef struct { uint8_t *data; int64_t len; } BoolArray;
static int destroyed = 0;
BoolArray native_owned_flags(void) {
  uint8_t *data = malloc(2);
  data[0] = 1;
  data[1] = 0;
  return (BoolArray){data, 2};
}
void destroy_flags(void *data) { destroyed += 1; free(data); }
double flag_destroy_count(void) { return destroyed; }
"#,
    )
    .unwrap();
    assert!(cc_command()
        .arg("-c")
        .arg(&native_c_path)
        .arg("-o")
        .arg(&native_obj_path)
        .status()
        .unwrap()
        .success());
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
        // A caught-error `console.log` reaches the JsValue console path, which lives in thaw-quickjs.
        .arg(build_staticlib("thaw-quickjs"))
        .arg("-o")
        .arg(&exe_path)
        .status()
        .unwrap()
        .success());
    let output = Command::new(&exe_path).output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "true false 2 1\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn ffi_call_marshals_js_value_arrays_in_both_directions() {
    let source = r#"
        declare function native_handle(value: number): JsValue;
        declare function native_handles(values: JsValue[]): JsValue[];

        function main(): void {
            const input: JsValue[] = [native_handle(11), native_handle(22)];
            console.log(native_handles(input).length);
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let mut program = thaw_hir::lower_module(&module).unwrap();
    thaw_hir::set_ffi_string_abi(
        &mut program,
        "native_handles",
        vec![thaw_hir::FfiStringAbi::NullTerminated],
        thaw_hir::FfiStringAbi::NullTerminated,
        thaw_hir::FfiCallingConvention::C,
        thaw_hir::FfiAggregateAbi::Portable,
    )
    .unwrap();

    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "ffi_js_value_arrays");
    compiler.compile_program(&program).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "thaw-hir-codegen-test-ffi-js-value-arrays-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let obj_path = dir.join("out.o");
    let exe_path = dir.join("out");
    let native_c_path = dir.join("native.c");
    let native_obj_path = dir.join("native.o");
    compiler.write_object_file(&obj_path).unwrap();
    std::fs::write(
        &native_c_path,
        "#include <stdint.h>\n\
         typedef struct { const uint64_t *data; int64_t len; } ThawHandleArray;\n\
         static const uint64_t result[] = {33, 44, 55};\n\
         uint64_t native_handle(double value) { return (uint64_t)value; }\n\
         ThawHandleArray native_handles(const uint64_t *values, int64_t len) {\n\
           int valid = len == 2 && values[0] == 11 && values[1] == 22;\n\
           return valid ? (ThawHandleArray){result, 3} : (ThawHandleArray){result, 0};\n\
         }\n",
    )
    .unwrap();
    assert!(cc_command()
        .arg("-c")
        .arg(&native_c_path)
        .arg("-o")
        .arg(&native_obj_path)
        .status()
        .unwrap()
        .success());
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
        // A caught-error `console.log` reaches the JsValue console path, which lives in thaw-quickjs.
        .arg(build_staticlib("thaw-quickjs"))
        .arg("-o")
        .arg(&exe_path)
        .status()
        .unwrap()
        .success());
    let output = Command::new(&exe_path).output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "3\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn ffi_call_recursively_marshals_array_fields_in_object_parameters() {
    let source = r#"
        declare function native_bundle(value: {
            label: string;
            numbers: number[];
            flags: boolean[];
            nested: { names: string[] };
        }): number;

        function main(): void {
            console.log(native_bundle({
                label: "bundle",
                numbers: [1, 2],
                flags: [true, false],
                nested: { names: ["left", "right"] }
            }));
        }
    "#;
        let module = thaw_parser::parse_typescript(source).unwrap();
        let program = thaw_hir::lower_module(&module).unwrap();
        let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "ffi_nested_object_arrays");
    compiler.compile_program(&program).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "thaw-hir-codegen-test-ffi-nested-object-arrays-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let obj_path = dir.join("out.o");
    let exe_path = dir.join("out");
    let native_c_path = dir.join("native.c");
    let native_obj_path = dir.join("native.o");
    compiler.write_object_file(&obj_path).unwrap();
    std::fs::write(
        &native_c_path,
        "#include <stdint.h>\n\
         #include <string.h>\n\
         double native_bundle(\n\
           const char *label,\n\
           const double *numbers, int64_t number_len,\n\
           const uint8_t *flags, int64_t flag_len,\n\
           const char **names, int64_t name_len\n\
         ) {\n\
           return strcmp(label, \"bundle\") == 0 &&\n\
             number_len == 2 && numbers[0] == 1 && numbers[1] == 2 &&\n\
             flag_len == 2 && flags[0] == 1 && flags[1] == 0 &&\n\
             name_len == 2 && strcmp(names[0], \"left\") == 0 && strcmp(names[1], \"right\") == 0\n\
             ? 42 : 0;\n\
         }\n",
    )
    .unwrap();
    assert!(cc_command()
        .arg("-c")
        .arg(&native_c_path)
        .arg("-o")
        .arg(&native_obj_path)
        .status()
        .unwrap()
        .success());
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
        // A caught-error `console.log` reaches the JsValue console path, which lives in thaw-quickjs.
        .arg(build_staticlib("thaw-quickjs"))
        .arg("-o")
        .arg(&exe_path)
        .status()
        .unwrap()
        .success());
    let output = Command::new(&exe_path).output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "42\n");
    let _ = std::fs::remove_dir_all(dir);
}

/// Same gap, for `Object` params: a real C function is far more
/// likely to be a flat multi-argument function than to agree with
/// Thaw's own arena struct layout, so an object-typed FFI parameter
/// is flattened into one scalar C argument per field, in declared
/// order. `x*10 + y` (rather than something field-order-symmetric
/// like `x + y`) specifically catches a field-order bug: if `x`/`y`
/// were swapped, `{x: 3, y: 4}` would produce `43` instead of `34`.
#[test]
fn ffi_call_recursively_marshals_tagged_fixed_parameters() {
    let source = r#"
        declare function native_tagged(
            optional: number | undefined,
            nullable: number | null,
            nullish: number | null | undefined,
            object: {
                optional: number | undefined;
                nullable: number | null;
                nullish: number | null | undefined;
            }
        ): number;

        function main(): void {
            console.log(native_tagged(
                undefined,
                null,
                3,
                { optional: 4, nullable: 5, nullish: undefined }
            ));
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "ffi_tagged_fixed_parameters");
    compiler.compile_program(&program).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "thaw-hir-codegen-test-ffi-tagged-fixed-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let obj_path = dir.join("out.o");
    let exe_path = dir.join("out");
    let native_c_path = dir.join("native.c");
    let native_obj_path = dir.join("native.o");
    compiler.write_object_file(&obj_path).unwrap();
    std::fs::write(
        &native_c_path,
        r#"#include <stdint.h>
double native_tagged(
  uint8_t optional_tag, double optional,
  uint8_t nullable_tag, double nullable,
  uint8_t nullish_tag, double nullish,
  uint8_t object_optional_tag, double object_optional,
  uint8_t object_nullable_tag, double object_nullable,
  uint8_t object_nullish_tag, double object_nullish
) {
  return optional_tag == 0 &&
    nullable_tag == 0 &&
    nullish_tag == 0 && nullish == 3 &&
    object_optional_tag == 1 && object_optional == 4 &&
    object_nullable_tag == 1 && object_nullable == 5 &&
    object_nullish_tag == 2
    ? 42 : 0;
}
"#,
    )
    .unwrap();
    assert!(cc_command()
        .arg("-c")
        .arg(&native_c_path)
        .arg("-o")
        .arg(&native_obj_path)
        .status()
        .unwrap()
        .success());
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
        // A caught-error `console.log` reaches the JsValue console path, which lives in thaw-quickjs.
        .arg(build_staticlib("thaw-quickjs"))
        .arg("-o")
        .arg(&exe_path)
        .status()
        .unwrap()
        .success());
    let output = Command::new(&exe_path).output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "42\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn ffi_call_restores_portable_tagged_returns() {
    let source = r#"
        declare function native_optional(present: boolean): number | undefined;
        declare function native_nullable(present: boolean): number | null;
        declare function native_nullish(state: number): number | null | undefined;

        function main(): void {
            console.log(native_optional(true), native_optional(false));
            console.log(native_nullable(true), native_nullable(false));
            console.log(native_nullish(0), native_nullish(1), native_nullish(2));
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let mut program = thaw_hir::lower_module(&module).unwrap();
    for symbol in ["native_optional", "native_nullable", "native_nullish"] {
        thaw_hir::set_ffi_string_abi(
            &mut program,
            symbol,
            vec![thaw_hir::FfiStringAbi::NullTerminated],
            thaw_hir::FfiStringAbi::NullTerminated,
            thaw_hir::FfiCallingConvention::C,
            thaw_hir::FfiAggregateAbi::Portable,
        )
        .unwrap();
    }
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "ffi_portable_tagged_returns");
    compiler.compile_program(&program).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "thaw-hir-codegen-test-ffi-tagged-returns-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let obj_path = dir.join("out.o");
    let exe_path = dir.join("out");
    let native_c_path = dir.join("native.c");
    let native_obj_path = dir.join("native.o");
    compiler.write_object_file(&obj_path).unwrap();
    std::fs::write(
        &native_c_path,
        r#"#include <stdint.h>
typedef struct { uint8_t tag; double value; } TaggedNumber;
TaggedNumber native_optional(uint8_t present) {
  return (TaggedNumber){present, 21};
}
TaggedNumber native_nullable(uint8_t present) {
  return (TaggedNumber){present, 22};
}
TaggedNumber native_nullish(double state) {
  return (TaggedNumber){(uint8_t)state, 23};
}
"#,
    )
    .unwrap();
    assert!(cc_command()
        .arg("-c")
        .arg(&native_c_path)
        .arg("-o")
        .arg(&native_obj_path)
        .status()
        .unwrap()
        .success());
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
        // A caught-error `console.log` reaches the JsValue console path, which lives in thaw-quickjs.
        .arg(build_staticlib("thaw-quickjs"))
        .arg("-o")
        .arg(&exe_path)
        .status()
        .unwrap()
        .success());
    let output = Command::new(&exe_path).output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "21 undefined\n22 null\n23 null undefined\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn ffi_call_marshals_portable_tuples_in_both_directions() {
    let source = r#"
        declare function native_tuple(value: [number, string, boolean]): [string, number, boolean];

        function main(): void {
            const result: [string, number, boolean] = native_tuple([1, "two", true]);
            console.log(result[0], result[1], result[2]);
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let mut program = thaw_hir::lower_module(&module).unwrap();
    thaw_hir::set_ffi_string_abi(
        &mut program,
        "native_tuple",
        vec![thaw_hir::FfiStringAbi::NullTerminated],
        thaw_hir::FfiStringAbi::NullTerminated,
        thaw_hir::FfiCallingConvention::C,
        thaw_hir::FfiAggregateAbi::Portable,
    )
    .unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "ffi_portable_tuples");
    compiler.compile_program(&program).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "thaw-hir-codegen-test-ffi-tuples-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let obj_path = dir.join("out.o");
    let exe_path = dir.join("out");
    let native_c_path = dir.join("native.c");
    let native_obj_path = dir.join("native.o");
    compiler.write_object_file(&obj_path).unwrap();
    std::fs::write(
        &native_c_path,
        r#"#include <stdbool.h>
#include <string.h>
typedef struct { const char *text; double number; bool flag; } NativeTuple;
NativeTuple native_tuple(double number, const char *text, bool flag) {
  static const char result[] = "tuple";
  return number == 1 && strcmp(text, "two") == 0 && flag
    ? (NativeTuple){result, 24, false}
    : (NativeTuple){result, 0, true};
}
"#,
    )
    .unwrap();
    assert!(cc_command()
        .arg("-c")
        .arg(&native_c_path)
        .arg("-o")
        .arg(&native_obj_path)
        .status()
        .unwrap()
        .success());
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
        // A caught-error `console.log` reaches the JsValue console path, which lives in thaw-quickjs.
        .arg(build_staticlib("thaw-quickjs"))
        .arg("-o")
        .arg(&exe_path)
        .status()
        .unwrap()
        .success());
    let output = Command::new(&exe_path).output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "tuple 24 false\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn ffi_owned_tuple_return_releases_each_owned_leaf() {
    let source = r#"
        declare function native_owned_tuple(): [string, boolean[]];
        declare function tuple_destroy_count(): number;

        function main(): void {
            const value: [string, boolean[]] = native_owned_tuple();
            console.log(value[0], value[1][0], value[1][1], tuple_destroy_count());
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let mut program = thaw_hir::lower_module(&module).unwrap();
    thaw_hir::set_ffi_ownership(
        &mut program,
        "native_owned_tuple",
        thaw_hir::FfiOwnership::Owned {
            destroy: "destroy_tuple_leaf".into(),
        },
        thaw_hir::FfiOwnership::Borrowed,
    )
    .unwrap();
    thaw_hir::set_ffi_string_abi(
        &mut program,
        "native_owned_tuple",
        vec![],
        thaw_hir::FfiStringAbi::NullTerminated,
        thaw_hir::FfiCallingConvention::C,
        thaw_hir::FfiAggregateAbi::Portable,
    )
    .unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "ffi_owned_tuple");
    compiler.compile_program(&program).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "thaw-hir-codegen-test-ffi-owned-tuple-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let obj_path = dir.join("out.o");
    let exe_path = dir.join("out");
    let native_c_path = dir.join("native.c");
    let native_obj_path = dir.join("native.o");
    compiler.write_object_file(&obj_path).unwrap();
    std::fs::write(
        &native_c_path,
        r#"#include <stdint.h>
#include <stdlib.h>
#include <string.h>
typedef struct { uint8_t *data; int64_t len; } BoolArray;
typedef struct { char *text; BoolArray flags; } OwnedTuple;
static int destroyed = 0;
OwnedTuple native_owned_tuple(void) {
  char *text = malloc(6);
  memcpy(text, "owned", 6);
  uint8_t *flags = malloc(2);
  flags[0] = 1;
  flags[1] = 0;
  return (OwnedTuple){text, {flags, 2}};
}
void destroy_tuple_leaf(void *value) { destroyed += 1; free(value); }
double tuple_destroy_count(void) { return destroyed; }
"#,
    )
    .unwrap();
    assert!(cc_command()
        .arg("-c")
        .arg(&native_c_path)
        .arg("-o")
        .arg(&native_obj_path)
        .status()
        .unwrap()
        .success());
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
        // A caught-error `console.log` reaches the JsValue console path, which lives in thaw-quickjs.
        .arg(build_staticlib("thaw-quickjs"))
        .arg("-o")
        .arg(&exe_path)
        .status()
        .unwrap()
        .success());
    let output = Command::new(&exe_path).output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "owned true false 2\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn ffi_call_marshals_an_object_into_one_scalar_argument_per_field() {
    let source = r#"
        declare function native_combine(p: { x: number; y: number }): number;

        function main(): void {
            console.log(native_combine({ x: 3, y: 4 }));
        }
    "#;

    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    assert_eq!(
        program.extern_functions.len(),
        1,
        "sanity: this is really an FFI call"
    );

    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "ffi_object_marshal");
    compiler.compile_program(&program).unwrap();

    let dir = std::env::temp_dir().join(format!(
        "thaw-hir-codegen-test-ffi_object_marshal-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let obj_path = dir.join("out.o");
    let exe_path = dir.join("out");
    let native_c_path = dir.join("native.c");
    let native_obj_path = dir.join("native.o");

    compiler.write_object_file(&obj_path).unwrap();

    std::fs::write(
        &native_c_path,
        "double native_combine(double x, double y) { return x * 10 + y; }\n",
    )
    .unwrap();
    let cc_status = cc_command()
        .arg("-c")
        .arg(&native_c_path)
        .arg("-o")
        .arg(&native_obj_path)
        .status()
        .expect("failed to invoke `cc` to build the native stand-in library");
    assert!(cc_status.success(), "compiling native.c failed");

    let arena_lib = build_staticlib("thaw-arena");
    let std_lib = build_staticlib("thaw-std");
    let runtime_lib = build_staticlib("thaw-runtime");
    let link_status = cc_command()
        .arg(&obj_path)
        .arg(&native_obj_path)
        .arg(&arena_lib)
        .arg(&std_lib)
        .arg(&runtime_lib)
        .arg(&std_lib)
        .arg(&runtime_lib)
        // A caught-error `console.log` reaches the JsValue console path, which lives in thaw-quickjs.
        .arg(build_staticlib("thaw-quickjs"))
        .arg("-o")
        .arg(&exe_path)
        .status()
        .expect("failed to invoke system `cc` linker");
    assert!(link_status.success(), "linking failed");

    let output = Command::new(&exe_path)
        .output()
        .expect("failed to execute compiled binary");
    assert!(output.status.success(), "binary exited non-zero");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "34\n");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn nested_ffi_tuples_use_their_full_storage_layout() {
    let pair = HirType::Tuple(vec![HirType::F64, HirType::F64]);
    let object = HirType::Object(vec![("pair".into(), pair.clone()), ("z".into(), HirType::F64)]);
    let tuple = HirType::Tuple(vec![pair, HirType::F64]);
    for value in [&object, &tuple] {
        assert_eq!(HirCompiler::ffi_aggregate_storage_layout(value, FfiAggregateAbi::Portable), (24, 8));
        assert_eq!(HirCompiler::ffi_aggregate_storage_layout(value, FfiAggregateAbi::Packed), (24, 1));
    }
    assert_eq!(HirCompiler::ffi_aggregate_storage_layout(&object, FfiAggregateAbi::Internal), (8, 8));
}

#[test]
fn ffi_dynamic_storage_validates_before_invoking_native() {
    let source = r#"
        class Native { value: number = 1; }
        declare function native_storage_sum(values: number[]): number;
        declare function native_storage_class(value: Native): number;
        declare function native_storage_record(value: {value: number}): number;
        declare function native_storage_bigint(value: bigint): number;
        declare function native_storage_collection(value: Map<string, number>): number;
        declare function native_storage_make(): Native;
        declare function native_storage_values(): number[];
        declare function native_storage_source_value(): number;
        declare function native_storage_source_element(): number;
        declare function native_storage_calls(): number;
        function main(): void {
            const values: number[] = [1, 2]; const exposed: any = values;
            console.log(native_storage_sum(values), native_storage_calls());
            const original: any = new Function("return {x: 7}")(); exposed[0] = original;
            try { native_storage_sum(values); console.log("accepted array"); }
            catch (error) { console.log(error instanceof TypeError, native_storage_calls()); }
            const instance = new Native(); const view: any = instance; view.value = 7;
            console.log(native_storage_class(instance), native_storage_calls());
            view.value = original;
            try { native_storage_class(instance); console.log("accepted field"); }
            catch (error) { console.log(error instanceof TypeError, native_storage_calls()); }
            const impostor: any = new Function("return {value: 7}")();
            try { native_storage_class(impostor); console.log("accepted brand"); }
            catch (error) { console.log(error instanceof TypeError, native_storage_calls()); }
            const getter: any = new Function("return {reads: 0, get value() { this.reads++; return this.reads === 1 ? 8 : {}; }}")();
            console.log(native_storage_record(getter), getter.reads, native_storage_calls());
            const bigint: any = 9n;
            console.log(native_storage_bigint(bigint), native_storage_calls());
            try { native_storage_bigint(original); console.log("accepted bigint"); }
            catch (error) { console.log(error instanceof TypeError, native_storage_calls()); }
            const collection = new Map<string, number>(); collection.set("a", 1);
            console.log(native_storage_collection(collection), native_storage_calls());
            const validForeign: any = new Function("return new Map([['a', 1]])")();
            console.log(native_storage_collection(validForeign), native_storage_calls());
            const foreign: any = new Function("return new Map([[1, 'bad']])")();
            try { native_storage_collection(foreign); console.log("accepted collection"); }
            catch (error) { console.log(error instanceof TypeError, native_storage_calls()); }
            console.log(exposed[0] === original, view.value === original, foreign.get(1));
            const returned: Native = native_storage_make(); const returnedView: any = returned;
            console.log(returned.value, returned instanceof Native, native_storage_calls());
            returnedView.value = original;
            console.log(returned.value === original, native_storage_source_value(), native_storage_calls());
            const returnedValues: number[] = native_storage_values(); const valuesView: any = returnedValues;
            console.log(returnedValues[0], native_storage_calls()); valuesView[0] = original;
            console.log(returnedValues[0] === original, typeof returnedValues[0], native_storage_source_element());
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let mut program = thaw_hir::lower_module(&module).unwrap();
    thaw_hir::set_ffi_string_abi(&mut program, "native_storage_values", vec![],
        thaw_hir::FfiStringAbi::NullTerminated, thaw_hir::FfiCallingConvention::C,
        thaw_hir::FfiAggregateAbi::Portable).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "ffi_dynamic_storage");
    compiler.compile_program(&program).unwrap();
    let dir = std::env::temp_dir().join(format!("thaw-ffi-dynamic-storage-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let object = dir.join("program.o");
    let native = dir.join("native.c");
    let native_object = dir.join("native.o");
    let executable = dir.join("program");
    compiler.write_object_file(&object).unwrap();
    std::fs::write(&native, r#"
        #include <stdint.h>
        static int calls;
        double native_storage_sum(const double *values, int64_t length) {
            calls++; double total = 0; for (int64_t i = 0; i < length; i++) total += values[i]; return total;
        }
        double native_storage_class(uint8_t identity, double value) { (void)identity; calls++; return value; }
        double native_storage_record(double value) { calls++; return value; }
        double native_storage_bigint(int64_t value) { calls++; return (double)value; }
        double native_storage_collection(void *value) { (void)value; calls++; return 1; }
        typedef struct { uint8_t identity; double value; } NativeStorageObject;
        static NativeStorageObject returned_object = {1, 11};
        static double returned_values[] = {1, 2};
        typedef struct { double *data; int64_t length; } NativeStorageValues;
        void *native_storage_make(void) { calls++; return &returned_object; }
        NativeStorageValues native_storage_values(void) { calls++; return (NativeStorageValues){returned_values, 2}; }
        double native_storage_source_value(void) { return returned_object.value; }
        double native_storage_source_element(void) { return returned_values[0]; }
        double native_storage_calls(void) { return calls; }
    "#).unwrap();
    assert!(cc_command().arg("-c").arg(&native).arg("-o").arg(&native_object).status().unwrap().success());
    let arena = build_staticlib("thaw-arena");
    let std_lib = build_staticlib("thaw-std");
    let runtime = build_staticlib("thaw-runtime");
    let quickjs = build_staticlib("thaw-quickjs");
    assert!(cc_command().arg(&object).arg(&native_object).arg(&arena).arg(&std_lib).arg(&runtime)
        .arg(&std_lib).arg(&runtime).arg(&quickjs).arg("-lm").arg("-Wl,--export-dynamic")
        .arg("-o").arg(&executable).status().unwrap().success());
    let output = Command::new(&executable).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "3 1\ntrue 1\n7 2\ntrue 2\ntrue 2\n8 1 3\n9 4\ntrue 4\n1 5\n1 6\ntrue 6\ntrue true bad\n11 false 7\ntrue 11 7\n1 8\ntrue object 1\n");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn ffi_js_owned_maps_and_sets_copy_in_before_native_invocation() {
    let source = r#"
        declare function native_map_sum(value: Map<string, number>): number;
        declare function native_map_order(value: Map<string, number>): number;
        declare function native_num_map_sum(value: Map<number, number>): number;
        declare function native_nested_map_sum(value: Map<string, Map<string, number>>): number;
        declare function native_any_map_lookup(value: Map<any, number>, key: any): number;
        declare function native_object_map_sum(value: Map<{value: number}, number>): number;
        declare function native_optional_json_map_sum(value: Map<string, any | undefined>): number;
        declare function native_optional_json_map_empty(): Map<string, any | undefined>;
        declare function native_string_map_empty(): Map<string, number>;
        declare function native_set_sum(value: Set<number>): number;
        declare function native_set_order(value: Set<number>): number;
        declare function native_set_size(value: Set<number>): number;
        declare function native_string_set_size(value: Set<string>): number;
        declare function native_any_set_size(value: Set<any>): number;
        declare function native_string_set_empty(): Set<string>;
        declare function native_nested_set_sum(value: Map<string, Set<number>>): number;
        declare function native_map_mutate(value: Map<string, number>): number;
        declare function native_set_mutate(value: Set<number>): number;
        declare function native_collection_calls(): number;
        function main(): void {
            const jsMap: any = new Function("return new Map([['a', 1], ['b', 2]])")();
            const jsSet: any = new Function("return new Set([1, 2, 3])")();
            try { console.log(native_map_sum(jsMap), native_collection_calls()); }
            catch (error) { console.log("map rejected", error instanceof TypeError, native_collection_calls()); }
            try { console.log(native_set_sum(jsSet), native_collection_calls()); }
            catch (error) { console.log("set rejected", error instanceof TypeError, native_collection_calls()); }
            try { native_map_mutate(jsMap); console.log(jsMap.get("changed"), native_collection_calls()); }
            catch (error) { console.log("map mutate rejected", error instanceof TypeError, native_collection_calls()); }
            try { native_set_mutate(jsSet); console.log(jsSet.has(9), native_collection_calls()); }
            catch (error) { console.log("set mutate rejected", error instanceof TypeError, native_collection_calls()); }
            const emptyMap: any = new Function("return new Map()")();
            const emptySet: any = new Function("return new Set()")();
            try { console.log(native_map_sum(emptyMap), native_collection_calls()); }
            catch (error) { console.log("empty map rejected", error instanceof TypeError, native_collection_calls()); }
            try { console.log(native_set_sum(emptySet), native_collection_calls()); }
            catch (error) { console.log("empty set rejected", error instanceof TypeError, native_collection_calls()); }
            const ordered: any = new Function("const m = new Map([['first', 11], ['last', 22]]); m.entries = () => { throw new Error('overridden entries'); }; m[Symbol.iterator] = () => { throw new Error('overridden iterator'); }; return m")();
            console.log(native_map_order(ordered), native_collection_calls());
            const numeric: any = new Function("return new Map([[NaN, 1], [NaN, 2], [-0, 3], [+0, 4]])")();
            console.log(native_num_map_sum(numeric), native_collection_calls());
            const nested: any = new Function("return new Map([['inner', new Map([['a', 5]])]])")();
            console.log(native_nested_map_sum(nested), native_collection_calls());
            const nestedBad: any = new Function("return new Map([['inner', new Map([['a', 5], ['b', 'bad']])]])")();
            try { native_nested_map_sum(nestedBad); console.log("accepted nested map"); }
            catch (error) { console.log(error instanceof TypeError, native_collection_calls()); }
            const objectKey: any = new Function("return {}")();
            const identityMap: any = new Function("key", "return new Map([[key, 17]])")(objectKey);
            console.log(native_any_map_lookup(identityMap, objectKey), native_collection_calls());
            const wrongNative: any = native_string_map_empty();
            try { native_num_map_sum(wrongNative); console.log("accepted wrong native map"); }
            catch (error) { console.log(error instanceof TypeError, native_collection_calls()); }
            const objectKeyMap: Map<{value: number}, number> = new Map<{value: number}, number>();
            objectKeyMap.set({value: 1}, 23);
            console.log(native_object_map_sum(objectKeyMap), native_collection_calls());
            const foreignObjectKeyMap: any = new Function("return new Map([[{value: 1}, 23]])")();
            try { native_object_map_sum(foreignObjectKeyMap); console.log("accepted object key"); }
            catch (error) { console.log(error instanceof TypeError, native_collection_calls()); }
            try { native_optional_json_map_sum(new Function("key", "return new Map([['held', key]])")(objectKey)); console.log("accepted wrapped value"); }
            catch (error) { console.log(error instanceof TypeError, native_collection_calls()); }
            console.log(native_optional_json_map_sum(native_optional_json_map_empty()), native_collection_calls());
            const nativeMap = new Map<string, number>(); nativeMap.set("a", 4);
            const nativeSet = new Set<number>(); nativeSet.add(5);
            console.log(native_map_sum(nativeMap), native_set_sum(nativeSet), native_collection_calls());
            native_map_mutate(nativeMap); console.log(nativeMap.get("changed"), native_collection_calls());
            const badKey: any = new Function("return new Map([[1, 2]])")();
            const badLateKey: any = new Function("return new Map([['a', 1], [1, 2]])")();
            const badValue: any = new Function("return new Map([['a', 1], ['b', 'bad']])")();
            const plain: any = new Function("return {entries: []}")();
            const forgedMap: any = new Function("return {__thaw_map_entries__: [['a', 1]]}")();
            const forgedSet: any = new Function("return {__thaw_set_values__: [1]}")();
            const wrongKind: any = new Function("return new Set([1])")();
            try { native_map_sum(badKey); console.log("accepted invalid map"); }
            catch (error) { console.log(error instanceof TypeError, native_collection_calls()); }
            try { native_map_sum(badLateKey); console.log("accepted invalid map"); }
            catch (error) { console.log(error instanceof TypeError, native_collection_calls()); }
            try { native_map_sum(badValue); console.log("accepted invalid map"); }
            catch (error) { console.log(error instanceof TypeError, native_collection_calls()); }
            try { native_map_sum(plain); console.log("accepted invalid map"); }
            catch (error) { console.log(error instanceof TypeError, native_collection_calls()); }
            try { native_map_sum(forgedMap); console.log("accepted invalid map"); }
            catch (error) { console.log(error instanceof TypeError, native_collection_calls()); }
            try { native_map_sum(wrongKind); console.log("accepted invalid map"); }
            catch (error) { console.log(error instanceof TypeError, native_collection_calls()); }
            try { native_set_sum(forgedSet); console.log("accepted invalid set"); }
            catch (error) { console.log(error instanceof TypeError, native_collection_calls()); }
            const duplicateSet: any = new Function("return new Set([1, 2, 2, 3])")();
            console.log(native_set_sum(duplicateSet), native_collection_calls());
            const numericSet: any = new Function("return new Set([NaN, NaN, -0, +0, 3])")();
            console.log(native_set_size(numericSet), native_collection_calls());
            const orderedSet: any = new Function("return new Set([11, 22])")();
            console.log(native_set_order(orderedSet), native_collection_calls());
            const nestedSet: any = new Function("return new Map([['inner', new Set([5, 7])]])")();
            console.log(native_nested_set_sum(nestedSet), native_collection_calls());
            const nestedBadSet: any = new Function("return new Map([['inner', new Set([5, 'bad'])]])")();
            try { native_nested_set_sum(nestedBadSet); console.log("accepted nested set"); }
            catch (error) { console.log(error instanceof TypeError, native_collection_calls()); }
            const badSetElement: any = new Function("return new Set([1, 'bad'])")();
            try { native_set_sum(badSetElement); console.log("accepted invalid set"); }
            catch (error) { console.log(error instanceof TypeError, native_collection_calls()); }
            const wrongSetKind: any = new Function("return new Map()")();
            try { native_set_sum(wrongSetKind); console.log("accepted invalid set"); }
            catch (error) { console.log(error instanceof TypeError, native_collection_calls()); }
            const wrongNativeSet: any = native_string_set_empty();
            try { native_set_sum(wrongNativeSet); console.log("accepted wrong native set"); }
            catch (error) { console.log(error instanceof TypeError, native_collection_calls()); }
            const stringSet: any = new Function("return new Set(['a', 'b', 'a'])")();
            console.log(native_string_set_size(stringSet), native_collection_calls());
            const anySet: any = new Function("return new Set([1, 1, 2])")();
            console.log(native_any_set_size(anySet), native_collection_calls());
            native_set_mutate(nativeSet); console.log(nativeSet.has(9), native_collection_calls());
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "ffi_js_collection_copyin");
    compiler.compile_program(&program).unwrap();
    let dir = std::env::temp_dir().join(format!("thaw-ffi-js-collection-copyin-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let object = dir.join("program.o");
    let native = dir.join("native.c");
    let native_object = dir.join("native.o");
    let executable = dir.join("program");
    compiler.write_object_file(&object).unwrap();
    std::fs::write(&native, r#"
        #include <stdint.h>
        #include <string.h>
        static int calls;
        static double sum_words(void *array) {
            if (!array) return 0;
            int64_t length; memcpy(&length, array, sizeof(length));
            double total = 0;
            for (int64_t i = 0; i < length; i++) {
                uint64_t word; double value;
                memcpy(&word, (uint8_t *)array + 8 + i * 8, sizeof(word));
                memcpy(&value, &word, sizeof(value)); total += value;
            }
            return total;
        }
        extern void *thaw_map_snapshot_values(const void *map);
        extern void *thaw_map_snapshot_keys(const void *map);
        extern void *thaw_map_new(void);
        extern uint64_t thaw_map_str_set(const void *map, const char *key, uint64_t value);
        extern uint64_t thaw_map_num_set(const void *map, double key, uint64_t value);
        extern void thaw_map_clear(const void *map);
        double native_map_sum(void *map) { calls++; return sum_words(thaw_map_snapshot_values(map)); }
        static double first_last(void *array) {
            int64_t length; memcpy(&length, array, sizeof(length));
            if (length < 2) return 0;
            uint64_t first_bits, last_bits; double first, last;
            memcpy(&first_bits, (uint8_t *)array + 8, sizeof(first_bits));
            memcpy(&last_bits, (uint8_t *)array + 8 + (length - 1) * 8, sizeof(last_bits));
            memcpy(&first, &first_bits, sizeof(first)); memcpy(&last, &last_bits, sizeof(last));
            return first * 100 + last;
        }
        double native_map_order(void *map) { calls++; return first_last(thaw_map_snapshot_values(map)); }
        void *native_string_map_empty(void) {
            double one = 1; uint64_t bits; memcpy(&bits, &one, sizeof(bits));
            void *map = thaw_map_new(); thaw_map_str_set(map, "layout", bits); thaw_map_clear(map); return map;
        }
        double native_num_map_sum(void *map) { calls++; return sum_words(thaw_map_snapshot_values(map)); }
        extern void *thaw_map_str_get_ptr(const void *map, const char *key);
        extern double thaw_map_any_get_f64(const void *map, const void *key);
        double native_nested_map_sum(void *map) { calls++; return sum_words(thaw_map_snapshot_values(thaw_map_str_get_ptr(map, "inner"))); }
        double native_any_map_lookup(void *map, void *key) { calls++; return thaw_map_any_get_f64(map, key); }
        double native_object_map_sum(void *map) { calls++; return sum_words(thaw_map_snapshot_values(map)); }
        double native_optional_json_map_sum(void *map) { calls++; return sum_words(thaw_map_snapshot_values(map)); }
        void *native_optional_json_map_empty(void) { return thaw_map_new(); }
        double native_set_sum(void *set) { calls++; return sum_words(thaw_map_snapshot_keys(set)); }
        double native_set_order(void *set) { calls++; return first_last(thaw_map_snapshot_keys(set)); }
        static double set_size(void *set) {
            calls++; void *keys = thaw_map_snapshot_keys(set); int64_t length;
            memcpy(&length, keys, sizeof(length)); return length;
        }
        double native_set_size(void *set) { return set_size(set); }
        double native_string_set_size(void *set) { return set_size(set); }
        double native_any_set_size(void *set) { return set_size(set); }
        void *native_string_set_empty(void) {
            void *set = thaw_map_new(); thaw_map_str_set(set, "layout", 0); thaw_map_clear(set); return set;
        }
        double native_nested_set_sum(void *map) {
            calls++; return sum_words(thaw_map_snapshot_keys(thaw_map_str_get_ptr(map, "inner")));
        }
        double native_map_mutate(void *map) {
            uint64_t bits; double value = 7; memcpy(&bits, &value, sizeof(bits));
            calls++; thaw_map_str_set(map, "changed", bits); return calls;
        }
        double native_set_mutate(void *set) {
            calls++; thaw_map_num_set(set, 9, 0); return calls;
        }
        double native_collection_calls(void) { return calls; }
    "#).unwrap();
    assert!(cc_command().arg("-c").arg(&native).arg("-o").arg(&native_object).status().unwrap().success());
    let arena = build_staticlib("thaw-arena");
    let std_lib = build_staticlib("thaw-std");
    let runtime = build_staticlib("thaw-runtime");
    let quickjs = build_staticlib("thaw-quickjs");
    assert!(cc_command().arg(&object).arg(&native_object).arg(&arena).arg(&std_lib).arg(&runtime)
        .arg(&std_lib).arg(&runtime).arg(&quickjs).arg("-lm").arg("-Wl,--export-dynamic")
        .arg("-o").arg(&executable).status().unwrap().success());
    let output = Command::new(&executable).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "3 1\n6 2\nundefined 3\nfalse 4\n0 5\n0 6\n1122 7\n6 8\n5 9\ntrue 9\n17 10\ntrue 10\n23 11\ntrue 11\ntrue 11\n0 12\n4 5 14\n7 15\ntrue 15\ntrue 15\ntrue 15\ntrue 15\ntrue 15\ntrue 15\ntrue 15\n6 16\n3 17\n1122 18\n12 19\ntrue 19\ntrue 19\ntrue 19\ntrue 19\n2 20\n2 21\ntrue 22\n");
    std::fs::remove_dir_all(dir).unwrap();
}
