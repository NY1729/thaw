#[test]
fn compiles_function_arrow_and_promise_callback_parameter_destructuring() {
    let source = r#"
        function describe(
            { x, nested: { flag }, ...rest }:
                { x: number; nested: { flag: boolean }; label: string; extra: number },
            [first, ...tail]: [number, number, number]
        ): number {
            console.log(x); console.log(flag);
            console.log(rest.label); console.log(rest.extra);
            console.log(first); console.log(tail[0]); console.log(tail[1]);
            return x + first;
        }
        async function objectValue(): Promise<{ value: number; label: string }> {
            await sleep(1); return { value: 8, label: "eight" };
        }
        function arrowValue(): number {
            const pick = ({ value }: { value: number }): number => value + 1;
            return pick({ value: 6 });
        }
        async function main(): Promise<void> {
            console.log(describe(
                { x: 1, nested: { flag: true }, label: "ok", extra: 4 },
                [2, 3, 5]
            ));
            console.log(arrowValue());
            const chained: number = await objectValue().then(({ value }) => value + 2);
            console.log(chained);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "parameter_destructuring"),
        "1\ntrue\nok\n4\n2\n3\n5\n3\n7\n10\n"
    );
}

#[test]
fn compiles_sequence_expressions_with_await_and_rejection() {
    let source = r#"
        function effect(value: number): number {
            console.log(value); return value;
        }
        async function asyncEffect(value: number, fail: boolean): Promise<number> {
            await sleep(1);
            console.log(value);
            if (fail) throw "sequence failed";
            return value;
        }
        async function main(): Promise<void> {
            const result = (effect(1), await asyncEffect(2, false), effect(3), 4);
            console.log(result);
            try {
                const skipped = (effect(5), await asyncEffect(6, true), effect(7), 8);
                console.log(skipped);
            } catch (error) {
                console.log(error);
            }
            (effect(9), console.log("done"));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "sequence_expressions"),
        "1\n2\n3\n4\n5\n6\nsequence failed\n9\ndone\n"
    );
}

#[test]
fn compiles_void_expressions_with_await_and_rejection() {
    let source = r#"
        function effect(): number {
            console.log("sync");
            return 1;
        }
        async function asyncEffect(fail: boolean): Promise<number> {
            await sleep(1);
            if (fail) throw "failed";
            console.log("async");
            return 2;
        }
        async function main(): Promise<void> {
            void effect();
            void (await asyncEffect(false));
            try {
                void (await asyncEffect(true));
            } catch (error) {
                console.log(error);
            }
        }
    "#;
    assert_eq!(
        compile_and_run(source, "void_expressions"),
        "sync\nasync\nfailed\n"
    );
}

#[test]
fn logical_operators_short_circuit_sync_and_awaited_operands() {
    let source = r#"
        function flag(label: string, value: boolean): boolean {
            console.log(label);
            return value;
        }
        async function asyncFlag(label: string, value: boolean): Promise<boolean> {
            await sleep(1);
            console.log(label);
            return value;
        }
        async function main(): Promise<void> {
            console.log(false && flag("wrong-sync-and", true));
            console.log(true || flag("wrong-sync-or", false));
            console.log(true && flag("sync-and", true));
            console.log(false || flag("sync-or", true));
            console.log(false && (await asyncFlag("wrong-async-and", true)));
            console.log(true || (await asyncFlag("wrong-async-or", false)));
            console.log(true && (await asyncFlag("async-and", true)));
            console.log(false || (await asyncFlag("async-or", true)));
            try {
                console.log(true && (await new Promise<boolean>((resolve, reject) => {
                    reject("logical rejection");
                })));
            } catch (error) {
                console.log(error);
            }
            console.log(false && (await new Promise<boolean>((resolve, reject) => {
                reject("skipped rejection");
            })));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "logical_short_circuit"),
        "false\ntrue\nsync-and\ntrue\nsync-or\ntrue\nfalse\ntrue\nasync-and\ntrue\nasync-or\ntrue\nlogical rejection\nfalse\n"
    );
}

#[test]
fn compiles_string_template_literals_with_ordered_and_awaited_interpolation() {
    let source = r#"
        function word(label: string, value: string): string {
            console.log(label);
            return value;
        }
        async function delayed(value: string): Promise<string> {
            await sleep(1);
            console.log("awaited");
            return value;
        }
        async function main(): Promise<void> {
            console.log(`plain`);
            console.log(`A:${word("first", "x")}:${word("second", "y")}:Z`);
            const enabled: boolean = true;
            console.log(`enabled=${enabled}, disabled=${false}`);
            console.log(String(enabled));
            console.log(`numbers=${0},${-0},${1.5},${1000000000000000000000},${0.0000001}`);
            console.log(String(0 / 0));
            console.log(`before:${await delayed("done")}:after`);
            console.log(``);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "string_templates"),
        "plain\nfirst\nsecond\nA:x:y:Z\nenabled=true, disabled=false\ntrue\nnumbers=0,0,1.5,1e+21,1e-7\nNaN\nawaited\nbefore:done:after\n\n"
    );
}

#[test]
fn compiles_primitive_string_addition_with_ordered_awaits() {
    let source = r#"
        function text(label: string, value: string): string {
            console.log(label);
            return value;
        }
        function number(label: string, value: number): number {
            console.log(label);
            return value;
        }
        async function delayed(value: number): Promise<number> {
            await sleep(1);
            console.log("awaited-number");
            return value;
        }
        async function main(): Promise<void> {
            console.log(text("left", "value=") + number("right", 42));
            console.log(7 + " items");
            console.log("enabled=" + true);
            console.log(false + " flag");
            console.log("large=" + 1000000000000000000000);
            console.log("async=" + (await delayed(9)));
            let accumulated: string = "total=";
            accumulated += number("compound", 12);
            accumulated += true;
            console.log(accumulated);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "primitive_string_addition"),
        "left\nright\nvalue=42\n7 items\nenabled=true\nfalse flag\nlarge=1e+21\nawaited-number\nasync=9\ncompound\ntotal=12true\n"
    );
}

#[test]
fn compiles_native_number_conversion_and_awaited_strings() {
    let source = r#"
        async function delayed(value: string): Promise<string> {
            await sleep(1);
            console.log("awaited-number-input");
            return value;
        }
        async function main(): Promise<void> {
            console.log(Number(true));
            console.log(Number(false));
            console.log(Number(7));
            console.log(Number(" 42.5 "));
            console.log(Number("0xff"));
            console.log(Number("0o10"));
            console.log(Number("0b101"));
            console.log(Number(""));
            console.log(Number("bad") === Number("bad"));
            console.log(Number(await delayed("12")));
        }
    "#;
    assert_eq!(
        compile_and_run(source, "native_number_conversion"),
        "1\n0\n7\n42.5\n255\n8\n5\n0\nfalse\nawaited-number-input\n12\n"
    );
}

#[test]
fn compiles_async_arrows() {
    let source = r#"
        interface Worker { run(): Promise<number>; }
        async function delayed(value: number): Promise<number> {
            await sleep(1);
            return value;
        }
        async function pause(): Promise<void> {
            await sleep(1);
        }
        async function main(): Promise<void> {
            const offset: number = 2;
            const double: (value: number) => Promise<number> =
                async value => value * 2;
            console.log(await double(21));
            const choose: (enabled: boolean) => Promise<number> = async enabled => {
                if (enabled) return 40 + offset;
                return 0;
            };
            console.log(await choose(true));
            const chooseBeforeAwait: (enabled: boolean) => Promise<number> = async enabled => {
                if (enabled) return 41 + offset;
                await sleep(1);
                return 0;
            };
            console.log(await chooseBeforeAwait(true));
            const finish: () => Promise<void> = async () => {
                console.log("done");
            };
            await finish();
            const worker: Worker = {
                run: async (): Promise<number> => 40 + offset,
            };
            console.log(await worker.run());
            const adopted: (value: number) => Promise<number> =
                async value => await delayed(value);
            console.log(await adopted(42));
            const adoptedBlock: (enabled: boolean) => Promise<number> = async enabled => {
                if (enabled) return await delayed(40 + offset);
                return await delayed(0);
            };
            console.log(await adoptedBlock(true));
            const suspended: (value: number) => Promise<number> = async value => {
                const loaded: number = await delayed(value);
                await sleep(1);
                return loaded + offset;
            };
            console.log(await suspended(40));
            const suspendedExpression: (value: number) => Promise<number> =
                async value => (await delayed(value)) + offset;
            console.log(await suspendedExpression(40));
            const suspendedVoid: () => Promise<void> = async () => {
                return await pause();
            };
            await suspendedVoid();
            console.log("void");
            const catches: () => Promise<string> = async () => {
                let message: string = "missed";
                try {
                    await new Promise<number>((resolve, reject) => reject("boom"));
                } catch (error) {
                    message = "caught " + error;
                }
                return message;
            };
            console.log(await catches());
        }
    "#;
    assert_eq!(
        compile_and_run(source, "expression_bodied_async_arrow"),
        "42\n42\n43\ndone\n42\n42\n42\n42\n42\nvoid\ncaught boom\n"
    );
}

#[test]
fn compiles_readonly_and_awaited_utility_types() {
    let source = r#"
        type Frozen = Readonly<{ value: number; label: string }>;
        type FrozenBox<T> = Readonly<{ value: T }>;
        type Loaded = Awaited<Promise<number>>;
        type DeepLoaded = Awaited<Promise<Promise<number>>>;
        type LoadedValue<T> = Awaited<Promise<T>>;
        type Present = NonNullable<number | null | undefined>;
        type PresentValue<T> = NonNullable<T>;
        function main(): void {
            const frozen: Frozen = { value: 40, label: "ready" };
            const box: FrozenBox<string> = { value: frozen.label };
            const loaded: Loaded = frozen.value + 2;
            const generic: LoadedValue<number> = loaded;
            const deep: DeepLoaded = generic;
            const present: Present = deep;
            const genericPresent: PresentValue<number | null> = present;
            console.log(box.value);
            console.log(genericPresent);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "readonly_and_awaited_utility_types"),
        "ready\n42\n"
    );
}

#[test]
fn preserves_union_discriminants_for_inferred_arrays_and_promise_all() {
    let source = r#"
        type Result =
            { kind: "number"; value: number } |
            { kind: "text"; value: string };
        async function number(): Promise<Result> {
            return { kind: "number", value: 1 };
        }
        async function text(): Promise<Result> {
            return { kind: "text", value: "async" };
        }
        function print(values: Result[]): void {
            for (const item of values) {
                if (item.kind === "number") console.log(item.value + 10);
                else console.log(item.value + "!");
            }
        }
        async function main(): Promise<void> {
            const first: Result = { kind: "number", value: 2 };
            const second: Result = { kind: "text", value: "sync" };
            const inferred = [first, second];
            print(inferred);
            const pending: Promise<Result>[] = [number(), text()];
            const fromVariable = await Promise.all(pending);
            print(fromVariable);
            const fromLiteral = await Promise.all([number(), text()]);
            print(fromLiteral);
        }
    "#;
    assert_eq!(
        compile_and_run(source, "inferred_union_array_metadata"),
        "12\nsync!\n11\nasync!\n11\nasync!\n"
    );
}

#[test]
fn compiles_and_runs_a_napi_addon_with_async_work() {
    let dir =
        std::env::temp_dir().join(format!("thaw-hir-codegen-test-napi-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let addon_c = dir.join("addon.c");
    let addon = dir.join("addon.node");
    std::fs::write(&addon_c, r#"
        #include <stddef.h>
        #include <stdio.h>
        #include <stdlib.h>
        typedef void* napi_env; typedef void* napi_value; typedef void* napi_callback_info;
        typedef void* napi_async_work;
        typedef int napi_status;
        extern napi_status napi_get_cb_info(napi_env, napi_callback_info, size_t*, napi_value*, napi_value*, void**);
        extern napi_status napi_get_value_double(napi_env, napi_value, double*);
        extern napi_status napi_create_double(napi_env, double, napi_value*);
        extern napi_status napi_create_function(napi_env, const char*, size_t, napi_value (*)(napi_env,napi_callback_info), void*, napi_value*);
        extern napi_status napi_set_named_property(napi_env, napi_value, const char*, napi_value);
        extern napi_status napi_throw_error(napi_env, const char*, const char*);
        extern napi_status napi_get_undefined(napi_env, napi_value*);
        extern napi_status napi_create_async_work(napi_env, napi_value, napi_value, void (*)(napi_env, void*), void (*)(napi_env, napi_status, void*), void*, napi_async_work*);
        extern napi_status napi_queue_async_work(napi_env, napi_async_work);
        extern napi_status napi_delete_async_work(napi_env, napi_async_work);
        extern napi_status napi_call_function(napi_env, napi_value, napi_value, size_t, const napi_value*, napi_value*);
        struct async_data { napi_env env; napi_async_work work; napi_value callback; int answer; };
        static napi_value add(napi_env env, napi_callback_info info) {
            size_t argc = 2; napi_value argv[2]; double a, b; napi_value result;
            napi_get_cb_info(env, info, &argc, argv, 0, 0);
            napi_get_value_double(env, argv[0], &a); napi_get_value_double(env, argv[1], &b);
            napi_create_double(env, a + b, &result); return result;
        }
        static napi_value fail(napi_env env, napi_callback_info info) {
            (void)info; napi_throw_error(env, 0, "native addon failed"); return 0;
        }
        static void execute_async(napi_env env, void* raw) {
            (void)env; ((struct async_data*)raw)->answer = 42;
        }
        static void complete_async(napi_env env, napi_status status, void* raw) {
            struct async_data* data = raw;
            printf("async %d status %d\n", data->answer, status);
            napi_value args[2], ignored;
            napi_get_undefined(env, &args[0]);
            napi_create_double(env, data->answer, &args[1]);
            napi_call_function(env, args[0], data->callback, 2, args, &ignored);
            napi_call_function(env, args[0], data->callback, 2, args, &ignored);
            napi_delete_async_work(env, data->work);
            free(data);
        }
        static napi_value schedule(napi_env env, napi_callback_info info) {
            struct async_data* data = calloc(1, sizeof(*data));
            size_t argc = 1;
            napi_value result;
            napi_get_cb_info(env, info, &argc, &data->callback, 0, 0);
            data->env = env;
            napi_create_async_work(env, 0, 0, execute_async, complete_async, data, &data->work);
            napi_queue_async_work(env, data->work);
            napi_get_undefined(env, &result);
            return result;
        }
        __attribute__((visibility("default"))) napi_value napi_register_module_v1(napi_env env, napi_value exports) {
            napi_value add_fn, fail_fn, schedule_fn;
            napi_create_function(env, "add", 3, add, 0, &add_fn);
            napi_create_function(env, "fail", 4, fail, 0, &fail_fn);
            napi_create_function(env, "schedule", 8, schedule, 0, &schedule_fn);
            napi_set_named_property(env, exports, "add", add_fn);
            napi_set_named_property(env, exports, "fail", fail_fn);
            napi_set_named_property(env, exports, "schedule", schedule_fn); return exports;
        }
    "#).unwrap();
    assert!(cc_command()
        .args(["-shared", "-fPIC"])
        .arg(&addon_c)
        .arg("-o")
        .arg(&addon)
        .status()
        .unwrap()
        .success());

    let source = format!(
        r#"
        function main(): void {{
            loadNativeAddon("{}");
            console.log(Number(callNativeAddon("add", JSON.parse("[20,22]"))));
            try {{
                const ignored = callNativeAddon("fail", JSON.parse("[]"));
            }} catch (error) {{
                console.log(error);
            }}
            const scheduled = callNativeAddonWithCallback(
                "schedule",
                JSON.parse("[]"),
                (error: Json, result: Json): Json => {{
                    console.log(Number(result));
                    return result;
                }}
            );
        }}
    "#,
        addon.display()
    );
    let module = thaw_parser::parse_typescript(&source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "napi_addon");
    compiler.compile_program(&program).unwrap();
    let obj = dir.join("out.o");
    let exe = dir.join("out");
    compiler.write_object_file(&obj).unwrap();
    let arena = build_staticlib("thaw-arena");
    let std = build_staticlib("thaw-std");
    let runtime = build_staticlib("thaw-runtime");
    let napi = build_staticlib("thaw-napi");
    assert!(cc_command()
        .arg(&obj)
        .arg(&arena)
        .arg(&std)
        .arg(&runtime)
        .arg(&napi)
        .args(["-lm", "-ldl", "-lpthread", "-Wl,--export-dynamic", "-o"])
        .arg(&exe)
        .status()
        .unwrap()
        .success());
    let output = Command::new(&exe).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "42\nnative addon failed\nasync 42 status 0\n42\n42\n"
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// Same mechanism, but through the Lambda `handler` entry point instead
/// of `main` (`emit_lambda_entry` has its own copy of the
/// `call_module_init_if_present` call, see hir_codegen.rs).
#[test]
fn runs_module_init_before_lambda_handler() {
    let source = r#"
        function __thaw_module_init(): void {
            loadScript("function greet() { return 'hi from registry'; }");
        }

        function handler(event: string): string {
            const result = callDynamic("greet", JSON.parse("[]"));
            return String(result);
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();

    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "module_init_lambda");
    compiler.compile_program(&program).unwrap();

    let dir = std::env::temp_dir().join(format!(
        "thaw-hir-codegen-test-module_init_lambda-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let obj_path = dir.join("out.o");
    let exe_path = dir.join("out");
    compiler.write_object_file(&obj_path).unwrap();

    let arena_lib = build_staticlib("thaw-arena");
    let runtime_lib = build_staticlib("thaw-runtime");
    let std_lib = build_staticlib("thaw-std");
    let quickjs_lib = build_staticlib("thaw-quickjs");

    let link_status = cc_command()
        .arg(&obj_path)
        .arg(&arena_lib)
        .arg(&std_lib)
        .arg(&runtime_lib)
        .arg(&quickjs_lib)
        .arg("-lm")
        .arg("-o")
        .arg(&exe_path)
        .status()
        .expect("failed to invoke system `cc` linker");
    assert!(link_status.success(), "linking failed");

    // Same mock Lambda Runtime API protocol as
    // `compiles_and_runs_a_lambda_handler_against_a_mock_runtime_api`:
    // bind a real TCP listener, tell the binary about it via
    // `AWS_LAMBDA_RUNTIME_API`, and check the response it posts back.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();

    let (tx, rx) = mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut conn, _) = listener.accept().unwrap();
        let mut buf = [0u8; 4096];
        let _ = conn.read(&mut buf).unwrap();
        let body = "\"ping\"";
        let response = format!(
            "HTTP/1.1 200 OK\r\nLambda-Runtime-Aws-Request-Id: test-req-1\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        conn.write_all(response.as_bytes()).unwrap();
        drop(conn);

        let (mut conn, _) = listener.accept().unwrap();
        let mut buf = Vec::new();
        conn.read_to_end(&mut buf).unwrap();
        tx.send(String::from_utf8_lossy(&buf).into_owned()).unwrap();

        let response =
            "HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
        conn.write_all(response.as_bytes()).unwrap();
    });

    let mut child = Command::new(&exe_path)
        .env("AWS_LAMBDA_RUNTIME_API", &addr)
        .stdout(Stdio::piped())
        .spawn()
        .expect("failed to spawn compiled Lambda handler binary");

    let post_request = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("handler never posted a response to the mock runtime API");
    server.join().unwrap();

    let _ = child.kill();
    let _ = child.wait_with_output();
    let _ = std::fs::remove_dir_all(&dir);

    assert!(post_request.starts_with("POST /2018-06-01/runtime/invocation/test-req-1/response"));
    assert!(
        post_request.ends_with("hi from registry"),
        "response body should be the module-init-loaded function's result, got: {post_request}"
    );
}

/// Async functions without an explicit suspension still use the uniform
/// Promise-handle ABI and resolve their completion in the initial state.
#[test]
fn compiles_async_functions_without_explicit_suspension() {
    let source = r#"
        async function computeStage(): Promise<string> {
            const s: string = process.env.STAGE;
            return s;
        }

        async function addAsync(a: number, b: number): Promise<number> {
            return a + b;
        }

        async function main(): Promise<void> {
            const stage: string = await computeStage();
            console.log(stage);
            const sum: number = await addAsync(2, 3);
            console.log(sum);
        }
    "#;
    assert_eq!(
        compile_and_run_with_env(source, "async_v1", &[("STAGE", "prod")]),
        "prod\n5\n"
    );
}

#[test]
fn await_sleep_is_driven_by_the_runtime_event_loop() {
    let source = r#"
        async function main(): Promise<void> {
            console.log("before");
            await sleep(1);
            console.log("after");
        }
    "#;
    assert_eq!(compile_and_run(source, "await_sleep"), "before\nafter\n");
}

#[test]
fn async_main_drains_napi_before_destroying_its_promise() {
    let source = r#"
        async function main(): Promise<void> {
            pollNativeAddonEvents();
            await Promise.resolve(undefined);
        }
    "#;
    let module = thaw_parser::parse_typescript(source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "async_napi_entry");
    compiler.compile_program(&program).unwrap();
    let ir = compiler.print_to_string();
    let drain = ir.find("drain_napi_for_async_main").unwrap();
    let resume = ir.find("resume_async_main_after_napi").unwrap();
    let destroy = ir[resume..].find("call void @thaw_promise_destroy").unwrap();
    assert!(drain < resume && destroy > 0);
}
