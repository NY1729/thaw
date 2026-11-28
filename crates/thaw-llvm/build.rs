//! Generates the list of `src/hir_codegen/tests/*_contracts.rs` files for `tests.rs` to include, so a PR adds a
//! test file without touching a shared list (the list used to collide whenever two PRs appended to it).

use std::{env, fs, path::PathBuf};

fn main() {
    let dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("src/hir_codegen/tests");
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)
        .expect("tests directory")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with("_contracts.rs"))
        })
        .collect();
    files.sort();
    let mut out = String::new();
    for path in &files {
        println!("cargo:rerun-if-changed={}", path.display());
        out.push_str(&format!("include!({:?});\n", path.display().to_string()));
    }
    fs::write(
        PathBuf::from(env::var("OUT_DIR").unwrap()).join("contract_tests.rs"),
        out,
    )
    .unwrap();
}
