// The build's feature scan of a bundle (`scan_bundle_for_build`): R6 method excuses, addon mode, and the `intl` feature.

fn blockers_of(report: &thaw_js::scan::Report) -> Vec<String> {
    report.blockers.keys().cloned().collect()
}

#[test]
fn scan_build_excuses_a_method_a_served_builtin_defines() {
    // `createScript` is defined by a served builtin (R6), so the build passes it; a bare scan blocks on it. The vm
    // load is dynamic: createScript's owner is vm, and only a loaded owner makes the call a finding (the owner rule)
    let bundle = thaw_js::scan::script_as_bundle(
        "module.exports = function (b) { process.getBuiltinModule('node:vm'); return b.createScript('x'); };",
    );
    let empty = std::collections::HashSet::new();
    let bare = thaw_js::scan::scan_bundle_in(
        &bundle,
        &thaw_js::scan::ScanContext {
            known_methods: &empty,
            installed_packages: &empty,
            served_peers: &empty,
        },
    )
    .unwrap();
    assert!(blockers_of(&bare).contains(&"method:createScript".to_string()), "{:?}", bare.blockers);
    let build = scan_bundle_for_build(&bundle, false, &empty, &empty).unwrap();
    assert!(build.is_clean(), "{:?}", build.blockers);
}

#[test]
fn scan_build_scans_a_native_addon_bundle_in_addon_mode() {
    // a `.node` require the bundle does not map: a blocker for a plain package, tolerated for a native addon
    let bundle = thaw_js::scan::script_as_bundle("module.exports = function () { return require('./build/native.node'); };");
    let empty = std::collections::HashSet::new();
    let plain = scan_bundle_for_build(&bundle, false, &empty, &empty).unwrap();
    assert!(!plain.is_clean(), "a plain package's unmapped .node require must block");
    let addon = scan_bundle_for_build(&bundle, true, &empty, &empty).unwrap();
    assert!(addon.is_clean(), "{:?}", addon.blockers);
}

#[test]
fn scan_build_has_intl_with_the_intl_feature() {
    // thaw-registry enables thaw-js's `intl` feature (as thaw-cli does): `Intl` is served, not a `global:Intl` blocker
    let bundle = thaw_js::scan::script_as_bundle("module.exports = function () { return new Intl.NumberFormat('en').format(1); };");
    let empty = std::collections::HashSet::new();
    let report = scan_bundle_for_build(&bundle, false, &empty, &empty).unwrap();
    assert!(report.is_clean(), "{:?}", report.blockers);
}

#[test]
fn scan_build_serves_an_installed_peer_the_bundle_requires() {
    // an installed package a bundle requires by its exact name blocks a bare scan; the build's served set serves it
    let bundle = thaw_js::scan::script_as_bundle("module.exports = function () { return require('pg'); };");
    let installed: std::collections::HashSet<String> = ["pg".to_string()].into_iter().collect();
    let none = std::collections::HashSet::new();
    let bare = scan_bundle_for_build(&bundle, false, &installed, &none).unwrap();
    assert!(!bare.is_clean(), "an installed peer that is not served must block");
    let served: std::collections::HashSet<String> = crate::served_peers_of(&bundle, &installed).unwrap().into_iter().collect();
    assert_eq!(served, installed);
    let build = scan_bundle_for_build(&bundle, false, &installed, &served).unwrap();
    assert!(build.is_clean(), "{:?}", build.blockers);
}
