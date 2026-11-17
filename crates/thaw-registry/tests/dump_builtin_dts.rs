//! Writes the ambient `.d.ts` text of every builtin in `tools/parity-aot/modules.txt` into the directory named by
//! `THAW_DUMP_DTS_DIR`, one file per module as `<module with ':' and '/' replaced by '_'>.d.ts` (for example
//! `node_path_posix.d.ts`). The parser that reads these files is `tools/parity-aot/dts_parse.py`.
//!
//! Ignored by default. Run with:
//! `THAW_DUMP_DTS_DIR=<dir> cargo test -p thaw-registry --test dump_builtin_dts -- --ignored`

use std::fs;
use std::path::PathBuf;

#[test]
#[ignore = "writes files; run with THAW_DUMP_DTS_DIR set (see module doc)"]
fn dump_builtin_dts() {
    let dir = PathBuf::from(
        std::env::var("THAW_DUMP_DTS_DIR").expect("THAW_DUMP_DTS_DIR must name a directory"),
    );
    fs::create_dir_all(&dir).expect("create THAW_DUMP_DTS_DIR");
    let modules = include_str!("../../../tools/parity-aot/modules.txt");
    let mut failures = Vec::new();
    let mut written = 0usize;
    for spec in modules.lines().map(str::trim).filter(|l| !l.is_empty()) {
        match thaw_registry::resolve_builtin(spec) {
            Ok(resolved) => {
                let file = dir.join(format!("{}.d.ts", spec.replace([':', '/'], "_")));
                fs::write(&file, resolved.dts_source)
                    .unwrap_or_else(|e| panic!("write {}: {e}", file.display()));
                written += 1;
            }
            Err(e) => failures.push(format!("{spec}: {e}")),
        }
    }
    eprintln!(
        "dump_builtin_dts: wrote {written} files to {}",
        dir.display()
    );
    assert!(
        failures.is_empty(),
        "modules without a builtin declaration:\n{}",
        failures.join("\n")
    );
}
