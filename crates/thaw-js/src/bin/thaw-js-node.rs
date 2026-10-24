//! `thaw-js-node <script.js>`: run a script like `node` does (event loop included); used by the
//! differential tests to compare stdout and the exit code with real Node.
use thaw_js::host::Package;
use thaw_js::Interp;

fn main() {
    // thaw-js-node [--pkg name=bundle.js] <script.js>: with --pkg, `require(name)` serves the bundle
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let pkg = match args.iter().position(|a| a == "--pkg") {
        Some(i) => {
            let spec = args.get(i + 1).cloned().unwrap_or_default();
            args.drain(i..(i + 2).min(args.len()));
            spec.split_once('=')
                .map(|(n, p)| (n.to_string(), p.to_string()))
        }
        None => None,
    };
    let Some(path) = args.first().cloned() else {
        eprintln!("usage: thaw-js-node [--pkg name=bundle.js] <script.js>");
        std::process::exit(2);
    };
    let source = std::fs::read_to_string(&path).expect("read script");
    let code = std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(move || {
            // the interpreter must stay at one address (async functions hold it); a package owns a boxed one
            let mut package;
            let mut plain;
            let it: &mut Interp = match &pkg {
                Some((name, bundle)) => {
                    let text = std::fs::read_to_string(bundle).expect("read bundle");
                    package = Package::load(&text).unwrap_or_else(|e| {
                        eprintln!("load {name}: {e}");
                        std::process::exit(3)
                    });
                    let exports = package.exports();
                    let it = package.interp();
                    it.install_require(vec![(name.clone(), exports)]);
                    it
                }
                None => {
                    plain = Box::new(Interp::new());
                    &mut plain
                }
            };
            it.configure_stack(256 << 20);
            it.set_stream(true);
            let r = it.run_main(&source);
            it.flush_partial();
            match r {
                Ok(code) => code,
                Err(thaw_js::Ctl::Throw(v)) => {
                    let text = it.to_str(&v);
                    eprintln!("Uncaught {text}");
                    1
                }
                Err(thaw_js::Ctl::Unsupported(f)) => {
                    eprintln!("unsupported: {f}");
                    3
                }
                Err(_) => 1,
            }
        })
        .unwrap()
        .join()
        .unwrap();
    std::process::exit(code);
}
