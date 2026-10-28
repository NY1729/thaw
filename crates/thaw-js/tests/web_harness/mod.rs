//! Shared harness for the web-global differential tests (`tests/web_<name>.rs`).
//!
//! Each `<dir>/<case>.js` runs on a fresh interpreter after the port sources are evaluated; its
//! console output must equal `<case>.out`, recorded from Node v22 (`node <case>.js > <case>.out`).

use thaw_js::{Ctl, Interp, Value};

fn run_case(ports: &[&str], src: &str) -> Result<Vec<String>, String> {
    let mut it = Interp::new();
    for port in ports {
        match it.run(port) {
            Ok(_) => {}
            Err(Ctl::Throw(v)) => return Err(format!("port threw {}", it.to_str(&v))),
            Err(Ctl::Unsupported(f)) => return Err(format!("port unsupported: {f}")),
            Err(_) => return Err("port stray completion".into()),
        }
    }
    let ran = it
        .run(src)
        .and_then(|_| it.run_event_loop().map(|_| Value::Undefined));
    match ran {
        Ok(_) => Ok(it.out.clone()),
        Err(Ctl::Throw(v)) => Err(format!("uncaught {}", it.to_str(&v))),
        Err(Ctl::Unsupported(f)) => Err(format!("unsupported: {f}")),
        Err(_) => Err("stray completion".into()),
    }
}

pub fn run_dir(dir: &str, ports: &[&str]) {
    let mut cases: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok()?.path().to_str().map(str::to_string))
        .filter(|p| p.ends_with(".js"))
        .collect();
    cases.sort();
    assert!(!cases.is_empty(), "no cases in {dir}");
    let mut failures = Vec::new();
    for js in &cases {
        let base = js.trim_end_matches(".js");
        let want = std::fs::read_to_string(format!("{base}.out")).unwrap();
        let src = std::fs::read_to_string(js).unwrap();
        // the interpreter is recursive: give it a large stack
        let got = std::thread::scope(|s| {
            std::thread::Builder::new()
                .stack_size(64 << 20)
                .spawn_scoped(s, || run_case(ports, &src))
                .unwrap()
                .join()
                .unwrap()
        });
        match got {
            Ok(lines) => {
                let got: String = lines.iter().map(|l| format!("{l}\n")).collect();
                if got != want {
                    failures.push(format!("{js}\n--- node\n{want}--- thaw-js\n{got}"));
                }
            }
            Err(e) => failures.push(format!("{js}: {e}")),
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} differ:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n=====\n")
    );
}
