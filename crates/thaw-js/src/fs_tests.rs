// Direct `host_fs` checks for the path operations (chmod, symlink, link, truncate, utimes,
// lutimes, statfs). Each test works in its own scratch directory.

use crate::sysio::host_fs;
use std::path::{Path, PathBuf};

fn scratch(name: &str) -> PathBuf {
    let base = std::env::var_os("THAW_JS_TEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../target/js-fs-tests"
            ))
        });
    let d = base.join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn call(op: &str, path: &Path, value: &str) -> serde_json::Value {
    serde_json::from_str(&host_fs(op, path.to_str().unwrap(), value, false)).unwrap()
}

#[test]
fn fs_path_ops_chmod() {
    use std::os::unix::fs::PermissionsExt;
    let d = scratch("chmod");
    let f = d.join("f.txt");
    std::fs::write(&f, b"x").unwrap();
    assert_eq!(call("chmod", &f, "416")["ok"], true);
    assert_eq!(
        std::fs::metadata(&f).unwrap().permissions().mode() & 0o777,
        0o640
    );
    assert_eq!(call("chmod", &f, "99999")["code"], "EINVAL");
    assert_eq!(call("chmod", &d.join("nope"), "416")["code"], "ENOENT");
}

#[test]
fn fs_path_ops_symlink() {
    let d = scratch("symlink");
    std::fs::write(d.join("target.txt"), b"hi").unwrap();
    let link = d.join("link");
    let link_str = link.to_str().unwrap();
    assert_eq!(
        call("symlink", Path::new("target.txt"), link_str)["ok"],
        true
    );
    assert_eq!(call("lstat", &link, "")["symlink"], true);
    assert_eq!(call("readlink", &link, "")["path"], "target.txt");
    assert_eq!(call("read", &link, "")["data"], "6869");
    assert_eq!(
        call("symlink", Path::new("target.txt"), link_str)["code"],
        "EEXIST"
    );
}

#[test]
fn fs_path_ops_link() {
    let d = scratch("link");
    let a = d.join("a.txt");
    let b = d.join("b.txt");
    std::fs::write(&a, b"x").unwrap();
    assert_eq!(call("link", &a, b.to_str().unwrap())["ok"], true);
    assert_eq!(call("stat", &a, "")["nlink"], 2);
    assert_eq!(call("stat", &b, "")["nlink"], 2);
    assert_eq!(call("link", &a, b.to_str().unwrap())["code"], "EEXIST");
}

#[test]
fn fs_path_ops_truncate() {
    let d = scratch("truncate");
    let f = d.join("t.txt");
    std::fs::write(&f, b"hello world").unwrap();
    assert_eq!(call("truncate", &f, "2")["ok"], true);
    assert_eq!(call("stat", &f, "")["length"], 2);
    assert_eq!(call("truncate", &f, "10")["ok"], true);
    assert_eq!(call("stat", &f, "")["length"], 10);
    assert_eq!(call("truncate", &d.join("missing"), "1")["code"], "ENOENT");
    assert_eq!(call("truncate", &f, "x")["code"], "EINVAL");
}

#[test]
fn fs_path_ops_utimes() {
    let d = scratch("utimes");
    let f = d.join("u.txt");
    std::fs::write(&f, b"x").unwrap();
    assert_eq!(call("utimes", &f, "1000,2000")["ok"], true);
    let mtime = call("stat", &f, "")["mtimeMs"].as_f64().unwrap();
    assert!((mtime - 2_000_000.0).abs() < 1e-6, "mtimeMs was {mtime}");
    assert_eq!(call("utimes", &f, "x,1")["code"], "EINVAL");
}

#[test]
fn fs_path_ops_lutimes_changes_link_not_target() {
    let d = scratch("lutimes");
    let target = d.join("target.txt");
    std::fs::write(&target, b"hi").unwrap();
    let link = d.join("link");
    std::os::unix::fs::symlink("target.txt", &link).unwrap();
    assert_eq!(call("lutimes", &link, "1000,2000")["ok"], true);
    let link_mtime = call("lstat", &link, "")["mtimeMs"].as_f64().unwrap();
    assert!(
        (link_mtime - 2_000_000.0).abs() < 1e-6,
        "link mtimeMs was {link_mtime}"
    );
    let target_mtime = call("stat", &target, "")["mtimeMs"].as_f64().unwrap();
    assert!(
        (target_mtime - 2_000_000.0).abs() > 1.0,
        "target mtimeMs was {target_mtime}"
    );
}

#[test]
fn fs_path_ops_statfs() {
    let d = scratch("statfs");
    let v = call("statfs", &d, "");
    assert_eq!(v["ok"], true);
    assert!(v["blocks"].as_u64().unwrap() > 0);
    assert!(v["bsize"].as_u64().unwrap() > 0);
    assert!(
        v["exact"]["bsize"]
            .as_str()
            .unwrap()
            .parse::<u64>()
            .unwrap()
            > 0
    );
}

fn call_recursive(op: &str, path: &Path, value: &str) -> serde_json::Value {
    serde_json::from_str(&host_fs(op, path.to_str().unwrap(), value, true)).unwrap()
}

#[test]
fn fs_mkdtemp_makes_private_unique_dirs() {
    use std::os::unix::fs::PermissionsExt;
    let d = scratch("mkdtemp");
    let prefix = d.join("x-");
    let a = call("mkdtemp", &prefix, "");
    let b = call("mkdtemp", &prefix, "");
    assert_eq!(a["ok"], true);
    assert_eq!(b["ok"], true);
    let (pa, pb) = (a["path"].as_str().unwrap(), b["path"].as_str().unwrap());
    assert_ne!(pa, pb);
    for p in [pa, pb] {
        let name = Path::new(p).file_name().unwrap().to_str().unwrap();
        let suffix = name.strip_prefix("x-").unwrap();
        assert_eq!(suffix.len(), 6);
        assert!(suffix.chars().all(|c| c.is_ascii_alphanumeric()));
        assert!(Path::new(p).is_dir());
        assert_eq!(
            std::fs::metadata(p).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
    assert_eq!(
        call("mkdtemp", &d.join("missing").join("x-"), "")["code"],
        "ENOENT"
    );
}

#[test]
fn fs_copy_excl_copies_mode_and_refuses_existing() {
    use std::os::unix::fs::PermissionsExt;
    let d = scratch("copy-excl");
    let src = d.join("src.txt");
    let dst = d.join("dst.txt");
    std::fs::write(&src, b"hello").unwrap();
    std::fs::set_permissions(&src, std::fs::Permissions::from_mode(0o640)).unwrap();
    let r = call("copy_excl", &src, dst.to_str().unwrap());
    assert_eq!(r["ok"], true);
    assert_eq!(r["length"], 5);
    assert_eq!(std::fs::read(&dst).unwrap(), b"hello");
    assert_eq!(
        std::fs::metadata(&dst).unwrap().permissions().mode() & 0o777,
        0o640
    );
    assert_eq!(
        call("copy_excl", &src, dst.to_str().unwrap())["code"],
        "EEXIST"
    );
}

#[test]
fn fs_copy_guard_flags_self_and_descendant_copies() {
    let d = scratch("copy-guard");
    let src = d.join("src");
    std::fs::create_dir_all(src.join("sub")).unwrap();
    let file = d.join("f.txt");
    std::fs::write(&file, b"x").unwrap();
    let sub = src.join("sub");
    let fresh = d.join("new.txt");
    assert_eq!(
        call("copy_guard", &src, sub.to_str().unwrap())["unsafe"],
        true
    );
    assert_eq!(
        call("copy_guard", &file, file.to_str().unwrap())["unsafe"],
        true
    );
    assert_eq!(
        call("copy_guard", &file, fresh.to_str().unwrap())["unsafe"],
        false
    );
    assert_eq!(
        call("copy_guard", &d.join("missing"), fresh.to_str().unwrap())["code"],
        "ENOENT"
    );
}

#[test]
fn fs_cp_recursive_and_single_file() {
    let d = scratch("cp");
    let src = d.join("tree");
    std::fs::create_dir_all(src.join("nested")).unwrap();
    std::fs::write(src.join("nested").join("a.txt"), b"alpha").unwrap();
    std::fs::write(src.join("b.txt"), b"beta").unwrap();
    std::os::unix::fs::symlink("b.txt", src.join("link")).unwrap();
    let dst = d.join("copy");
    let r = call_recursive("cp", &src, dst.to_str().unwrap());
    assert_eq!(r["ok"], true);
    assert_eq!(r["length"], 13);
    assert_eq!(
        std::fs::read(dst.join("nested").join("a.txt")).unwrap(),
        b"alpha"
    );
    assert_eq!(std::fs::read(dst.join("b.txt")).unwrap(), b"beta");
    // ponytail: symlinks are followed (copied as file contents), not recreated as links.
    assert_eq!(std::fs::read(dst.join("link")).unwrap(), b"beta");
    assert_eq!(
        call_recursive("cp", &src, src.join("inner").to_str().unwrap())["code"],
        "EINVAL"
    );
    let single = d.join("single.txt");
    let s = call("cp", &src.join("b.txt"), single.to_str().unwrap());
    assert_eq!(s["length"], 4);
    assert_eq!(std::fs::read(&single).unwrap(), b"beta");
}
