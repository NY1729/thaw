use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

// Copied from crates/thaw-cli/build.rs (no shared crate, by design).
fn hash_tree(path: &Path, hasher: &mut Sha256) {
    let mut entries = std::fs::read_dir(path)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    entries.sort();
    for entry in entries {
        hasher.update(entry.to_string_lossy().as_bytes());
        if entry.is_dir() {
            hash_tree(&entry, hasher);
        } else {
            hasher.update(std::fs::read(&entry).unwrap());
        }
    }
}

fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    println!("cargo:rerun-if-changed=exports.txt");
    let exports = std::fs::read_to_string(manifest_dir.join("exports.txt")).unwrap();

    // ABI hash: exports list + ABI version + reserved engine/layout inputs +
    // the source trees of the three runtime crates.
    let mut hasher = Sha256::new();
    hasher.update(exports.as_bytes());
    hasher.update(b"THAW_ABI_VERSION=1\n");
    hasher.update(b"variant=core;engine=none\n");
    hasher.update(b"jsvalue_layout=-\n");
    for dir in ["thaw-arena", "thaw-jit", "thaw-runtime", "thaw-std"] {
        let path = manifest_dir.join("..").join(dir);
        println!("cargo:rerun-if-changed={}", path.display());
        hash_tree(&path, &mut hasher);
    }
    let digest = format!("{:x}", hasher.finalize());
    let hash = &digest[..16];
    std::fs::write(
        out_dir.join("abi_hash.rs"),
        format!(
            "pub static ABI_HASH_CSTR: &[u8] = b\"{hash}\\0\";\n\
             #[used]\n\
             pub static ABI_MARKER: &[u8] = b\"THAW_ABI_HASH={hash};\\0\";\n"
        ),
    )
    .unwrap();
}
