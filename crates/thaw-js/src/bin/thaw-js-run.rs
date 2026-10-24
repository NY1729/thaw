//! `thaw-js-run <bundle.js> <path> [args-json]`: load a registry bundle on thaw-js, call
//! `path` with a JSON array of arguments and print the JSON result (or `ERROR: ...`).
use thaw_js::host::{Native, Package};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        eprintln!("usage: thaw-js-run <bundle.js> <path> [args-json]");
        std::process::exit(2);
    }
    let source = std::fs::read_to_string(&args[0]).expect("read bundle");
    let call_args = match Native::from_json(args.get(2).map_or("[]", String::as_str)) {
        Ok(Native::Array(a)) => Some(a),
        Ok(Native::Null) => None, // read the property instead of calling it
        _ => {
            eprintln!("args must be a JSON array");
            std::process::exit(2);
        }
    };
    // deep recursion in the tree-walker needs a big stack
    let handle = std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(move || {
            // THAW_JS_REPEAT=N: time N extra calls (benchmarking aid)
            if let (Ok(n), Ok(mut p)) = (
                std::env::var("THAW_JS_REPEAT").map(|v| v.parse::<usize>().unwrap_or(0)),
                Package::load(&source),
            ) {
                let start = std::time::Instant::now();
                for _ in 0..n {
                    let _ = p.call(&args[1], call_args.as_deref().unwrap_or(&[]));
                }
                eprintln!(
                    "{n} calls in {:?} ({:?}/call)",
                    start.elapsed(),
                    start.elapsed() / n.max(1) as u32
                );
            }
            match Package::load(&source) {
                Ok(mut p) => match match &call_args {
                    Some(a) => p.call(&args[1], a),
                    None => p.get(&args[1]),
                } {
                    Ok(v) => println!("{}", v.to_json()),
                    Err(e) => println!("ERROR: {e}"),
                },
                Err(e) => println!("ERROR: {e}"),
            }
        })
        .unwrap();
    handle.join().unwrap();
}
