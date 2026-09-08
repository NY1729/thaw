
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
    let runtime_lib = build_staticlib("thaw-runtime");
    let link_status = cc_command()
        .arg(&obj_path)
        .arg(&native_obj_path)
        .arg(&arena_lib)
        .arg(&runtime_lib)
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

        function main(): void {
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
           int count = (int)raw_count; int sum = 0; va_list args;\n\
           va_start(args, raw_count);\n\
           for (int i = 0; i < count; ++i) sum += va_arg(args, int);\n\
           va_end(args); return (double)sum;\n\
         }\n\
         double native_sum_i64(double raw_count, ...) {\n\
           int count = (int)raw_count; long long sum = 0; va_list args;\n\
           va_start(args, raw_count);\n\
           for (int i = 0; i < count; ++i) sum += va_arg(args, long long);\n\
           va_end(args); return (double)sum;\n\
         }\n\
         double native_sum_u32(double raw_count, ...) {\n\
           int count = (int)raw_count; unsigned int sum = 0; va_list args;\n\
           va_start(args, raw_count);\n\
           for (int i = 0; i < count; ++i) sum += va_arg(args, unsigned int);\n\
           va_end(args); return (double)sum;\n\
         }\n\
         double native_sum_u64(double raw_count, ...) {\n\
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
    let runtime_lib = build_staticlib("thaw-runtime");
    assert!(cc_command()
        .arg(&obj_path)
        .arg(&native_obj_path)
        .arg(&arena_lib)
        .arg(&runtime_lib)
        .arg("-o")
        .arg(&exe_path)
        .status()
        .unwrap()
        .success());
    let output = Command::new(&exe_path).output().unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "0\n10\n3\n9\n40\n15\n6\n3\n60\n26\n3\n4\n7\n6\n3\n6\n42\n42\n"
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
    let runtime_lib = build_staticlib("thaw-runtime");
    assert!(cc_command()
        .arg(&obj_path)
        .arg(&native_obj_path)
        .arg(&arena_lib)
        .arg(&runtime_lib)
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
    let runtime_lib = build_staticlib("thaw-runtime");
    assert!(cc_command()
        .arg(&obj_path)
        .arg(&native_obj_path)
        .arg(&arena_lib)
        .arg(&runtime_lib)
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
    let runtime_lib = build_staticlib("thaw-runtime");
    assert!(cc_command()
        .arg(&obj_path)
        .arg(&native_obj_path)
        .arg(&arena_lib)
        .arg(&runtime_lib)
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
    let runtime_lib = build_staticlib("thaw-runtime");
    assert!(cc_command()
        .arg(&obj_path)
        .arg(&native_obj_path)
        .arg(&arena_lib)
        .arg(&runtime_lib)
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
    let runtime_lib = build_staticlib("thaw-runtime");
    assert!(cc_command()
        .arg(&obj_path)
        .arg(&native_obj_path)
        .arg(&arena_lib)
        .arg(&runtime_lib)
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
    let runtime_lib = build_staticlib("thaw-runtime");
    assert!(cc_command()
        .arg(&obj_path)
        .arg(&native_obj_path)
        .arg(&arena_lib)
        .arg(&runtime_lib)
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
    let runtime_lib = build_staticlib("thaw-runtime");
    assert!(cc_command()
        .arg(&obj_path)
        .arg(&native_obj_path)
        .arg(&arena_lib)
        .arg(&runtime_lib)
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
    let runtime_lib = build_staticlib("thaw-runtime");
    assert!(cc_command()
        .arg(&obj_path)
        .arg(&native_obj_path)
        .arg(&arena_lib)
        .arg(&runtime_lib)
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
         ThawStringSlice native_slice(const char *value, int64_t len) {\n\
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
    let runtime_lib = build_staticlib("thaw-runtime");
    assert!(cc_command()
        .arg(&obj_path)
        .arg(&native_obj_path)
        .arg(&arena_lib)
        .arg(&runtime_lib)
        .arg("-o")
        .arg(&exe_path)
        .status()
        .unwrap()
        .success());
    let output = Command::new(&exe_path).output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "OK!\n");
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
    let runtime_lib = build_staticlib("thaw-runtime");
    let link_status = cc_command()
        .arg(&obj_path)
        .arg(&native_obj_path)
        .arg(&arena_lib)
        .arg(&runtime_lib)
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
    let runtime_lib = build_staticlib("thaw-runtime");
    assert!(cc_command()
        .arg(&obj_path)
        .arg(&native_obj_path)
        .arg(&arena_lib)
        .arg(&runtime_lib)
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
    let runtime_lib = build_staticlib("thaw-runtime");
    assert!(cc_command()
        .arg(&obj_path)
        .arg(&native_obj_path)
        .arg(&arena_lib)
        .arg(&runtime_lib)
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
    let runtime_lib = build_staticlib("thaw-runtime");
    assert!(cc_command()
        .arg(&obj_path)
        .arg(&native_obj_path)
        .arg(&arena_lib)
        .arg(&runtime_lib)
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
    let runtime_lib = build_staticlib("thaw-runtime");
    assert!(cc_command()
        .arg(&obj_path)
        .arg(&native_obj_path)
        .arg(&arena_lib)
        .arg(&runtime_lib)
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
    let runtime_lib = build_staticlib("thaw-runtime");
    assert!(cc_command()
        .arg(&obj_path)
        .arg(&native_obj_path)
        .arg(&arena_lib)
        .arg(&runtime_lib)
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
    let runtime_lib = build_staticlib("thaw-runtime");
    assert!(cc_command()
        .arg(&obj_path)
        .arg(&native_obj_path)
        .arg(&arena_lib)
        .arg(&runtime_lib)
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
    let runtime_lib = build_staticlib("thaw-runtime");
    assert!(cc_command()
        .arg(&obj_path)
        .arg(&native_obj_path)
        .arg(&arena_lib)
        .arg(&runtime_lib)
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
    let runtime_lib = build_staticlib("thaw-runtime");
    assert!(cc_command()
        .arg(&obj_path)
        .arg(&native_obj_path)
        .arg(&arena_lib)
        .arg(&runtime_lib)
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
    let runtime_lib = build_staticlib("thaw-runtime");
    assert!(cc_command()
        .arg(&obj_path)
        .arg(&native_obj_path)
        .arg(&arena_lib)
        .arg(&runtime_lib)
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
    let runtime_lib = build_staticlib("thaw-runtime");
    let link_status = cc_command()
        .arg(&obj_path)
        .arg(&native_obj_path)
        .arg(&arena_lib)
        .arg(&runtime_lib)
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
