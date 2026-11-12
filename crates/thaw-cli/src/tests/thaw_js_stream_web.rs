// node:stream and node:stream/web are both allowlisted for thaw-js routing. Kept separate from the
// pinned allowlist test.

#[test]
fn thaw_js_stream_web_is_allowlisted() {
    assert!(THAW_JS_BUILTIN_MODULES.contains(&"node:stream/web"));
    assert!(THAW_JS_BUILTIN_MODULES.contains(&"node:stream"));
}

#[test]
fn thaw_js_routing_serves_a_writable_with_a_write_callback_without_linking_quickjs() {
    let dir = std::env::temp_dir().join(format!("thaw-cli-thaw-js-writable-{}", std::process::id()));
    let registry = dir.join("modules");
    std::fs::create_dir_all(&registry).unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        "import { Writable } from 'node:stream';\n\
         function main(): void {\n\
           const out: string[] = [];\n\
           const sink = new Writable({ write(chunk: any, enc: string, cb: any) { out.push(String(chunk)); cb(); } });\n\
           sink.write('a');\n\
           sink.end();\n\
           console.log(out.join(','));\n\
         }\n",
    )
    .unwrap();
    let output = dir.join("app");
    let manifest = thaw_js_build(&entry, &output, &registry, true);
    assert_eq!(manifest["quickjs"], false, "{manifest}");
    // Same text real Node prints for the equivalent JavaScript.
    assert_eq!(thaw_js_run_guarded(&output), "a\n");
    let bytes = std::fs::read(&output).unwrap();
    assert!(!bytes.windows(8).any(|window| window == b"rquickjs"), "QuickJS was linked");
    let _ = std::fs::remove_dir_all(dir);
}

// Builds a Writable whose `write` method is `write_method`, routes it on thaw-js, and returns the
// program's stdout. Checks the program does not link QuickJS.
fn thaw_js_writable_method_output(tag: &str, write_method: &str) -> String {
    let dir = std::env::temp_dir().join(format!("thaw-cli-thaw-js-writable-{tag}-{}", std::process::id()));
    let registry = dir.join("modules");
    std::fs::create_dir_all(&registry).unwrap();
    let entry = dir.join("main.ts");
    std::fs::write(
        &entry,
        format!(
            "import {{ Writable }} from 'node:stream';\n\
             function main(): void {{\n\
               const sink = new Writable({{ {write_method} }});\n\
               sink.write('a');\n\
               sink.end();\n\
             }}\n"
        ),
    )
    .unwrap();
    let output = dir.join("app");
    let manifest = thaw_js_build(&entry, &output, &registry, true);
    assert_eq!(manifest["quickjs"], false, "{manifest}");
    let stdout = thaw_js_run_guarded(&output);
    let bytes = std::fs::read(&output).unwrap();
    assert!(!bytes.windows(8).any(|window| window == b"rquickjs"), "QuickJS was linked");
    let _ = std::fs::remove_dir_all(dir);
    stdout
}

// Same text real Node prints for the equivalent JavaScript: the callback gets a Buffer chunk,
// the 'buffer' encoding, and a callable `cb`.
#[test]
fn thaw_js_routing_writable_write_with_typed_callback_parameter_runs_on_thaw_js() {
    let stdout = thaw_js_writable_method_output(
        "typed",
        "write(chunk: any, enc: string, cb: () => void) { console.log(typeof cb, enc, String(chunk)); cb(); }",
    );
    assert_eq!(stdout, "function buffer a\n");
}

#[test]
fn thaw_js_routing_writable_write_with_unannotated_parameters_runs_on_thaw_js() {
    let stdout = thaw_js_writable_method_output(
        "untyped",
        "write(chunk, enc, cb) { console.log(typeof cb, enc, String(chunk)); cb(); }",
    );
    assert_eq!(stdout, "function buffer a\n");
}

// Arrow-valued option property (not a method): the plain callback ABI, which the IR test
// `thaw_js_routed_ctor_arrow_callback_object_literal_is_json_with_handles` also covers.
#[test]
fn thaw_js_routing_writable_write_arrow_property_runs_on_thaw_js() {
    let stdout = thaw_js_writable_method_output(
        "arrow",
        "write: (chunk: any, enc: string, cb: () => void) => { console.log(typeof cb, enc, String(chunk)); cb(); }",
    );
    assert_eq!(stdout, "function buffer a\n");
}
