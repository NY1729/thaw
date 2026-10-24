//! `thaw-js-scan <bundle.js>...` : print, per bundle, the constructs thaw-js cannot run (tab separated).
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: thaw-js-scan <bundle.js>...");
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
        match thaw_js::scan::scan_bundle(&source) {
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
