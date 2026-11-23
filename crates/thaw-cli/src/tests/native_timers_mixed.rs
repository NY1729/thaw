// Native timers mixed with a JS engine (or another event-loop owner): the build rebuilds with timers
// served by the build's engine. Expected output recorded from Node 22.23.1.
//
// NOT RUN locally (the coordinator runs them): `cargo test -p thaw-cli --lib -- native_timers_mixed`
// (under `ulimit -v 8388608`, `RUST_MIN_STACK=16777216`, one process, as with the other thaw-cli builds).

/// A thaw-js routed package (`tjs-kit`) mixed with a native-eligible `setTimeout`: the native attempt
/// conflicts, the retry stays on thaw-js (no thaw-js, no "fell back" blocker) and prints the Node order.
#[test]
fn native_timers_mixed_thaw_js_package_retry_stays_on_thaw_js() {
    let (dir, registry, entry) = thaw_js_fixture(
        "native-timers-mixed-package",
        "import { apply } from 'tjs-kit';\nfunction main(): void {\n  setTimeout((): void => console.log(\"timer\"), 5);\n  console.log(apply([1, 2, 3], (x: number): number => x * 2).join(','));\n  console.log(\"sync\");\n}\n",
    );
    let output = dir.join("app");
    let manifest = thaw_js_build(&entry, &output, &registry);
    assert_eq!(manifest["thaw_js_routed"], serde_json::json!(["tjs-kit"]));
    assert!(!manifest.to_string().contains("fell back to QuickJS"), "{manifest}");
    assert_eq!(thaw_js_run_guarded(&output), "2,4,6\nsync\ntimer\n");
    let _ = std::fs::remove_dir_all(dir);
}

