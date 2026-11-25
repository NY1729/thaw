//! Global content-addressed store of finished `add` results.
//!
//! `<cache>/thaw/registry/<pkg>@<version>/<key>/{manifest.json,files/}`. `files/` is the
//! package directory exactly as `add` published it into a project's `thaw_modules/<pkg>`;
//! `manifest.json` carries the key, the `AddedPackage` report and a sha256 per file. Entries
//! are written to a temp dir and renamed into place, and a read re-hashes every file, so a
//! torn, truncated or tampered entry is a miss, never a hit. Materialising always copies
//! (never hardlinks): later registry operations rewrite project files in place, which would
//! corrupt a shared inode.
use super::AddedPackage;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(serde::Serialize, serde::Deserialize)]
struct Manifest {
    key: String,
    added: AddedPackage,
    files: BTreeMap<String, String>,
}

/// `<cache>/thaw/registry`, or `None` when `THAW_NO_REGISTRY_STORE` is set or no cache dir exists.
pub(super) fn store_root() -> Option<PathBuf> {
    if matches!(std::env::var("THAW_NO_REGISTRY_STORE").as_deref(), Ok(v) if !v.is_empty() && v != "0") {
        return None;
    }
    let base = match std::env::var_os("XDG_CACHE_HOME").filter(|v| !v.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(std::env::var_os("HOME").filter(|v| !v.is_empty())?).join(".cache"),
    };
    Some(base.join("thaw").join("registry"))
}

/// Everything that shapes a package's output: format, this build (version + executable
/// size/mtime, so a rebuilt bundler never reads an older build's entries), package, exact
/// version, target platform, and the unverified-prebuild opt-in.
pub(super) fn key(name: &str, version: &str) -> String {
    let exe = std::env::current_exe()
        .and_then(fs::metadata)
        .map(|m| {
            let mtime = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok());
            format!("{}:{}", m.len(), mtime.map_or(0, |d| d.as_nanos()))
        })
        .unwrap_or_default();
    key_from(&[
        "thaw-registry-store-v1",
        env!("CARGO_PKG_VERSION"),
        &exe,
        name,
        version,
        std::env::consts::OS,
        std::env::consts::ARCH,
        &std::env::var("THAW_ALLOW_UNVERIFIED_PREBUILD").unwrap_or_default(),
    ])
}

fn key_from(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update((part.len() as u64).to_le_bytes());
        hasher.update(part.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

fn entry_dir(root: &Path, name: &str, version: &str, key: &str) -> PathBuf {
    root.join(format!("{}@{version}", name.replace('/', "__"))).join(key)
}

/// Relative file paths under `dir`; `None` if anything but plain files/dirs is present.
fn list_files(dir: &Path) -> Option<Vec<String>> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<String>) -> Option<()> {
        for entry in fs::read_dir(dir).ok()? {
            let entry = entry.ok()?;
            let kind = entry.file_type().ok()?;
            if kind.is_dir() {
                walk(base, &entry.path(), out)?;
            } else if kind.is_file() {
                out.push(entry.path().strip_prefix(base).ok()?.to_str()?.to_string());
            } else {
                return None;
            }
        }
        Some(())
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out)?;
    out.sort();
    Some(out)
}

fn hash_file(path: &Path) -> Option<String> {
    Some(format!("{:x}", Sha256::digest(fs::read(path).ok()?)))
}

/// A verified entry: the report and its `files/` directory, or `None` for anything doubtful.
pub(super) fn lookup(root: &Path, name: &str, version: &str, key: &str) -> Option<(AddedPackage, PathBuf)> {
    let entry = entry_dir(root, name, version, key);
    let manifest: Manifest = serde_json::from_slice(&fs::read(entry.join("manifest.json")).ok()?).ok()?;
    let files = entry.join("files");
    if manifest.key != key || list_files(&files)? != manifest.files.keys().cloned().collect::<Vec<_>>() {
        return None;
    }
    for (relative, digest) in &manifest.files {
        if hash_file(&files.join(relative))?.as_str() != digest {
            return None;
        }
    }
    Some((manifest.added, files))
}

/// Copies a verified entry's files into the empty directory `dest`.
pub(super) fn materialize(files: &Path, dest: &Path) -> Result<(), String> {
    for relative in list_files(files).ok_or("store entry changed while reading")? {
        let to = dest.join(&relative);
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("failed to create `{}`: {e}", parent.display()))?;
        }
        fs::copy(files.join(&relative), &to).map_err(|e| format!("failed to copy `{relative}`: {e}"))?;
    }
    Ok(())
}

/// Stores `package_dir` (a freshly published project package). Best effort: `Err` only
/// reports why nothing was stored; callers ignore it.
pub(super) fn publish(
    root: &Path,
    name: &str,
    version: &str,
    key: &str,
    package_dir: &Path,
    added: &AddedPackage,
) -> Result<(), String> {
    let list = list_files(package_dir).ok_or("package contains non-regular files")?;
    let entry = entry_dir(root, name, version, key);
    let parent = entry.parent().ok_or("no parent")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let tmp = parent.join(format!(".tmp-{}-{}", std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
    let result = (|| {
        let files = tmp.join("files");
        fs::create_dir_all(&files).map_err(|e| e.to_string())?;
        materialize(package_dir, &files)?;
        // Hash what was copied, not the project's source, so the manifest describes the entry.
        let mut hashes = BTreeMap::new();
        for relative in &list {
            hashes.insert(relative.clone(), hash_file(&files.join(relative)).ok_or("unreadable copy")?);
        }
        let manifest = Manifest { key: key.to_string(), added: added.clone(), files: hashes };
        fs::write(tmp.join("manifest.json"), serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        for attempt in 0..2 {
            match fs::rename(&tmp, &entry) {
                Ok(()) => return Ok(()),
                // Lost a race or a bad entry sits there: keep a valid one, otherwise move it aside.
                Err(_) if entry.exists() => {
                    if lookup(root, name, version, key).is_some() {
                        return Ok(());
                    }
                    if attempt == 0 {
                        let bad = parent.join(format!(".bad-{}-{}", std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
                        if fs::rename(&entry, &bad).is_ok() {
                            let _ = fs::remove_dir_all(&bad);
                        }
                    }
                }
                Err(e) => return Err(e.to_string()),
            }
        }
        Err("could not replace invalid entry".to_string())
    })();
    let _ = fs::remove_dir_all(&tmp);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn added() -> AddedPackage {
        AddedPackage {
            dts_relative_path: "index.d.ts".into(),
            js_relative_path: "index.js".into(),
            bundled_file_count: 2,
            resolved_version: "1.0.0".into(),
            dependency_versions: BTreeMap::from([("p".into(), "1.0.0".into())]),
            native_addon: None,
            native_diagnostic: None,
        }
    }

    fn package(dir: &Path) -> PathBuf {
        let package = dir.join("project/p");
        fs::create_dir_all(package.join("subpaths/x")).unwrap();
        fs::write(package.join("bundle.js.gz"), b"bundle").unwrap();
        fs::write(package.join("subpaths/x/bundle.js.gz"), b"sub").unwrap();
        package
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("thaw-store-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn key_covers_every_input() {
        let base = key_from(&["a", "b", "c"]);
        assert_ne!(base, key_from(&["a", "b", "d"]));
        assert_ne!(base, key_from(&["ab", "", "c"]), "length-prefixed, so no boundary collisions");
        assert_ne!(key("p", "1.0.0"), key("p", "1.0.1"));
        assert_ne!(key("p", "1.0.0"), key("q", "1.0.0"));
        assert_eq!(key("p", "1.0.0"), key("p", "1.0.0"));
    }

    #[test]
    fn round_trip_is_byte_identical_and_survives_republish() {
        let dir = scratch("round");
        let (root, package) = (dir.join("store"), package(&dir));
        let k = key("p", "1.0.0");
        assert!(lookup(&root, "p", "1.0.0", &k).is_none());
        publish(&root, "p", "1.0.0", &k, &package, &added()).unwrap();
        publish(&root, "p", "1.0.0", &k, &package, &added()).unwrap();
        let (report, files) = lookup(&root, "p", "1.0.0", &k).unwrap();
        assert_eq!(report, added());
        let out = dir.join("out");
        materialize(&files, &out).unwrap();
        assert_eq!(fs::read(out.join("subpaths/x/bundle.js.gz")).unwrap(), b"sub");
        assert_eq!(list_files(&out), list_files(&package));
        assert_eq!(fs::read_dir(root.join("p@1.0.0")).unwrap().count(), 1, "no temp leftovers");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn corruption_is_a_miss_and_is_replaced() {
        let dir = scratch("corrupt");
        let (root, package) = (dir.join("store"), package(&dir));
        let k = key("p", "1.0.0");
        publish(&root, "p", "1.0.0", &k, &package, &added()).unwrap();
        let entry = entry_dir(&root, "p", "1.0.0", &k);
        for damage in 0..4 {
            match damage {
                0 => fs::write(entry.join("files/bundle.js.gz"), b"tampered").unwrap(),
                1 => fs::remove_file(entry.join("files/subpaths/x/bundle.js.gz")).unwrap(),
                2 => fs::write(entry.join("files/extra"), b"x").unwrap(),
                _ => fs::write(entry.join("manifest.json"), b"{").unwrap(),
            }
            assert!(lookup(&root, "p", "1.0.0", &k).is_none(), "damage {damage}");
            publish(&root, "p", "1.0.0", &k, &package, &added()).unwrap();
            assert!(lookup(&root, "p", "1.0.0", &k).is_some(), "repaired {damage}");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn concurrent_publish_leaves_one_valid_entry() {
        let dir = scratch("race");
        let (root, package) = (dir.join("store"), package(&dir));
        let k = key("p", "1.0.0");
        std::thread::scope(|s| {
            for _ in 0..8 {
                s.spawn(|| publish(&root, "p", "1.0.0", &k, &package, &added()).unwrap());
            }
        });
        assert!(lookup(&root, "p", "1.0.0", &k).is_some());
        assert_eq!(fs::read_dir(root.join("p@1.0.0")).unwrap().count(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn real_install_output_is_byte_identical_through_the_store() {
        let dir = scratch("real");
        let package = dir.join("node_modules/leftpad");
        fs::create_dir_all(&package).unwrap();
        fs::write(package.join("package.json"), r#"{"name":"leftpad","version":"1.2.3","main":"index.js","types":"index.d.ts"}"#).unwrap();
        fs::write(package.join("index.js"), "module.exports = (s) => ' ' + s;").unwrap();
        fs::write(package.join("index.d.ts"), "declare function leftpad(s: string): string;\nexport = leftpad;\n").unwrap();
        let first = dir.join("p1");
        let added = crate::add_installed(&first, &dir.join("node_modules"), "leftpad").unwrap();
        let (root, k) = (dir.join("store"), key("leftpad", "1.2.3"));
        publish(&root, "leftpad", "1.2.3", &k, &first.join("leftpad"), &added).unwrap();
        let (report, files) = lookup(&root, "leftpad", "1.2.3", &k).unwrap();
        assert_eq!(report, added);
        let second = dir.join("p2/leftpad");
        materialize(&files, &second).unwrap();
        let names = list_files(&first.join("leftpad")).unwrap();
        assert!(names.contains(&"bundle.js.gz".to_string()) && names.contains(&"instances.json".to_string()));
        assert_eq!(Some(names.clone()), list_files(&second));
        for n in names {
            assert_eq!(fs::read(first.join("leftpad").join(&n)).unwrap(), fs::read(second.join(&n)).unwrap(), "{n}");
        }
        let _ = fs::remove_dir_all(&dir);
    }
}
