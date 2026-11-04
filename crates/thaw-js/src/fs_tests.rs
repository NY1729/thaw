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

// Replays, at host level, the exact op sequence the JS shim issues for
// `open(path,'w') -> writeFile -> stat -> close -> open(path,'r') -> readFile`.
fn fd_call(op: &str, fd: &str, value: &str) -> serde_json::Value {
    serde_json::from_str(&host_fs(op, fd, value, false)).unwrap()
}

#[test]
fn fs_fd_replay_file_handle_sequence() {
    let d = scratch("fd-replay");
    let path = d.join("h.txt");
    let p = path.to_str().unwrap();
    let w = call("fd_open", &path, "w,438");
    eprintln!("replay open w: {w}");
    assert_eq!(w["ok"], true, "open w: {w}");
    let fd = w["fd"].as_u64().unwrap().to_string();
    let wr = fd_call("fd_write_all", &fd, "68656c6c6f");
    eprintln!("replay write_all: {wr}");
    assert_eq!(wr["ok"], true, "write_all: {wr}");
    let st = fd_call("fd_stat", &fd, "");
    eprintln!("replay stat: length={}", st["length"]);
    assert_eq!(st["length"], 5, "stat: {st}");
    let cl = fd_call("fd_close", &fd, "");
    assert_eq!(cl["ok"], true, "close: {cl}");
    let r = call("fd_open", &path, "r,438");
    assert_eq!(r["ok"], true, "open r: {r}");
    let rfd = r["fd"].as_u64().unwrap().to_string();
    let data = fd_call("fd_read_all", &rfd, "");
    eprintln!("replay read_all: {data}");
    assert_eq!(data["data"], "68656c6c6f", "read_all: {data}");
    assert_eq!(fd_call("fd_close", &rfd, "")["ok"], true);
    assert_eq!(std::fs::read(p).unwrap(), b"hello");
    assert_eq!(fd_call("fd_tell", &rfd, "")["code"], "EBADF");
}

#[test]
fn fs_fd_ops_sync_truncate_tell_chmod_utimes_statfs() {
    use std::os::unix::fs::PermissionsExt;
    let d = scratch("fd-ops");
    let path = d.join("o.txt");
    let fd = call("fd_open", &path, "w+,438")["fd"]
        .as_u64()
        .unwrap()
        .to_string();
    assert_eq!(fd_call("fd_write_all", &fd, "68656c6c6f")["ok"], true);
    assert_eq!(fd_call("fd_sync", &fd, "")["ok"], true);
    assert_eq!(fd_call("fd_datasync", &fd, "")["ok"], true);
    assert_eq!(fd_call("fd_tell", &fd, "")["position"], 5);
    assert_eq!(fd_call("fd_truncate", &fd, "2")["ok"], true);
    assert_eq!(fd_call("fd_stat", &fd, "")["length"], 2);
    assert_eq!(fd_call("fd_truncate", &fd, "x")["code"], "EINVAL");
    assert_eq!(fd_call("fd_chmod", &fd, "384")["ok"], true);
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(fd_call("fd_chmod", &fd, "x")["code"], "EINVAL");
    assert_eq!(fd_call("fd_utimes", &fd, "5,6")["ok"], true);
    let mtime = fd_call("fd_stat", &fd, "")["mtimeMs"].as_f64().unwrap();
    assert!((mtime - 6000.0).abs() < 1e-6, "mtimeMs was {mtime}");
    let st = fd_call("fd_statfs", &fd, "");
    assert_eq!(st["ok"], true);
    assert!(st["bsize"].as_u64().unwrap() > 0);
    assert_eq!(fd_call("fd_close", &fd, "")["ok"], true);
    for op in [
        "fd_sync",
        "fd_datasync",
        "fd_tell",
        "fd_truncate",
        "fd_chmod",
        "fd_utimes",
        "fd_statfs",
    ] {
        assert_eq!(fd_call(op, &fd, "1")["code"], "EBADF", "{op} after close");
    }
    assert_eq!(fd_call("fd_sync", "999", "")["code"], "EBADF");
}

// Hex of a plain name, to compare against `nameHex` / `relativeHex`.
fn hex_of(s: &str) -> String {
    s.bytes().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn fd_table_is_shared_across_threads() {
    let d = scratch("fd-threads");
    let path = d.join("t.txt");
    let w = call("fd_open", &path, "w,438");
    assert_eq!(w["ok"], true, "open w: {w}");
    let wfd = w["fd"].as_u64().unwrap().to_string();
    // Write, stat and close on a different thread than the one that opened the fd.
    let (wr, st, cl) = std::thread::spawn(move || {
        (
            fd_call("fd_write_all", &wfd, "68656c6c6f"),
            fd_call("fd_stat", &wfd, ""),
            fd_call("fd_close", &wfd, ""),
        )
    })
    .join()
    .unwrap();
    assert_eq!(wr["ok"], true, "write_all off-thread: {wr}");
    assert_eq!(st["length"], 5, "stat off-thread: {st}");
    assert_eq!(cl["ok"], true, "close off-thread: {cl}");
    // Reopen on another thread, read back on this one.
    let p = path.clone();
    let r = std::thread::spawn(move || call("fd_open", &p, "r,438"))
        .join()
        .unwrap();
    assert_eq!(r["ok"], true, "open r off-thread: {r}");
    let rfd = r["fd"].as_u64().unwrap().to_string();
    let data = fd_call("fd_read_all", &rfd, "");
    assert_eq!(
        data["data"], "68656c6c6f",
        "read_all on main thread: {data}"
    );
    assert_eq!(fd_call("fd_close", &rfd, "")["ok"], true);
    assert_eq!(fd_call("fd_tell", &rfd, "")["code"], "EBADF");
}

#[test]
fn fs_readdir_entries_sorted_by_name_bytes() {
    let d = scratch("readdir-sort");
    // Created out of order; expected order is byte order: "10" < "9" < "B" < "a" < "z" < "é".
    for name in ["é", "a", "B", "9", "10"] {
        std::fs::write(d.join(name), b"x").unwrap();
    }
    let names = |v: &serde_json::Value| -> Vec<String> {
        v["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["nameHex"].as_str().unwrap().to_string())
            .collect()
    };
    let expected: Vec<String> = ["10", "9", "B", "a", "é"]
        .iter()
        .map(|s| hex_of(s))
        .collect();
    assert_eq!(names(&call("readdir", &d, "")), expected);

    // Recursive: each directory is sorted, and a sub-directory's children follow it in place.
    std::fs::create_dir(d.join("z")).unwrap();
    std::fs::write(d.join("z").join("b"), b"x").unwrap();
    std::fs::write(d.join("z").join("a"), b"x").unwrap();
    let rec = call_recursive("readdir", &d, "");
    let rel: Vec<String> = rec["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["relativeHex"].as_str().unwrap().to_string())
        .collect();
    let expected_rel: Vec<String> = ["10", "9", "B", "a", "z", "z/a", "z/b", "é"]
        .iter()
        .map(|s| hex_of(s))
        .collect();
    assert_eq!(rel, expected_rel);
}
