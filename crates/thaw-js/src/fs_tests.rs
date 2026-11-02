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
