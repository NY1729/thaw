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

#[test]
fn scan_build_accepts_the_zlib_builtin_without_the_zstd_feature() {
    // the zstd stream native only exists with the `zstd` feature; the zlib builtin must reach it through a typeof-guarded
    // alias so a build without the feature still scans clean (an unguarded call refused every package that requires zlib)
    let dir = temp_registry("scan_build_zlib_builtin");
    fs::write(dir.join("index.js"), "module.exports = require('node:zlib');").unwrap();
    let node_modules = temp_registry("scan_build_zlib_builtin_node_modules");
    let (bundle, _, _, _) = bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    let empty = std::collections::HashSet::new();
    let report = scan_bundle_for_build(&bundle, false, &empty, &empty).unwrap();
    assert!(report.is_clean(), "{:?}", report.blockers);
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}

fn scan_cache_test_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("thaw-scan-cache-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    dir
}

#[test]
fn scan_cache_roundtrips_a_report_and_rejects_corrupt_or_foreign_entries() {
    let bundle = thaw_js::scan::script_as_bundle("module.exports = function () { return require('./x.node'); };");
    let empty = std::collections::HashSet::new();
    let report = scan_bundle_for_build_uncached(&bundle, false, &empty, &empty).unwrap();
    assert!(!report.blockers.is_empty());
    let key = scan_cache_key(&bundle, false, &empty, &empty);
    let dir = scan_cache_test_dir("rt");
    assert!(scan_cache_load(&dir, &key).is_none());
    scan_cache_store(&dir, &key, &report);
    let back = scan_cache_load(&dir, &key).unwrap();
    assert_eq!(format!("{report:?}"), format!("{back:?}"));
    // truncated / garbage entries are misses
    let path = dir.join(format!("{key}.json"));
    let text = fs::read_to_string(&path).unwrap();
    fs::write(&path, &text[..text.len() / 2]).unwrap();
    assert!(scan_cache_load(&dir, &key).is_none());
    fs::write(&path, "").unwrap();
    assert!(scan_cache_load(&dir, &key).is_none());
    // an entry whose embedded key differs (renamed/foreign file) is a miss
    fs::write(&path, text.replace(&key, &"0".repeat(64))).unwrap();
    assert!(scan_cache_load(&dir, &key).is_none());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn scan_cache_key_covers_every_input() {
    let set = |names: &[&str]| names.iter().map(|n| n.to_string()).collect::<std::collections::HashSet<_>>();
    let base = scan_cache_key("b", false, &set(&["a"]), &set(&[]));
    assert_eq!(base, scan_cache_key("b", false, &set(&["a"]), &set(&[])));
    assert_ne!(base, scan_cache_key("c", false, &set(&["a"]), &set(&[])));
    assert_ne!(base, scan_cache_key("b", true, &set(&["a"]), &set(&[])));
    assert_ne!(base, scan_cache_key("b", false, &set(&["a", "z"]), &set(&[])));
    assert_ne!(base, scan_cache_key("b", false, &set(&[]), &set(&["a"])));
    assert_ne!(base, scan_cache_key("b", false, &set(&[]), &set(&[])));
}

#[test]
fn scan_cache_store_bounds_the_directory() {
    let dir = scan_cache_test_dir("bound");
    let report = thaw_js::scan::Report::default();
    for i in 0..300 {
        scan_cache_store(&dir, &format!("{i:064}"), &report);
    }
    let count = fs::read_dir(&dir).unwrap().count();
    assert!(count <= 256, "{count}");
    let _ = fs::remove_dir_all(&dir);
}
