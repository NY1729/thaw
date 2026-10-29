/// Generates a program shaped like `speedrun/scale{pairs}.ts`: `pairs` pairs of `f{k}`/`g{k}`
/// functions plus a `main` that calls every pair.
fn scale_program(pairs: usize) -> String {
    let mut source = String::from("interface P { id: number; name: string; tags: string[]; }\n");
    for k in 0..pairs {
        let m = k % 3;
        source.push_str(&format!(
            "function f{k}(a: number, b: number): number {{\n  let s = 0;\n  for (let i = 0; i < a; i++) {{ if (i % 3 === {m}) {{ s += i * b; }} else {{ s -= 1; }} }}\n  const arr: number[] = [a, b, s];\n  return arr.map((x) => x + {k}).reduce((p, c) => p + c, 0);\n}}\n"
        ));
        source.push_str(&format!(
            "function g{k}(p: P): string {{ return p.name + \":\" + p.tags.join(\",\") + String(p.id + {k}); }}\n"
        ));
    }
    source.push_str("function main(): void {\n  let t = 0;\n");
    for k in 0..pairs {
        source.push_str(&format!(
            "  t += f{k}({}, 2); console.log(g{k}({{ id: {k}, name: 'n{k}', tags: ['a','b'] }}));\n",
            k + 5
        ));
    }
    source.push_str("  console.log(t);\n}\n");
    source
}

#[test]
fn storage_analysis_scales_to_240_pairs() {
    // The timing run does not also replay the quadratic reference algorithm.
    STORAGE_REFERENCE_CHECK.with(|check| check.set(false));
    let mut elapsed_240 = None;
    for pairs in [60] { // PROBE
        let module = thaw_parser::parse_typescript(&scale_program(pairs)).expect("parse error");
        let started = std::time::Instant::now();
        lower_module(&module).expect("lowering error");
        let elapsed = started.elapsed();
        eprintln!("storage scale: {pairs} pairs lowered in {elapsed:?}");
        if pairs == 240 {
            elapsed_240 = Some(elapsed);
        }
    }
    let elapsed = elapsed_240.expect("240-pair run");
    assert!(
        elapsed < std::time::Duration::from_secs(10),
        "240-pair program took {elapsed:?} to lower (budget 10 s)"
    );
}
