fn pruned_names(source: &str) -> Vec<String> {
    let dts = "export declare function used(x: any): any;\nexport declare function unusedA(x: any): any;\nexport declare function unusedB(x: any): any;\n";
    let mut functions = thaw_bridge::parse_dts_named(
        dts,
        thaw_parser::common::FileName::Custom("pkg.d.ts".into()),
    )
    .unwrap();
    prune_unused_functions(&mut functions, "pkg", source, true);
    functions.into_iter().map(|f| f.name).collect()
}

#[test]
fn shim_pruning_keeps_only_used_functions_for_plain_imports() {
    assert_eq!(
        pruned_names("import { used } from 'pkg';\nused();"),
        ["used"]
    );
    assert_eq!(pruned_names("import p from 'pkg';\np.used(1);"), ["used"]);
    assert_eq!(
        pruned_names("import * as p from \"pkg\";\np.used(1); p.unusedB(2);"),
        ["used", "unusedB"]
    );
}

#[test]
fn shim_pruning_keeps_everything_when_reachability_is_unclear() {
    let all = ["used", "unusedA", "unusedB"];
    for source in [
        "import p from 'pkg';\nconst q = p;\nq.used();",
        "import p from 'pkg';\np(1); p.used();",
        "const p = require('pkg');\np.used();",
        "export { used } from 'pkg';",
        "import p from 'pkg';\nexport const o = [p];",
        "p.used();",
    ] {
        assert_eq!(pruned_names(source), all, "{source}");
    }
}
