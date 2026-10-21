// Wave LB: a JavaScript Map/Set copy-in that fails at a later entry (here the third) must throw a
// catchable TypeError, never call the native function, leave nothing half-built behind, and not
// disturb later successful copy-ins. `LB_COPYIN_ROUNDS` raises the loop count for the manual
// ASAN/LeakSanitizer comparison recorded in triage/lbstatus.json.

#[test]
fn lb_contract_ffi_map_copyin_failing_at_third_entry_is_clean() {
    let rounds: u32 = std::env::var("LB_COPYIN_ROUNDS").ok().and_then(|value| value.parse().ok()).unwrap_or(20);
    // LB_CONTROL=1 swaps the failing copy-in for a plain `throw new TypeError`, the control for the
    // LeakSanitizer comparison (the catch path itself must not be blamed on the copy-in).
    let control = std::env::var("LB_CONTROL").is_ok();
    let flush: u32 = if std::env::var("LB_FLUSH").is_ok() { 60 } else { 0 };
    // LB_SUCCESS=1 passes the valid collections instead (success-path control).
    let (map_arg, set_arg) = if std::env::var("LB_SUCCESS").is_ok() { ("good", "goodSet") } else { ("bad", "badSet") };
    // LB_READ=1 only reads the captured collections (no FFI call): a closure-capture control.
    let (map_call, set_call) = if std::env::var("LB_READ").is_ok() {
        (format!("String({map_arg}.size)"), format!("String({set_arg}.size)"))
    } else {
        (format!("native_number_map_sum({map_arg})"), format!("native_set_sum({set_arg})"))
    };
    let source = format!(
        r#"
        declare function native_number_map_sum(value: Map<string, number>): number;
        declare function native_set_sum(value: Set<number>): number;
        declare function native_calls(): number;
        async function main(): Promise<void> {{
            const bad: any = new Function("return new Map([['a', 1], ['b', 2], ['c', 'wrong']])")();
            const badSet: any = new Function("return new Set([1, 2, 'wrong'])")();
            const good: any = new Function("return new Map([['a', 1], ['b', 2], ['c', 3]])")();
            const goodSet: any = new Function("return new Set([1, 2, 3])")();
            let rejected = 0;
            for (let i = 0; i < {rounds}; i++) {{
                try {{ if ({control}) throw new TypeError("control"); {map_call}; }} catch (error) {{ if (error instanceof TypeError) rejected++; }}
                try {{ if ({control}) throw new TypeError("control"); {set_call}; }} catch (error) {{ if (error instanceof TypeError) rejected++; }}
                // Each timer turn resets the arena, which is when argument temporaries registered as
                // arena-owned Json roots are released.
                await new Promise<void>(resolve => setTimeout(resolve, 0));
            }}
            // LB_FLUSH=1 (manual leak runs): enough traced arena growth across timer turns for
            // reset_if_grown to run and reclaim arena-owned Json roots before the process exits.
            for (let flush = 0; flush < {flush}; flush++) {{
                const big: number[] = [];
                for (let k = 0; k < 20000; k++) big.push(k + flush);
                if (big.length !== 20000) console.log("unexpected");
                await new Promise<void>(resolve => setTimeout(resolve, 0));
            }}
            console.log(rejected, native_calls());
            console.log(native_number_map_sum(good), native_set_sum(goodSet), native_calls());
        }}
    "#
    );
    let module = thaw_parser::parse_typescript(&source).unwrap();
    let program = thaw_hir::lower_module(&module).unwrap();
    let context = Context::create();
    let mut compiler = HirCompiler::new(&context, "lb_ffi_map_copyin_failure");
    compiler.compile_program(&program).unwrap();
    if let Ok(path) = std::env::var("LB_DUMP_IR") {
        std::fs::write(path, compiler.module.print_to_string().to_string()).unwrap();
    }
    let dir = std::env::temp_dir().join(format!("thaw-lb-copyin-failure-{}", std::process::id()));
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
        double native_number_map_sum(void *map) { calls++; return sum_words(thaw_map_snapshot_values(map)); }
        double native_set_sum(void *set) { calls++; return sum_words(thaw_map_snapshot_keys(set)); }
        double native_calls(void) { return calls; }
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
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("{} 0\n6 6 2\n", rounds * 2)
    );
    std::fs::remove_dir_all(dir).unwrap();
}
