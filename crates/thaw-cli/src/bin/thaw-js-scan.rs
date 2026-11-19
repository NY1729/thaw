//! `thaw-js-scan [--installed=a,b,@s/c] [--addon=stem,...] <bundle.js>...` : print, per bundle, the constructs
//! thaw-js cannot run (tab separated), with the build's verdict (`thaw_registry::scan_bundle_for_build`).
//! The first line is `#scan<TAB>build<TAB>installed=N<TAB>addon=...`, the mode the rows were scanned in.
//! `--installed` names the registry packages installed (the build reads them from the registry; none by default).
//! `--addon` lists the bundle file stems of native-addon packages (the build's `native_addon`, which the bundle
//! file does not say), scanned in addon mode. This crate builds thaw-js with the `intl` and `wasm` features.
fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut installed = std::collections::HashSet::new();
    let mut addon = std::collections::HashSet::new();
    while let Some(flag) = args.first().filter(|a| a.starts_with("--")).cloned() {
        args.remove(0);
        let list = |value: &str| -> Vec<String> {
            value
                .split(',')
                .filter(|p| !p.is_empty())
                .map(str::to_string)
                .collect()
        };
        if let Some(value) = flag.strip_prefix("--installed=") {
            installed.extend(list(value));
        } else if let Some(value) = flag.strip_prefix("--addon=") {
            addon.extend(list(value));
        } else {
            eprintln!("thaw-js-scan: unknown flag {flag}");
            std::process::exit(2);
        }
    }
    if args.is_empty() {
        eprintln!("usage: thaw-js-scan [--installed=a,b] [--addon=stem,...] <bundle.js>...");
        std::process::exit(2);
    }
    let mut addon_list: Vec<&String> = addon.iter().collect();
    addon_list.sort();
    println!(
        "#scan\tbuild\tinstalled={}\taddon={}",
        installed.len(),
        addon_list
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(",")
    );
    for path in args {
        let name = std::path::Path::new(&path)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let source = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                println!("{name}\terror\t{e}");
                continue;
            }
        };
        let native_addon = addon.contains(&name);
        match thaw_registry::scan_bundle_for_build(&source, native_addon, &installed) {
            Ok(r) => {
                println!(
                    "{name}\tmodules\t{}/{}\t{}",
                    r.factories_reachable,
                    r.factories_total,
                    if r.is_clean() { "CLEAN" } else { "BLOCKED" }
                );
                for (feature, (count, first)) in &r.blockers {
                    println!("{name}\tblocker\t{feature}\t{count}\t{first}");
                }
            }
            Err(e) => println!("{name}\terror\t{e}"),
        }
    }
}
