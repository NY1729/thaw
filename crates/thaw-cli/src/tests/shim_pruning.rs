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
fn shim_pruning_handles_a_default_binding_named_like_the_package() {
    assert_eq!(
        pruned_names("import pkg from \"pkg\";\npkg.used(1);"),
        ["used"]
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

fn nested_chain_pruned_names(source: &str) -> Vec<String> {
    let dts = "export declare function mid(): string;\n\
        export declare namespace mid {\n\
            export { leaf as member };\n\
        }\n\
        export declare function leaf(): string;\n\
        export declare function other(): string;\n";
    let filename = thaw_parser::common::FileName::Custom("pkg.d.ts".into());
    let unpruned = thaw_bridge::parse_dts_named(dts, filename.clone()).unwrap();
    let nested = thaw_bridge::nested_namespace_members_named(dts, &filename);
    assert_eq!(nested["mid"]["member"], "leaf");
    let mut functions = unpruned.clone();
    prune_unused_functions(&mut functions, "pkg", source, true);
    keep_nested_namespace_targets(unpruned, &mut functions, &nested, source);
    functions.into_iter().map(|f| f.name).collect()
}

#[test]
fn shim_pruning_keeps_a_nested_namespace_target_when_its_namespace_is_mentioned() {
    assert_eq!(
        nested_chain_pruned_names("import pkg from 'pkg';\npkg.mid.member();"),
        ["mid", "leaf"]
    );
}

#[test]
fn shim_pruning_drops_a_nested_namespace_target_when_its_namespace_is_not_mentioned() {
    assert_eq!(
        nested_chain_pruned_names("import pkg from 'pkg';\npkg.other();"),
        ["other"]
    );
}

fn synthetic_chain_pruned_names(source: &str) -> Vec<String> {
    // `coerce.number` has no top-level export: the registry flattens it to a synthetic `__thaw_ns_coerce_number`.
    let dts = "export declare namespace coerce {\n\
            export { __thaw_ns_coerce_number as number };\n\
        }\n\
        export declare function coerce(): string;\n\
        export declare function __thaw_ns_coerce_number(): number;\n";
    let filename = thaw_parser::common::FileName::Custom("pkg.d.ts".into());
    let unpruned = thaw_bridge::parse_dts_named(dts, filename.clone()).unwrap();
    let nested = thaw_bridge::nested_namespace_members_named(dts, &filename);
    assert_eq!(nested["coerce"]["number"], "__thaw_ns_coerce_number");
    let mut functions = unpruned.clone();
    prune_unused_functions(&mut functions, "pkg", source, true);
    keep_nested_namespace_targets(unpruned, &mut functions, &nested, source);
    functions.into_iter().map(|f| f.name).collect()
}

#[test]
fn shim_pruning_does_not_restore_a_synthetic_namespace_target() {
    assert_eq!(
        synthetic_chain_pruned_names("import pkg from 'pkg';\npkg.coerce.number();"),
        ["coerce"]
    );
}
