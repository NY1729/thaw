//! `thaw-js-scan [--installed=a,b,@s/c] <bundle.js>...` : print, per bundle, the constructs thaw-js cannot run
//! (tab separated). `--installed` names the registry packages installed (see `ScanContext`); none by default.
fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let installed: std::collections::HashSet<String> = match args
        .first()
        .and_then(|a| a.strip_prefix("--installed="))
        .map(|list| {
            list.split(',')
                .filter(|p| !p.is_empty())
                .map(str::to_string)
                .collect()
        }) {
        Some(set) => {
            args.remove(0);
            set
        }
        None => Default::default(),
    };
    let known = std::collections::HashSet::new();
    let ctx = thaw_js::scan::ScanContext {
        known_methods: &known,
        installed_packages: &installed,
    };
    if args.is_empty() {
        eprintln!("usage: thaw-js-scan [--installed=a,b] <bundle.js>...");
        std::process::exit(2);
    }
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
        match thaw_js::scan::scan_bundle_in(&source, &ctx) {
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
