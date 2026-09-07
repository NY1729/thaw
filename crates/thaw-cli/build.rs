use std::hash::{Hash, Hasher};
use std::path::Path;

fn hash_tree(path: &Path, hasher: &mut impl Hasher) {
    let mut entries = std::fs::read_dir(path)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    entries.sort();
    for entry in entries {
        entry.strip_prefix("../..").unwrap_or(&entry).hash(hasher);
        if entry.is_dir() {
            hash_tree(&entry, hasher);
        } else {
            std::fs::read(entry).unwrap().hash(hasher);
        }
    }
}

fn main() {
    let root = Path::new("../..");
    let inputs = [
        "Cargo.lock",
        "Cargo.toml",
        "crates/thaw-arena",
        "crates/thaw-runtime",
        "crates/thaw-std",
        "crates/thaw-jit",
        "crates/thaw-quickjs",
        "crates/thaw-napi",
    ];
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for input in inputs {
        let path = root.join(input);
        println!("cargo:rerun-if-changed={}", path.display());
        if path.is_dir() {
            hash_tree(&path, &mut hasher);
        } else {
            input.hash(&mut hasher);
            std::fs::read(path).unwrap().hash(&mut hasher);
        }
    }
    for name in ["TARGET", "RUSTC", "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS"] {
        std::env::var(name).unwrap_or_default().hash(&mut hasher);
    }
    println!(
        "cargo:rustc-env=THAW_RUNTIME_FINGERPRINT={:016x}",
        hasher.finish()
    );
}
