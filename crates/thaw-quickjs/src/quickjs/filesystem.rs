fn fs_error(operation: &str, path: &str, error: io::Error) -> String {
    let code = match error.kind() {
        io::ErrorKind::NotFound => "ENOENT",
        io::ErrorKind::PermissionDenied => "EACCES",
        io::ErrorKind::AlreadyExists => "EEXIST",
        io::ErrorKind::InvalidInput => "EINVAL",
        io::ErrorKind::IsADirectory => "EISDIR",
        io::ErrorKind::NotADirectory => "ENOTDIR",
        _ => "EIO",
    };
    serde_json::json!({ "ok": false, "code": code, "operation": operation, "path": path, "message": error.to_string() }).to_string()
}

#[cfg(unix)]
fn fs_symlink(target: &str, link: &str) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(not(unix))]
fn fs_symlink(_target: &str, _link: &str) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "symbolic links are unsupported",
    ))
}

#[cfg(unix)]
fn fs_chmod(path: &str, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn fs_chmod(_path: &str, _mode: u32) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "chmod is unsupported",
    ))
}

#[cfg(unix)]
fn fs_chown(path: &str, value: &str, follow: bool) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let mut values = value.split(',');
    let uid = values
        .next()
        .unwrap_or("0")
        .parse::<libc::uid_t>()
        .unwrap_or(0);
    let gid = values
        .next()
        .unwrap_or("0")
        .parse::<libc::gid_t>()
        .unwrap_or(0);
    let path = CString::new(std::ffi::OsStr::new(path).as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains a NUL byte"))?;
    let result = unsafe {
        if follow {
            libc::chown(path.as_ptr(), uid, gid)
        } else {
            libc::lchown(path.as_ptr(), uid, gid)
        }
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(not(unix))]
fn fs_chown(_path: &str, _value: &str, _follow: bool) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "ownership changes are unsupported",
    ))
}

#[cfg(unix)]
fn fs_access(path: &str, mode: i32) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let path = CString::new(std::ffi::OsStr::new(path).as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains a NUL byte"))?;
    if unsafe { libc::access(path.as_ptr(), mode) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(unix)]
fn fs_statfs(path: &str) -> io::Result<serde_json::Value> {
    use std::os::unix::ffi::OsStrExt;
    let path = CString::new(std::ffi::OsStr::new(path).as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains a NUL byte"))?;
    let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let stats = unsafe { stats.assume_init() };
    Ok(
        serde_json::json!({ "ok": true, "type": 0, "bsize": stats.f_bsize, "blocks": stats.f_blocks, "bfree": stats.f_bfree, "bavail": stats.f_bavail, "files": stats.f_files, "ffree": stats.f_ffree }),
    )
}

#[cfg(not(unix))]
fn fs_statfs(_path: &str) -> io::Result<serde_json::Value> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "filesystem statistics are unsupported",
    ))
}

#[cfg(not(unix))]
fn fs_access(path: &str, mode: i32) -> io::Result<()> {
    let metadata = std::fs::metadata(path)?;
    if mode & 2 != 0 && metadata.permissions().readonly() {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "path is read-only",
        ))
    } else {
        Ok(())
    }
}

fn fs_copy_recursive(source: &std::path::Path, destination: &std::path::Path) -> io::Result<u64> {
    let metadata = std::fs::symlink_metadata(source)?;
    if metadata.is_dir() {
        std::fs::create_dir_all(destination)?;
        let mut copied = 0;
        for entry in std::fs::read_dir(source)? {
            let entry = entry?;
            copied += fs_copy_recursive(&entry.path(), &destination.join(entry.file_name()))?;
        }
        Ok(copied)
    } else {
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(source, destination)
    }
}

#[cfg(unix)]
fn fs_write_with_mode(path: &str, value: &str, append: bool) -> io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let (mode, encoded) = value.split_once(',').unwrap_or(("438", value));
    let mut options = std::fs::OpenOptions::new();
    options
        .create(true)
        .write(true)
        .append(append)
        .truncate(!append)
        .mode(mode.parse::<u32>().unwrap_or(0o666));
    options.open(path)?.write_all(&hex_decode(encoded))
}

#[cfg(not(unix))]
fn fs_write_with_mode(path: &str, value: &str, append: bool) -> io::Result<()> {
    let (_, encoded) = value.split_once(',').unwrap_or(("438", value));
    let mut options = std::fs::OpenOptions::new();
    options
        .create(true)
        .write(true)
        .append(append)
        .truncate(!append);
    options.open(path)?.write_all(&hex_decode(encoded))
}

#[cfg(unix)]
fn fs_mkdir_with_mode(path: &str, value: &str, recursive: bool) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    let mut builder = std::fs::DirBuilder::new();
    builder
        .recursive(recursive)
        .mode(value.parse::<u32>().unwrap_or(0o777))
        .create(path)
}

#[cfg(not(unix))]
fn fs_mkdir_with_mode(path: &str, _value: &str, recursive: bool) -> io::Result<()> {
    std::fs::DirBuilder::new().recursive(recursive).create(path)
}

fn system_time_millis(time: io::Result<std::time::SystemTime>) -> f64 {
    time.ok()
        .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

#[cfg(unix)]
fn fs_metadata_record(metadata: std::fs::Metadata) -> serde_json::Value {
    use std::os::unix::fs::MetadataExt;
    let ctime = metadata.ctime() as f64 * 1000.0 + metadata.ctime_nsec() as f64 / 1_000_000.0;
    serde_json::json!({ "ok": true, "length": metadata.len(), "file": metadata.is_file(), "directory": metadata.is_dir(), "symlink": metadata.file_type().is_symlink(), "readonly": metadata.permissions().readonly(), "dev": metadata.dev(), "ino": metadata.ino(), "mode": metadata.mode(), "nlink": metadata.nlink(), "uid": metadata.uid(), "gid": metadata.gid(), "rdev": metadata.rdev(), "blksize": metadata.blksize(), "blocks": metadata.blocks(), "atimeMs": system_time_millis(metadata.accessed()), "mtimeMs": system_time_millis(metadata.modified()), "ctimeMs": ctime, "birthtimeMs": system_time_millis(metadata.created()) })
}

#[cfg(not(unix))]
fn fs_metadata_record(metadata: std::fs::Metadata) -> serde_json::Value {
    let modified = system_time_millis(metadata.modified());
    serde_json::json!({ "ok": true, "length": metadata.len(), "file": metadata.is_file(), "directory": metadata.is_dir(), "symlink": metadata.file_type().is_symlink(), "readonly": metadata.permissions().readonly(), "dev": 0, "ino": 0, "mode": if metadata.is_dir() { 16877 } else { 33188 }, "nlink": 1, "uid": 0, "gid": 0, "rdev": 0, "blksize": 0, "blocks": 0, "atimeMs": system_time_millis(metadata.accessed()), "mtimeMs": modified, "ctimeMs": modified, "birthtimeMs": system_time_millis(metadata.created()) })
}

fn fs_utimes(path: &str, value: &str) -> io::Result<()> {
    let mut values = value.split(',');
    let accessed = values.next().unwrap_or("0").parse::<f64>().unwrap_or(0.0);
    let modified = values.next().unwrap_or("0").parse::<f64>().unwrap_or(0.0);
    let times = std::fs::FileTimes::new()
        .set_accessed(std::time::UNIX_EPOCH + Duration::from_secs_f64(accessed.max(0.0)))
        .set_modified(std::time::UNIX_EPOCH + Duration::from_secs_f64(modified.max(0.0)));
    std::fs::OpenOptions::new()
        .read(true)
        .open(path)?
        .set_times(times)
}

#[cfg(unix)]
fn fs_lutimes(path: &str, value: &str) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let mut values = value.split(',');
    let to_timespec = |value: Option<&str>| {
        let seconds = value.unwrap_or("0").parse::<f64>().unwrap_or(0.0).max(0.0);
        libc::timespec {
            tv_sec: seconds.trunc() as libc::time_t,
            tv_nsec: (seconds.fract() * 1_000_000_000.0).round() as libc::c_long,
        }
    };
    let times = [to_timespec(values.next()), to_timespec(values.next())];
    let path = CString::new(std::ffi::OsStr::new(path).as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains a NUL byte"))?;
    if unsafe {
        libc::utimensat(
            libc::AT_FDCWD,
            path.as_ptr(),
            times.as_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } == 0
    {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(not(unix))]
fn fs_lutimes(_path: &str, _value: &str) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "symbolic-link timestamps are unsupported",
    ))
}

fn host_fs(operation: String, path: String, value: String, recursive: bool) -> String {
    let result = match operation.as_str() {
        "exists" => return serde_json::json!({ "ok": true, "exists": std::path::Path::new(&path).exists() }).to_string(),
        "access" => fs_access(&path, value.parse::<i32>().unwrap_or(0)).map(|_| serde_json::json!({ "ok": true })),
        "statfs" => fs_statfs(&path),
        "read" => std::fs::read(&path).map(|bytes| serde_json::json!({ "ok": true, "data": hex_encode(&bytes) })),
        "read_range" => (|| -> io::Result<serde_json::Value> {
            let (position, length) = value.split_once(',').unwrap_or(("0", "0"));
            let mut file = std::fs::File::open(&path)?;
            file.seek(SeekFrom::Start(position.parse::<u64>().unwrap_or(0)))?;
            let mut bytes = vec![0; length.parse::<usize>().unwrap_or(0)];
            let count = file.read(&mut bytes)?;
            bytes.truncate(count);
            Ok(serde_json::json!({ "ok": true, "data": hex_encode(&bytes), "length": count }))
        })(),
        "write" => std::fs::write(&path, hex_decode(&value)).map(|_| serde_json::json!({ "ok": true })),
        "write_mode" => fs_write_with_mode(&path, &value, false).map(|_| serde_json::json!({ "ok": true })),
        "append_mode" => fs_write_with_mode(&path, &value, true).map(|_| serde_json::json!({ "ok": true })),
        "write_range" => (|| -> io::Result<serde_json::Value> {
            let (position, encoded) = value.split_once(':').unwrap_or(("0", ""));
            let bytes = hex_decode(encoded);
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .write(true)
                .open(&path)?;
            file.seek(SeekFrom::Start(position.parse::<u64>().unwrap_or(0)))?;
            file.write_all(&bytes)?;
            Ok(serde_json::json!({ "ok": true, "length": bytes.len() }))
        })(),
        "append" => std::fs::OpenOptions::new().create(true).append(true).open(&path).and_then(|mut file| file.write_all(&hex_decode(&value))).map(|_| serde_json::json!({ "ok": true })),
        "mkdir" => if recursive { std::fs::create_dir_all(&path) } else { std::fs::create_dir(&path) }.map(|_| serde_json::json!({ "ok": true })),
        "mkdir_mode" => fs_mkdir_with_mode(&path, &value, recursive).map(|_| serde_json::json!({ "ok": true })),
        "readdir" => std::fs::read_dir(&path).and_then(|entries| entries.map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned())).collect::<io::Result<Vec<_>>>()).map(|entries| serde_json::json!({ "ok": true, "entries": entries })),
        "stat" => std::fs::metadata(&path).map(fs_metadata_record),
        "lstat" => std::fs::symlink_metadata(&path).map(fs_metadata_record),
        "unlink" => std::fs::remove_file(&path).map(|_| serde_json::json!({ "ok": true })),
        "rmdir" => if recursive { std::fs::remove_dir_all(&path) } else { std::fs::remove_dir(&path) }.map(|_| serde_json::json!({ "ok": true })),
        "rename" => std::fs::rename(&path, &value).map(|_| serde_json::json!({ "ok": true })),
        "copy" => std::fs::copy(&path, &value).map(|bytes| serde_json::json!({ "ok": true, "length": bytes })),
        "cp" => if recursive { fs_copy_recursive(std::path::Path::new(&path), std::path::Path::new(&value)) } else { std::fs::copy(&path, &value) }.map(|bytes| serde_json::json!({ "ok": true, "length": bytes })),
        "realpath" => std::fs::canonicalize(&path).map(|resolved| serde_json::json!({ "ok": true, "path": resolved.to_string_lossy() })),
        "mkdtemp" => (|| -> io::Result<serde_json::Value> { let mut random = [0u8; 6]; getrandom::getrandom(&mut random).map_err(|error| io::Error::other(error.to_string()))?; let created = format!("{}{}", path, hex_encode(&random)); std::fs::create_dir(&created)?; Ok(serde_json::json!({ "ok": true, "path": created })) })(),
        "truncate" => std::fs::OpenOptions::new().write(true).open(&path).and_then(|file| file.set_len(value.parse::<u64>().unwrap_or(0))).map(|_| serde_json::json!({ "ok": true })),
        "link" => std::fs::hard_link(&path, &value).map(|_| serde_json::json!({ "ok": true })),
        "symlink" => fs_symlink(&path, &value).map(|_| serde_json::json!({ "ok": true })),
        "readlink" => std::fs::read_link(&path).map(|target| serde_json::json!({ "ok": true, "path": target.to_string_lossy() })),
        "chmod" => fs_chmod(&path, value.parse::<u32>().unwrap_or(0)).map(|_| serde_json::json!({ "ok": true })),
        "utimes" => fs_utimes(&path, &value).map(|_| serde_json::json!({ "ok": true })),
        "lutimes" => fs_lutimes(&path, &value).map(|_| serde_json::json!({ "ok": true })),
        "chown" => fs_chown(&path, &value, true).map(|_| serde_json::json!({ "ok": true })),
        "lchown" => fs_chown(&path, &value, false).map(|_| serde_json::json!({ "ok": true })),
        _ => Err(io::Error::new(io::ErrorKind::InvalidInput, "unknown filesystem operation")),
    };
    result
        .map(|value| value.to_string())
        .unwrap_or_else(|error| fs_error(&operation, &path, error))
}

thread_local! {
    static JS: RefCell<Option<(Runtime, Context)>> = const { RefCell::new(None) };
    static NET_STREAMS: RefCell<(u32, HashMap<u32, TcpStream>)> = RefCell::new((1, HashMap::new()));
    static NET_LISTENERS: RefCell<(u32, HashMap<u32, TcpListener>)> = RefCell::new((1, HashMap::new()));
    static UDP_SOCKETS: RefCell<(u32, HashMap<u32, UdpSocket>)> = RefCell::new((1, HashMap::new()));
    #[cfg(feature = "tls")]
    static TLS_STREAMS: RefCell<TlsStreamTable> = RefCell::new((1, HashMap::new()));
    #[cfg(feature = "tls")]
    static TLS_SERVER_STREAMS: RefCell<TlsServerStreamTable> = RefCell::new((1, HashMap::new()));
    #[cfg(feature = "tls")]
    static TLS_LISTENERS: RefCell<(u32, HashMap<u32, TlsListener>)> = RefCell::new((1, HashMap::new()));
    #[cfg(feature = "tls")]
    static TLS_CLIENT_CERTIFICATES: RefCell<HashMap<u32, TlsCertificates>> = RefCell::new(HashMap::new());
    #[cfg(feature = "tls")]
    static TLS_SERVER_CERTIFICATES: RefCell<HashMap<u32, TlsCertificates>> = RefCell::new(HashMap::new());
    static HOST_WORKERS: RefCell<HostWorkerTable> = RefCell::new(HostWorkerTable { next_handle: 1, workers: HashMap::new(), shared_env: Arc::new(Mutex::new(HashMap::new())) });
    static HOST_CHILDREN: RefCell<HostChildTable> = RefCell::new(HostChildTable { next_handle: 1, children: HashMap::new() });
    #[cfg(feature = "wasm")]
    static WASM: RefCell<WasmTable> = RefCell::new(WasmTable::default());
    #[cfg(feature = "wasm")]
    static WASM_JS_IMPORTS: RefCell<(u32, HashMap<u32, WasmJsImport>)> = RefCell::new((1, HashMap::new()));
    #[cfg(feature = "wasm")]
    static WASM_JS_VALUES: RefCell<(u32, HashMap<u32, WasmJsValue>)> = RefCell::new((1, HashMap::new()));
}
