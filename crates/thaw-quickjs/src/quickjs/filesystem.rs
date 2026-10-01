#[cfg(unix)]
fn fs_raw_error_code(raw: i32) -> Option<&'static str> {
    if raw == libc::ENOTSUP || raw == libc::EOPNOTSUPP { return Some("ENOTSUP"); }
    Some(match raw {
        libc::EPERM => "EPERM",
        libc::ENOENT => "ENOENT",
        libc::ESRCH => "ESRCH",
        libc::EINTR => "EINTR",
        libc::EIO => "EIO",
        libc::ENXIO => "ENXIO",
        libc::E2BIG => "E2BIG",
        libc::ENOEXEC => "ENOEXEC",
        libc::EBADF => "EBADF",
        libc::ECHILD => "ECHILD",
        libc::EAGAIN => "EAGAIN",
        libc::ENOMEM => "ENOMEM",
        libc::EACCES => "EACCES",
        libc::EFAULT => "EFAULT",
        libc::EBUSY => "EBUSY",
        libc::EEXIST => "EEXIST",
        libc::EXDEV => "EXDEV",
        libc::ENODEV => "ENODEV",
        libc::ENOTDIR => "ENOTDIR",
        libc::EISDIR => "EISDIR",
        libc::EINVAL => "EINVAL",
        libc::ENFILE => "ENFILE",
        libc::EMFILE => "EMFILE",
        libc::ENOTTY => "ENOTTY",
        libc::ETXTBSY => "ETXTBSY",
        libc::EFBIG => "EFBIG",
        libc::ENOSPC => "ENOSPC",
        libc::ESPIPE => "ESPIPE",
        libc::EROFS => "EROFS",
        libc::EMLINK => "EMLINK",
        libc::EPIPE => "EPIPE",
        libc::EDOM => "EDOM",
        libc::ERANGE => "ERANGE",
        libc::EDEADLK => "EDEADLK",
        libc::ENAMETOOLONG => "ENAMETOOLONG",
        libc::ENOLCK => "ENOLCK",
        libc::ENOSYS => "ENOSYS",
        libc::ENOTEMPTY => "ENOTEMPTY",
        libc::ELOOP => "ELOOP",
        libc::EOVERFLOW => "EOVERFLOW",
        libc::EILSEQ => "EILSEQ",
        libc::EADDRINUSE => "EADDRINUSE",
        libc::EADDRNOTAVAIL => "EADDRNOTAVAIL",
        libc::ENETDOWN => "ENETDOWN",
        libc::ENETUNREACH => "ENETUNREACH",
        libc::ECONNABORTED => "ECONNABORTED",
        libc::ECONNRESET => "ECONNRESET",
        libc::ENOBUFS => "ENOBUFS",
        libc::ENOTCONN => "ENOTCONN",
        libc::ETIMEDOUT => "ETIMEDOUT",
        libc::ECONNREFUSED => "ECONNREFUSED",
        libc::EHOSTUNREACH => "EHOSTUNREACH",
        libc::ESTALE => "ESTALE",
        libc::EDQUOT => "EDQUOT",
        libc::ECANCELED => "ECANCELED",
        _ => return None,
    })
}

#[cfg(not(unix))]
fn fs_raw_error_code(_raw: i32) -> Option<&'static str> {
    None
}

fn fs_kind_error_code(kind: io::ErrorKind) -> Option<&'static str> {
    Some(match kind {
        io::ErrorKind::NotFound => "ENOENT",
        io::ErrorKind::PermissionDenied => "EACCES",
        io::ErrorKind::AlreadyExists => "EEXIST",
        io::ErrorKind::InvalidInput => "EINVAL",
        io::ErrorKind::IsADirectory => "EISDIR",
        io::ErrorKind::NotADirectory => "ENOTDIR",
        _ => return None,
    })
}

fn fs_error(operation: &str, path: &str, error: io::Error) -> String {
    let code = if let Some(raw) = error.raw_os_error() {
        fs_raw_error_code(raw)
            .or_else(|| if cfg!(unix) { None } else { fs_kind_error_code(error.kind()) })
            .map(str::to_owned)
            .unwrap_or_else(|| format!("ERRNO_{raw}"))
    } else {
        fs_kind_error_code(error.kind()).unwrap_or("EIO").to_string()
    };
    serde_json::json!({ "ok": false, "code": code, "operation": operation, "path": path, "message": error.to_string() }).to_string()
}

#[cfg(unix)]
fn fs_symlink(target: &std::path::Path, link: &std::path::Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(not(unix))]
fn fs_symlink(_target: &std::path::Path, _link: &std::path::Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "symbolic links are unsupported",
    ))
}

#[cfg(unix)]
fn fs_chmod(path: &std::path::Path, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn fs_chmod(_path: &std::path::Path, _mode: u32) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "chmod is unsupported",
    ))
}

#[cfg(unix)]
fn fs_chown(path: &std::path::Path, value: &str, follow: bool) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let mut values = value.split(',');
    let parse_id = |value: Option<&str>| -> io::Result<libc::uid_t> {
        let value = value.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing owner ID"))?
            .parse::<i64>().map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid owner ID"))?;
        if value == -1 { return Ok(libc::uid_t::MAX); }
        libc::uid_t::try_from(value).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid owner ID"))
    };
    let uid = parse_id(values.next())?;
    let gid = parse_id(values.next())?;
    let path = CString::new(path.as_os_str().as_bytes())
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
fn fs_chown(_path: &std::path::Path, _value: &str, _follow: bool) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "ownership changes are unsupported",
    ))
}

#[cfg(unix)]
fn fs_access(path: &std::path::Path, mode: i32) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains a NUL byte"))?;
    if unsafe { libc::access(path.as_ptr(), mode) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(any(target_os = "linux", target_os = "android", target_os = "macos", target_os = "freebsd", target_os = "dragonfly", target_os = "aix"))]
fn fs_statfs_type(call: impl FnOnce(*mut libc::statfs) -> libc::c_int) -> io::Result<u64> {
    let mut stats = std::mem::MaybeUninit::<libc::statfs>::uninit();
    if call(stats.as_mut_ptr()) != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { stats.assume_init().f_type as u64 })
}

#[cfg(unix)]
fn fs_statfs_record(stats: libc::statvfs, fs_type: u64) -> serde_json::Value {
    let exact = serde_json::json!({
        "type": fs_type.to_string(), "bsize": stats.f_bsize.to_string(),
        "blocks": stats.f_blocks.to_string(), "bfree": stats.f_bfree.to_string(),
        "bavail": stats.f_bavail.to_string(), "files": stats.f_files.to_string(),
        "ffree": stats.f_ffree.to_string(),
    });
    serde_json::json!({ "ok": true, "type": fs_type, "bsize": stats.f_bsize,
        "blocks": stats.f_blocks, "bfree": stats.f_bfree, "bavail": stats.f_bavail,
        "files": stats.f_files, "ffree": stats.f_ffree, "exact": exact })
}

#[cfg(unix)]
fn fs_statfs(path: &std::path::Path) -> io::Result<serde_json::Value> {
    use std::os::unix::ffi::OsStrExt;
    let path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains a NUL byte"))?;
    let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let stats = unsafe { stats.assume_init() };
    #[cfg(any(target_os = "linux", target_os = "android", target_os = "macos", target_os = "freebsd", target_os = "dragonfly", target_os = "aix"))]
    let fs_type = fs_statfs_type(|record| unsafe { libc::statfs(path.as_ptr(), record) })?;
    #[cfg(not(any(target_os = "linux", target_os = "android", target_os = "macos", target_os = "freebsd", target_os = "dragonfly", target_os = "aix")))]
    let fs_type = 0;
    Ok(fs_statfs_record(stats, fs_type))
}

#[cfg(not(unix))]
fn fs_statfs(_path: &std::path::Path) -> io::Result<serde_json::Value> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "filesystem statistics are unsupported",
    ))
}

#[cfg(not(unix))]
fn fs_access(path: &std::path::Path, mode: i32) -> io::Result<()> {
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

#[cfg(unix)]
fn fs_same_metadata(source: &std::fs::Metadata, destination: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    source.dev() == destination.dev() && source.ino() == destination.ino()
}

fn fs_same_file(source: &std::path::Path, destination: &std::path::Path) -> io::Result<bool> {
    let source_meta = std::fs::metadata(source)?;
    let destination_meta = match std::fs::metadata(destination) {
        Ok(value) => value,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    #[cfg(unix)] { Ok(fs_same_metadata(&source_meta, &destination_meta)) }
    #[cfg(not(unix))] {
        let _ = (source_meta, destination_meta);
        Ok(std::fs::canonicalize(source)? == std::fs::canonicalize(destination)?)
    }
}

fn fs_copy_destination_resolved(destination: &std::path::Path) -> io::Result<std::path::PathBuf> {
    use std::path::Component;
    let absolute = if destination.is_absolute() { destination.to_path_buf() }
        else { std::env::current_dir()?.join(destination) };
    let mut resolved = std::path::PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => resolved.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => { resolved.pop(); }
            Component::Normal(name) => {
                resolved.push(name);
                match std::fs::canonicalize(&resolved) {
                    Ok(path) => resolved = path,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {
                        if std::fs::symlink_metadata(&resolved).is_ok_and(|meta| meta.file_type().is_symlink()) {
                            return Err(io::Error::new(io::ErrorKind::InvalidInput, "unresolved destination symlink"));
                        }
                    }
                    Err(error) => return Err(error),
                }
            }
        }
    }
    Ok(resolved)
}

fn fs_copy_unsafe_relation(source: &std::path::Path, destination: &std::path::Path, dereference: bool) -> io::Result<bool> {
    let source_link = std::fs::symlink_metadata(source)?;
    if source_link.file_type().is_symlink() && !dereference {
        return match std::fs::symlink_metadata(destination) {
            Ok(destination_link) => {
                #[cfg(unix)] { Ok(fs_same_metadata(&source_link, &destination_link)) }
                #[cfg(not(unix))] { Ok(std::fs::canonicalize(source)? == std::fs::canonicalize(destination)?) }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        };
    }
    if fs_same_file(source, destination)? { return Ok(true); }
    let canonical_source = std::fs::canonicalize(source)?;
    let canonical_destination = fs_copy_destination_resolved(destination)?;
    Ok(canonical_destination == canonical_source || (source_link.is_dir() || (dereference && std::fs::metadata(source)?.is_dir()))
        && canonical_destination.starts_with(&canonical_source))
}

fn fs_copy_file(source: &std::path::Path, destination: &std::path::Path) -> io::Result<u64> {
    let mut input = std::fs::File::open(source)?;
    let source_meta = input.metadata()?;
    if fs_same_file(source, destination)? {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "source and destination are the same file"));
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true);
    #[cfg(unix)] { use std::os::unix::fs::{OpenOptionsExt, PermissionsExt}; options.mode(source_meta.permissions().mode() & 0o7777); }
    let mut output = options.open(destination)?;
    #[cfg(unix)]
    if fs_same_metadata(&source_meta, &output.metadata()?) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "source and destination are the same file"));
    }
    #[cfg(not(unix))]
    if fs_same_file(source, destination)? {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "source and destination are the same file"));
    }
    output.set_len(0)?;
    let bytes = io::copy(&mut input, &mut output)?;
    output.set_permissions(source_meta.permissions())?;
    Ok(bytes)
}

fn fs_copy_exclusive(source: &std::path::Path, destination: &std::path::Path) -> io::Result<u64> {
    let mut input = std::fs::File::open(source)?;
    let source_meta = input.metadata()?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)] { use std::os::unix::fs::{OpenOptionsExt, PermissionsExt}; options.mode(source_meta.permissions().mode() & 0o7777); }
    let mut output = options.open(destination)?;
    let bytes = io::copy(&mut input, &mut output)?;
    output.set_permissions(source_meta.permissions())?;
    Ok(bytes)
}

fn fs_parse_mode(value: &str) -> io::Result<u32> {
    let mode = value.parse::<u32>()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid file mode"))?;
    if mode > 0o7777 { return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid file mode")); }
    Ok(mode)
}

fn fs_copy_recursive(source: &std::path::Path, destination: &std::path::Path) -> io::Result<u64> {
    let metadata = std::fs::symlink_metadata(source)?;
    if metadata.is_dir() {
        if fs_copy_unsafe_relation(source, destination, false)? {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "destination is source or its descendant"));
        }
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
        fs_copy_file(source, destination)
    }
}

fn fs_create_empty(path: &std::path::Path, value: &str, exclusive: bool) -> io::Result<()> {
    let mode = fs_parse_mode(value)?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(!exclusive).create_new(exclusive);
    #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; options.mode(mode); }
    #[cfg(not(unix))] let _ = mode;
    options.open(path).map(|_| ())
}

#[cfg(unix)]
fn fs_write_with_mode(path: &std::path::Path, value: &str, append: bool, exclusive: bool) -> io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let (mode, encoded) = value.split_once(',').ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing file mode"))?;
    let mode = fs_parse_mode(mode)?;
    let mut options = std::fs::OpenOptions::new();
    options
        .create(!exclusive)
        .create_new(exclusive)
        .write(true)
        .append(append)
        .truncate(!append && !exclusive)
        .mode(mode);
    options.open(path)?.write_all(&hex_decode(encoded))
}

#[cfg(not(unix))]
fn fs_write_with_mode(path: &std::path::Path, value: &str, append: bool, exclusive: bool) -> io::Result<()> {
    let (mode, encoded) = value.split_once(',').ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing file mode"))?;
    fs_parse_mode(mode)?;
    let mut options = std::fs::OpenOptions::new();
    options
        .create(!exclusive)
        .create_new(exclusive)
        .write(true)
        .append(append)
        .truncate(!append && !exclusive);
    options.open(path)?.write_all(&hex_decode(encoded))
}

#[cfg(unix)]
fn fs_create_dir_mode(path: &std::path::Path, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new().mode(mode).create(path)
}

#[cfg(not(unix))]
fn fs_create_dir_mode(path: &std::path::Path, _mode: u32) -> io::Result<()> {
    std::fs::create_dir(path)
}

fn fs_mkdir_recursive(path: &std::path::Path, mode: u32) -> io::Result<Option<std::path::PathBuf>> {
    match fs_create_dir_mode(path, mode) {
        Ok(()) => Ok(Some(path.to_path_buf())),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists && path.is_dir() => Ok(None),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let parent = path.parent().filter(|parent| !parent.as_os_str().is_empty())
                .ok_or(error)?;
            let first = fs_mkdir_recursive(parent, mode)?;
            match fs_create_dir_mode(path, mode) {
                Ok(()) => Ok(first.or_else(|| Some(path.to_path_buf()))),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists && path.is_dir() => Ok(first),
                Err(error) => Err(error),
            }
        }
        Err(error) => Err(error),
    }
}

fn fs_mkdir_with_mode(path: &std::path::Path, value: &str, recursive: bool) -> io::Result<Option<std::path::PathBuf>> {
    let mode = fs_parse_mode(value)?;
    if !recursive {
        fs_create_dir_mode(path, mode)?;
        return Ok(None);
    }
    fs_mkdir_recursive(path, mode)
}

fn system_time_millis(time: io::Result<std::time::SystemTime>) -> f64 {
    time.ok().map(|value| match value.duration_since(std::time::UNIX_EPOCH) {
        Ok(duration) => duration.as_secs_f64() * 1000.0,
        Err(error) => -error.duration().as_secs_f64() * 1000.0,
    }).unwrap_or(0.0)
}

fn system_time_nanos(time: io::Result<std::time::SystemTime>) -> i128 {
    time.ok().map(|value| match value.duration_since(std::time::UNIX_EPOCH) {
        Ok(duration) => i128::from(duration.as_secs()) * 1_000_000_000 + i128::from(duration.subsec_nanos()),
        Err(error) => {
            let duration = error.duration();
            -(i128::from(duration.as_secs()) * 1_000_000_000 + i128::from(duration.subsec_nanos()))
        }
    }).unwrap_or(0)
}

#[cfg(unix)]
fn fs_metadata_record(metadata: std::fs::Metadata) -> serde_json::Value {
    use std::os::unix::fs::MetadataExt;
    let ctime = metadata.ctime() as f64 * 1000.0 + metadata.ctime_nsec() as f64 / 1_000_000.0;
    let exact = serde_json::json!({
        "length": metadata.len().to_string(), "dev": metadata.dev().to_string(),
        "ino": metadata.ino().to_string(), "mode": metadata.mode().to_string(),
        "nlink": metadata.nlink().to_string(), "uid": metadata.uid().to_string(),
        "gid": metadata.gid().to_string(), "rdev": metadata.rdev().to_string(),
        "blksize": metadata.blksize().to_string(), "blocks": metadata.blocks().to_string(),
        "atimeNs": (i128::from(metadata.atime()) * 1_000_000_000 + i128::from(metadata.atime_nsec())).to_string(),
        "mtimeNs": (i128::from(metadata.mtime()) * 1_000_000_000 + i128::from(metadata.mtime_nsec())).to_string(),
        "ctimeNs": (i128::from(metadata.ctime()) * 1_000_000_000 + i128::from(metadata.ctime_nsec())).to_string(),
        "birthtimeNs": system_time_nanos(metadata.created()).to_string(),
    });
    serde_json::json!({ "ok": true, "length": metadata.len(), "file": metadata.is_file(), "directory": metadata.is_dir(), "symlink": metadata.file_type().is_symlink(), "readonly": metadata.permissions().readonly(), "dev": metadata.dev(), "ino": metadata.ino(), "mode": metadata.mode(), "nlink": metadata.nlink(), "uid": metadata.uid(), "gid": metadata.gid(), "rdev": metadata.rdev(), "blksize": metadata.blksize(), "blocks": metadata.blocks(), "atimeMs": system_time_millis(metadata.accessed()), "mtimeMs": system_time_millis(metadata.modified()), "ctimeMs": ctime, "birthtimeMs": system_time_millis(metadata.created()), "exact": exact })
}

#[cfg(unix)]
fn fs_name_bytes(name: &std::ffi::OsStr) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    name.as_bytes().to_vec()
}

#[cfg(not(unix))]
fn fs_name_bytes(name: &std::ffi::OsStr) -> Vec<u8> {
    name.to_string_lossy().as_bytes().to_vec()
}

fn fs_readdir_entries(path: &std::path::Path, typed: bool, recursive: bool) -> io::Result<serde_json::Value> {
    fn visit(directory: &std::path::Path, prefix: &[u8], typed: bool, recursive: bool, output: &mut Vec<serde_json::Value>) -> io::Result<()> {
        for entry in std::fs::read_dir(directory)? {
            let entry = entry?;
            let name_bytes = fs_name_bytes(&entry.file_name());
            let mut relative_bytes = prefix.to_vec();
            if !relative_bytes.is_empty() { relative_bytes.push(b'/'); }
            relative_bytes.extend_from_slice(&name_bytes);
            let entry_path = entry.path();
            let metadata = if typed || recursive { Some(std::fs::symlink_metadata(&entry_path)?) } else { None };
            let descend = recursive && metadata.as_ref().is_some_and(|value| value.is_dir() && !value.file_type().is_symlink());
            output.push(serde_json::json!({
                "nameHex": hex_encode(&name_bytes),
                "relativeHex": hex_encode(&relative_bytes),
                "pathHex": hex_encode(&fs_name_bytes(entry_path.as_os_str())),
                "parentPath": directory.to_string_lossy(),
                "stat": metadata.map(fs_metadata_record),
            }));
            if descend { visit(&entry_path, &relative_bytes, typed, recursive, output)?; }
        }
        Ok(())
    }
    let mut entries = Vec::new();
    visit(path, &[], typed, recursive, &mut entries)?;
    Ok(serde_json::json!({ "ok": true, "entries": entries }))
}

fn fs_created_directory_record(created: Option<std::path::PathBuf>) -> serde_json::Value {
    match created {
        Some(path) => serde_json::json!({ "ok": true, "created": path.to_string_lossy(), "createdHex": hex_encode(&fs_name_bytes(path.as_os_str())) }),
        None => serde_json::json!({ "ok": true, "created": null, "createdHex": null }),
    }
}

fn fs_decode_raw_path(encoded: &str) -> io::Result<Vec<u8>> {
    if encoded.len() % 2 != 0 || !encoded.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid encoded path"));
    }
    let bytes = hex_decode(encoded);
    if bytes.contains(&0) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "path contains a NUL byte"));
    }
    Ok(bytes)
}

#[cfg(unix)]
fn fs_raw_path(encoded: &str) -> io::Result<std::path::PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    Ok(std::ffi::OsString::from_vec(fs_decode_raw_path(encoded)?).into())
}

#[cfg(not(unix))]
fn fs_raw_path(encoded: &str) -> io::Result<std::path::PathBuf> {
    String::from_utf8(fs_decode_raw_path(encoded)?).map(std::path::PathBuf::from)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid UTF-8 path"))
}

#[cfg(not(unix))]
fn fs_metadata_record(metadata: std::fs::Metadata) -> serde_json::Value {
    let modified = system_time_millis(metadata.modified());
    let exact = serde_json::json!({
        "length": metadata.len().to_string(), "dev": "0", "ino": "0",
        "mode": if metadata.is_dir() { "16877" } else { "33188" },
        "nlink": "1", "uid": "0", "gid": "0", "rdev": "0", "blksize": "0", "blocks": "0",
        "atimeNs": system_time_nanos(metadata.accessed()).to_string(),
        "mtimeNs": system_time_nanos(metadata.modified()).to_string(),
        "ctimeNs": system_time_nanos(metadata.modified()).to_string(),
        "birthtimeNs": system_time_nanos(metadata.created()).to_string(),
    });
    serde_json::json!({ "ok": true, "length": metadata.len(), "file": metadata.is_file(), "directory": metadata.is_dir(), "symlink": metadata.file_type().is_symlink(), "readonly": metadata.permissions().readonly(), "dev": 0, "ino": 0, "mode": if metadata.is_dir() { 16877 } else { 33188 }, "nlink": 1, "uid": 0, "gid": 0, "rdev": 0, "blksize": 0, "blocks": 0, "atimeMs": system_time_millis(metadata.accessed()), "mtimeMs": modified, "ctimeMs": modified, "birthtimeMs": system_time_millis(metadata.created()), "exact": exact })
}

fn parse_fs_time(value: Option<&str>) -> io::Result<f64> {
    let seconds = value.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing timestamp"))?
        .parse::<f64>().map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid timestamp"))?;
    if !seconds.is_finite() { return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid timestamp")); }
    Ok(seconds)
}

#[cfg(unix)]
fn fs_utimes(path: &std::path::Path, value: &str) -> io::Result<()> {
    fs_path_utimes(path, value, 0)
}

#[cfg(not(unix))]
fn fs_utimes(path: &std::path::Path, value: &str) -> io::Result<()> {
    let mut values = value.split(',');
    let timestamp = |seconds: f64| -> io::Result<std::time::SystemTime> {
        if seconds.abs() >= u64::MAX as f64 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "timestamp out of range"));
        }
        let duration = Duration::from_secs_f64(seconds.abs());
        let time = if seconds < 0.0 { std::time::UNIX_EPOCH.checked_sub(duration) }
            else { std::time::UNIX_EPOCH.checked_add(duration) };
        time.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "timestamp out of range"))
    };
    let times = std::fs::FileTimes::new()
        .set_accessed(timestamp(parse_fs_time(values.next())?)?)
        .set_modified(timestamp(parse_fs_time(values.next())?)?);
    std::fs::OpenOptions::new().write(true).open(path)?.set_times(times)
}

#[cfg(unix)]
fn fs_lutimes(path: &std::path::Path, value: &str) -> io::Result<()> {
    fs_path_utimes(path, value, libc::AT_SYMLINK_NOFOLLOW)
}

#[cfg(unix)]
fn fs_timespec(value: Option<&str>) -> io::Result<libc::timespec> {
    let seconds = parse_fs_time(value)?;
    let mut sec = seconds.trunc() as i128;
    if sec < libc::time_t::MIN as i128 || sec > libc::time_t::MAX as i128 {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "timestamp out of range"));
    }
    let mut nsec = ((seconds - sec as f64) * 1_000_000_000.0).trunc() as i128;
    if nsec < 0 {
        sec -= 1;
        nsec += 1_000_000_000;
    } else if nsec >= 1_000_000_000 {
        sec += 1;
        nsec -= 1_000_000_000;
    }
    if sec < libc::time_t::MIN as i128 || sec > libc::time_t::MAX as i128
        || !(0..1_000_000_000).contains(&nsec) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "timestamp out of range"));
    }
    Ok(libc::timespec { tv_sec: sec as libc::time_t, tv_nsec: nsec as libc::c_long })
}

#[cfg(unix)]
fn fs_path_utimes(path: &std::path::Path, value: &str, flags: libc::c_int) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let mut values = value.split(',');
    let times = [fs_timespec(values.next())?, fs_timespec(values.next())?];
    let path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains a NUL byte"))?;
    if unsafe { libc::utimensat(libc::AT_FDCWD, path.as_ptr(), times.as_ptr(), flags) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(not(unix))]
fn fs_lutimes(_path: &std::path::Path, _value: &str) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "symbolic-link timestamps are unsupported",
    ))
}

struct FsHandleTable {
    next: u32,
    files: HashMap<u32, std::fs::File>,
}

impl FsHandleTable {
    fn new() -> Self { Self { next: 10, files: HashMap::new() } }
    fn get(&mut self, fd: u32) -> io::Result<&mut std::fs::File> {
        self.files.get_mut(&fd).ok_or_else(|| io::Error::from_raw_os_error(libc::EBADF))
    }
}

fn fs_fd_time(value: &str) -> io::Result<std::fs::FileTimes> {
    let mut values = value.split(',');
    let timestamp = |seconds: f64| -> io::Result<std::time::SystemTime> {
        if seconds.abs() >= u64::MAX as f64 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "timestamp out of range"));
        }
        let duration = Duration::from_secs_f64(seconds.abs());
        let time = if seconds < 0.0 { std::time::UNIX_EPOCH.checked_sub(duration) }
            else { std::time::UNIX_EPOCH.checked_add(duration) };
        time.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "timestamp out of range"))
    };
    Ok(std::fs::FileTimes::new()
        .set_accessed(timestamp(parse_fs_time(values.next())?)?)
        .set_modified(timestamp(parse_fs_time(values.next())?)?))
}

fn fs_open_fd(path: &std::path::Path, value: &str, table: &mut FsHandleTable) -> io::Result<u32> {
    let (flag, mode) = value.split_once(',').ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing open mode"))?;
    let mode = fs_parse_mode(mode)?;
    let fd = table.next;
    if fd == u32::MAX { return Err(io::Error::new(io::ErrorKind::Other, "too many file handles")); }
    let file = if let Some(bits) = flag.strip_prefix('#') {
        let bits = bits.parse::<i32>().map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid open flags"))?;
        #[cfg(unix)] {
            use std::os::fd::FromRawFd;
            use std::os::unix::ffi::OsStrExt;
            let path = CString::new(path.as_os_str().as_bytes())
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains a NUL byte"))?;
            let raw = unsafe { libc::open(path.as_ptr(), bits, mode as libc::mode_t) };
            if raw < 0 { return Err(io::Error::last_os_error()); }
            unsafe { std::fs::File::from_raw_fd(raw) }
        }
        #[cfg(not(unix))] {
            let _ = bits;
            return Err(io::Error::new(io::ErrorKind::Unsupported, "numeric open flags are unsupported"));
        }
    } else {
        let mut options = std::fs::OpenOptions::new();
        match flag {
            "r" | "rs" => { options.read(true); },
            "r+" | "rs+" => { options.read(true).write(true); },
            "w" => { options.write(true).create(true).truncate(true); },
            "wx" | "xw" => { options.write(true).create_new(true); },
            "w+" => { options.read(true).write(true).create(true).truncate(true); },
            "wx+" | "xw+" => { options.read(true).write(true).create_new(true); },
            "a" | "as" => { options.append(true).create(true); },
            "ax" | "xa" => { options.append(true).create_new(true); },
            "a+" | "as+" => { options.read(true).append(true).create(true); },
            "ax+" | "xa+" => { options.read(true).append(true).create_new(true); },
            _ => return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid open flags")),
        }
        #[cfg(unix)] {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(mode);
            if flag.contains('s') { options.custom_flags(libc::O_SYNC); }
        }
        options.open(path)?
    };
    table.next += 1;
    table.files.insert(fd, file);
    Ok(fd)
}

fn fs_fd_operation(operation: &str, fd: u32, value: &str, table: &mut FsHandleTable) -> io::Result<serde_json::Value> {
    if operation == "fd_close" {
        if table.files.remove(&fd).is_none() { return Err(io::Error::from_raw_os_error(libc::EBADF)); }
        return Ok(serde_json::json!({ "ok": true }));
    }
    let file = table.get(fd)?;
    match operation {
        "fd_read" => {
            let (position, length) = value.split_once(',').ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid read options"))?;
            let length = length.parse::<usize>().map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid read length"))?;
            let mut bytes = vec![0; length];
            let count = if position == "-1" { file.read(&mut bytes)? } else {
                let position = position.parse::<u64>().map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid read position"))?;
                let cursor = file.stream_position()?;
                file.seek(SeekFrom::Start(position))?;
                let result = file.read(&mut bytes);
                file.seek(SeekFrom::Start(cursor))?;
                result?
            };
            bytes.truncate(count);
            Ok(serde_json::json!({ "ok": true, "data": hex_encode(&bytes), "length": count }))
        }
        "fd_write" => {
            let (position, encoded) = value.split_once(':').ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid write options"))?;
            let bytes = hex_decode(encoded);
            let count = if position == "-1" { file.write(&bytes)? } else {
                let position = position.parse::<u64>().map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid write position"))?;
                let cursor = file.stream_position()?;
                file.seek(SeekFrom::Start(position))?;
                let result = file.write(&bytes);
                file.seek(SeekFrom::Start(cursor))?;
                result?
            };
            Ok(serde_json::json!({ "ok": true, "length": count }))
        }
        "fd_read_all" => { let mut bytes = Vec::new(); file.read_to_end(&mut bytes)?; Ok(serde_json::json!({ "ok": true, "data": hex_encode(&bytes) })) }
        "fd_write_all" => { file.write_all(&hex_decode(value))?; Ok(serde_json::json!({ "ok": true })) }
        "fd_stat" => Ok(fs_metadata_record(file.metadata()?)),
        "fd_statfs" => {
            #[cfg(unix)] {
                use std::os::fd::AsRawFd;
                let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
                if unsafe { libc::fstatvfs(file.as_raw_fd(), stats.as_mut_ptr()) } != 0 { return Err(io::Error::last_os_error()); }
                let stats = unsafe { stats.assume_init() };
                #[cfg(any(target_os = "linux", target_os = "android", target_os = "macos", target_os = "freebsd", target_os = "dragonfly", target_os = "aix"))]
                let fs_type = fs_statfs_type(|record| unsafe { libc::fstatfs(file.as_raw_fd(), record) })?;
                #[cfg(not(any(target_os = "linux", target_os = "android", target_os = "macos", target_os = "freebsd", target_os = "dragonfly", target_os = "aix")))]
                let fs_type = 0;
                Ok(fs_statfs_record(stats, fs_type))
            }
            #[cfg(not(unix))] { Err(io::Error::new(io::ErrorKind::Unsupported, "filesystem statistics are unsupported")) }
        }
        "fd_tell" => Ok(serde_json::json!({ "ok": true, "position": file.stream_position()? })),
        "fd_truncate" => { let length = value.parse::<u64>().map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid truncate length"))?; file.set_len(length)?; Ok(serde_json::json!({ "ok": true })) }
        "fd_sync" => { file.sync_all()?; Ok(serde_json::json!({ "ok": true })) }
        "fd_datasync" => { file.sync_data()?; Ok(serde_json::json!({ "ok": true })) }
        "fd_chmod" => {
            let mode = fs_parse_mode(value)?;
            #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; file.set_permissions(std::fs::Permissions::from_mode(mode))?; }
            #[cfg(not(unix))] { let _ = mode; return Err(io::Error::new(io::ErrorKind::Unsupported, "fchmod unsupported")); }
            Ok(serde_json::json!({ "ok": true }))
        }
        "fd_utimes" => { file.set_times(fs_fd_time(value)?)?; Ok(serde_json::json!({ "ok": true })) }
        "fd_chown" => {
            #[cfg(unix)] {
                use std::os::fd::AsRawFd;
                let mut values = value.split(',');
                let parse = |part: Option<&str>| -> io::Result<libc::uid_t> {
                    let number = part.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing owner ID"))?.parse::<i64>()
                        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid owner ID"))?;
                    if number == -1 { Ok(libc::uid_t::MAX) } else { libc::uid_t::try_from(number).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid owner ID")) }
                };
                let uid = parse(values.next())?; let gid = parse(values.next())?;
                if unsafe { libc::fchown(file.as_raw_fd(), uid, gid) } != 0 { return Err(io::Error::last_os_error()); }
            }
            #[cfg(not(unix))] { return Err(io::Error::new(io::ErrorKind::Unsupported, "fchown unsupported")); }
            Ok(serde_json::json!({ "ok": true }))
        }
        _ => Err(io::Error::new(io::ErrorKind::InvalidInput, "unknown descriptor operation")),
    }
}

fn host_fs(operation: String, path: String, value: String, recursive: bool, table: &mut FsHandleTable) -> String {
    if path.as_bytes().contains(&0) {
        return fs_error(&operation, &path, io::Error::new(io::ErrorKind::InvalidInput, "path contains a NUL byte"));
    }
    let (operation_without_value, raw_value) = operation.strip_suffix("_raw_value")
        .map_or((operation.as_str(), false), |base| (base, true));
    let (base_operation, raw_path) = operation_without_value.strip_suffix("_raw")
        .map_or((operation_without_value, false), |base| (base, true));
    let path_buf = if raw_path {
        match fs_raw_path(&path) { Ok(path) => path, Err(error) => return fs_error(base_operation, &path, error) }
    } else { std::path::PathBuf::from(&path) };
    let value_path = if raw_value {
        match fs_raw_path(&value) { Ok(path) => path, Err(error) => return fs_error(base_operation, &path, error) }
    } else { std::path::PathBuf::from(&value) };
    let path_ref = path_buf.as_path();
    let value_path_ref = value_path.as_path();
    if base_operation == "fd_open" {
        return fs_open_fd(path_ref, &value, table)
            .map(|fd| serde_json::json!({ "ok": true, "fd": fd }).to_string())
            .unwrap_or_else(|error| fs_error(&operation, &path, error));
    }
    if base_operation.starts_with("fd_") {
        let result = path.parse::<u32>().map_err(|_| io::Error::from_raw_os_error(libc::EBADF))
            .and_then(|fd| fs_fd_operation(base_operation, fd, &value, table));
        return result.map(|value| value.to_string()).unwrap_or_else(|error| fs_error(&operation, &path, error));
    }
    if matches!(base_operation, "rename" | "copy" | "copy_excl" | "cp" | "same_file" | "copy_guard" | "link" | "symlink") && value.as_bytes().contains(&0) {
        return fs_error(&operation, &path, io::Error::new(io::ErrorKind::InvalidInput, "path contains a NUL byte"));
    }
    let result = match base_operation {
        "exists" => return serde_json::json!({ "ok": true, "exists": path_ref.exists() }).to_string(),
        "access" => fs_access(path_ref, value.parse::<i32>().unwrap_or(0)).map(|_| serde_json::json!({ "ok": true })),
        "statfs" => fs_statfs(path_ref),
        "read" => std::fs::read(path_ref).map(|bytes| serde_json::json!({ "ok": true, "data": hex_encode(&bytes) })),
        "read_range" => (|| -> io::Result<serde_json::Value> {
            let (position, length) = value.split_once(',').unwrap_or(("0", "0"));
            let mut file = std::fs::File::open(path_ref)?;
            file.seek(SeekFrom::Start(position.parse::<u64>().unwrap_or(0)))?;
            let mut bytes = vec![0; length.parse::<usize>().unwrap_or(0)];
            let count = file.read(&mut bytes)?;
            bytes.truncate(count);
            Ok(serde_json::json!({ "ok": true, "data": hex_encode(&bytes), "length": count }))
        })(),
        "write" => std::fs::write(path_ref, hex_decode(&value)).map(|_| serde_json::json!({ "ok": true })),
        "create_empty" => fs_create_empty(path_ref, &value, false).map(|_| serde_json::json!({ "ok": true })),
        "create_empty_excl" => fs_create_empty(path_ref, &value, true).map(|_| serde_json::json!({ "ok": true })),
        "write_existing" => std::fs::OpenOptions::new().write(true).open(path_ref)
            .and_then(|mut file| file.write_all(&hex_decode(&value)))
            .map(|_| serde_json::json!({ "ok": true })),
        "write_mode" => fs_write_with_mode(path_ref, &value, false, false).map(|_| serde_json::json!({ "ok": true })),
        "append_mode" => fs_write_with_mode(path_ref, &value, true, false).map(|_| serde_json::json!({ "ok": true })),
        "write_mode_excl" => fs_write_with_mode(path_ref, &value, false, true).map(|_| serde_json::json!({ "ok": true })),
        "append_mode_excl" => fs_write_with_mode(path_ref, &value, true, true).map(|_| serde_json::json!({ "ok": true })),
        "same_file" => fs_same_file(path_ref, value_path_ref).map(|same| serde_json::json!({ "ok": true, "same": same })),
        "copy_guard" => fs_copy_unsafe_relation(path_ref, value_path_ref, recursive).map(|unsafe_path| serde_json::json!({ "ok": true, "unsafe": unsafe_path })),
        "write_range" => (|| -> io::Result<serde_json::Value> {
            let (position, encoded) = value.split_once(':').unwrap_or(("0", ""));
            let bytes = hex_decode(encoded);
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .write(true)
                .open(path_ref)?;
            file.seek(SeekFrom::Start(position.parse::<u64>().unwrap_or(0)))?;
            file.write_all(&bytes)?;
            Ok(serde_json::json!({ "ok": true, "length": bytes.len() }))
        })(),
        "append" => std::fs::OpenOptions::new().create(true).append(true).open(path_ref).and_then(|mut file| file.write_all(&hex_decode(&value))).map(|_| serde_json::json!({ "ok": true })),
        "mkdir" => fs_mkdir_with_mode(path_ref, "511", recursive).map(|created| fs_created_directory_record(created)),
        "mkdir_mode" => fs_mkdir_with_mode(path_ref, &value, recursive).map(|created| fs_created_directory_record(created)),
        "readdir" => fs_readdir_entries(path_ref, value == "typed", recursive),
        "stat" => std::fs::metadata(path_ref).map(fs_metadata_record),
        "lstat" => std::fs::symlink_metadata(path_ref).map(fs_metadata_record),
        "unlink" => std::fs::remove_file(path_ref).map(|_| serde_json::json!({ "ok": true })),
        "rmdir" => if recursive { std::fs::remove_dir_all(path_ref) } else { std::fs::remove_dir(path_ref) }.map(|_| serde_json::json!({ "ok": true })),
        "rename" => std::fs::rename(path_ref, value_path_ref).map(|_| serde_json::json!({ "ok": true })),
        "copy" => fs_copy_file(path_ref, value_path_ref).map(|bytes| serde_json::json!({ "ok": true, "length": bytes })),
        "copy_excl" => fs_copy_exclusive(path_ref, value_path_ref).map(|bytes| serde_json::json!({ "ok": true, "length": bytes })),
        "cp" => if recursive { fs_copy_recursive(path_ref, value_path_ref) } else { fs_copy_file(path_ref, value_path_ref) }.map(|bytes| serde_json::json!({ "ok": true, "length": bytes })),
        "realpath" => std::fs::canonicalize(path_ref).map(|resolved| serde_json::json!({ "ok": true, "path": resolved.to_string_lossy(), "pathHex": hex_encode(&fs_name_bytes(resolved.as_os_str())) })),
        "mkdtemp" => (|| -> io::Result<serde_json::Value> { let mut random = [0u8; 6]; getrandom::getrandom(&mut random).map_err(|error| io::Error::other(error.to_string()))?; let mut name = path_ref.as_os_str().to_os_string(); name.push(hex_encode(&random)); let created = std::path::PathBuf::from(name); #[cfg(unix)] { use std::os::unix::fs::DirBuilderExt; std::fs::DirBuilder::new().mode(0o700).create(&created)?; } #[cfg(not(unix))] std::fs::create_dir(&created)?; Ok(serde_json::json!({ "ok": true, "path": created.to_string_lossy(), "pathHex": hex_encode(&fs_name_bytes(created.as_os_str())) })) })(),
        "truncate" => value.parse::<u64>().map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid truncate length"))
            .and_then(|length| std::fs::OpenOptions::new().write(true).open(path_ref)?.set_len(length))
            .map(|_| serde_json::json!({ "ok": true })),
        "link" => std::fs::hard_link(path_ref, value_path_ref).map(|_| serde_json::json!({ "ok": true })),
        "symlink" => fs_symlink(path_ref, value_path_ref).map(|_| serde_json::json!({ "ok": true })),
        "readlink" => std::fs::read_link(path_ref).map(|target| serde_json::json!({ "ok": true, "path": target.to_string_lossy(), "pathHex": hex_encode(&fs_name_bytes(target.as_os_str())) })),
        "chmod" => fs_parse_mode(&value)
            .and_then(|mode| fs_chmod(path_ref, mode)).map(|_| serde_json::json!({ "ok": true })),
        "utimes" => fs_utimes(path_ref, &value).map(|_| serde_json::json!({ "ok": true })),
        "lutimes" => fs_lutimes(path_ref, &value).map(|_| serde_json::json!({ "ok": true })),
        "chown" => fs_chown(path_ref, &value, true).map(|_| serde_json::json!({ "ok": true })),
        "lchown" => fs_chown(path_ref, &value, false).map(|_| serde_json::json!({ "ok": true })),
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
    static HOST_STDIN: RefCell<Option<HostStdin>> = const { RefCell::new(None) };
    #[cfg(feature = "wasm")]
    static WASM: RefCell<WasmTable> = RefCell::new(WasmTable::default());
    #[cfg(feature = "wasm")]
    static WASM_JS_IMPORTS: RefCell<(u32, HashMap<u32, WasmJsImport>)> = RefCell::new((1, HashMap::new()));
    #[cfg(feature = "wasm")]
    static WASM_JS_VALUES: RefCell<(u32, HashMap<u32, WasmJsValue>)> = RefCell::new((1, HashMap::new()));
}

#[cfg(all(test, unix))]
#[test]
fn fs_metadata_wire_preserves_integer_fields_and_nanoseconds() {
    use std::os::unix::fs::MetadataExt;
    let path = std::env::temp_dir().join(format!(
        "thaw_fs_exact_metadata_{}_{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("test"),
    ));
    std::fs::write(&path, b"data").unwrap();
    let metadata = std::fs::metadata(&path).unwrap();
    let record = fs_metadata_record(metadata.clone());
    assert_eq!(record["exact"]["length"], metadata.len().to_string());
    assert_eq!(record["exact"]["ino"], metadata.ino().to_string());
    assert_eq!(record["exact"]["dev"], metadata.dev().to_string());
    assert_eq!(record["exact"]["mtimeNs"],
        (i128::from(metadata.mtime()) * 1_000_000_000 + i128::from(metadata.mtime_nsec())).to_string());
    assert_eq!(system_time_nanos(Ok(std::time::UNIX_EPOCH - std::time::Duration::from_nanos(1))), -1);
    std::fs::remove_file(path).unwrap();
}

#[cfg(all(test, unix))]
#[test]
fn fs_error_preserves_native_errno_and_path_operation() {
    let code = |raw| {
        serde_json::from_str::<serde_json::Value>(&fs_error("rmdir", "/example", io::Error::from_raw_os_error(raw)))
            .unwrap()["code"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    assert_eq!(code(libc::EPERM), "EPERM");
    assert_eq!(code(libc::EACCES), "EACCES");
    assert_eq!(code(libc::ELOOP), "ELOOP");
    assert_eq!(code(libc::ENOSPC), "ENOSPC");
    assert_eq!(code(libc::EXDEV), "EXDEV");
    assert_eq!(code(libc::ENOTSUP), "ENOTSUP");
    assert_eq!(code(libc::EOPNOTSUPP), "ENOTSUP");
    assert_eq!(code(123456), "ERRNO_123456");
    let fallback: serde_json::Value = serde_json::from_str(&fs_error(
        "rmdir", "/example", io::Error::new(io::ErrorKind::InvalidInput, "invalid path"),
    )).unwrap();
    assert_eq!(fallback["code"], "EINVAL");

    let directory = std::env::temp_dir().join(format!(
        "thaw_fs_errno_{}_{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
    ));
    std::fs::create_dir(&directory).unwrap();
    std::fs::write(directory.join("child"), b"x").unwrap();
    let result: serde_json::Value = serde_json::from_str(&host_fs(
        "rmdir".into(), directory.to_string_lossy().into_owned(), String::new(), false,
        &mut FsHandleTable::new(),
    )).unwrap();
    assert_eq!(result["code"], "ENOTEMPTY");
    assert_eq!(result["operation"], "rmdir");
    assert_eq!(result["path"], directory.to_string_lossy().as_ref());
    std::fs::remove_dir_all(directory).unwrap();
}

#[cfg(all(test, windows))]
#[test]
fn fs_error_preserves_platform_error_kinds() {
    let code = |raw| -> String {
        serde_json::from_str::<serde_json::Value>(&fs_error("open", "C:\\example", io::Error::from_raw_os_error(raw)))
            .unwrap()["code"].as_str().unwrap().to_owned()
    };
    assert_eq!(code(2), "ENOENT");
    assert_eq!(code(5), "EACCES");
    assert_eq!(code(123456), "ERRNO_123456");
}

#[cfg(all(test, target_os = "linux"))]
#[test]
fn fs_statfs_reports_linux_type_for_path_and_descriptor() {
    let directory = std::env::temp_dir();
    let mut table = FsHandleTable::new();
    let fd = table.next;
    table.next += 1;
    table.files.insert(fd, std::fs::File::open(&directory).unwrap());
    let path_result: serde_json::Value = serde_json::from_str(&host_fs(
        "statfs".into(), directory.to_string_lossy().into_owned(), String::new(), false, &mut table,
    )).unwrap();
    let fd_result: serde_json::Value = serde_json::from_str(&host_fs(
        "fd_statfs".into(), fd.to_string(), String::new(), false, &mut table,
    )).unwrap();
    assert_eq!(path_result["ok"], true);
    assert_eq!(fd_result["ok"], true);
    assert_ne!(path_result["type"], 0);
    assert_eq!(fd_result["type"], path_result["type"]);
    assert_eq!(fd_result["blocks"], path_result["blocks"]);
}

#[cfg(all(test, unix))]
#[test]
fn fs_timespec_normalizes_negative_fraction_and_bounds() {
    let tiny = fs_timespec(Some("-0.00000000000000001")).unwrap();
    assert_eq!((tiny.tv_sec, tiny.tv_nsec), (0, 0));
    let negative = fs_timespec(Some("-0.000000001")).unwrap();
    assert_eq!((negative.tv_sec, negative.tv_nsec), (-1, 999_999_999));
    let subnanosecond = fs_timespec(Some("0.0000000006")).unwrap();
    assert_eq!((subnanosecond.tv_sec, subnanosecond.tv_nsec), (0, 0));
    let milliseconds = fs_timespec(Some("1.234")).unwrap();
    assert_eq!((milliseconds.tv_sec, milliseconds.tv_nsec), (1, 233_999_999));
    assert!(fs_timespec(Some("NaN")).is_err());
    assert!(fs_timespec(Some("9223372036854775808")).is_err());
    assert!(fs_timespec(Some("-1e300")).is_err());
    assert!(fs_timespec(Some("1e300")).is_err());
}

#[cfg(all(test, unix))]
#[test]
fn fs_utimes_updates_permissionless_target_and_preserves_link_behavior() {
    use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};
    let directory = std::env::temp_dir().join(format!(
        "thaw_fs_utimes_{}_{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
    ));
    std::fs::create_dir(&directory).unwrap();
    let target = directory.join("target");
    let link = directory.join("link");
    std::fs::write(&target, b"x").unwrap();
    symlink(&target, &link).unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o000)).unwrap();
    fs_utimes(&target, "-0.000000001,1.234").unwrap();
    let target_stat = std::fs::metadata(&target).unwrap();
    assert_eq!((target_stat.atime(), target_stat.atime_nsec()), (-1, 999_999_999));
    assert_eq!((target_stat.mtime(), target_stat.mtime_nsec()), (1, 233_999_999));
    fs_lutimes(&link, "2.5,3.5").unwrap();
    assert_eq!(std::fs::symlink_metadata(&link).unwrap().mtime(), 3);
    assert_eq!(std::fs::metadata(&target).unwrap().mtime(), 1);
    fs_utimes(&link, "4,5").unwrap();
    assert_eq!(std::fs::metadata(&target).unwrap().mtime(), 5);
    assert_eq!(std::fs::symlink_metadata(&link).unwrap().mtime(), 3);
    std::fs::remove_dir_all(directory).unwrap();
}

#[cfg(all(test, unix))]
#[test]
fn fs_raw_path_transport_rejects_malformed_hex_and_nul() {
    use std::os::unix::ffi::OsStrExt;
    assert_eq!(fs_raw_path("ff2ffe").unwrap().as_os_str().as_bytes(), &[0xff, b'/', 0xfe]);
    assert_eq!(fs_raw_path("f").unwrap_err().kind(), io::ErrorKind::InvalidInput);
    assert_eq!(fs_raw_path("fg").unwrap_err().kind(), io::ErrorKind::InvalidInput);
    assert_eq!(fs_raw_path("610062").unwrap_err().kind(), io::ErrorKind::InvalidInput);
}

#[cfg(all(test, target_os = "linux", target_pointer_width = "64"))]
#[test]
fn fs_statfs_record_preserves_large_counters_as_decimal_strings() {
    let mut stats: libc::statvfs = unsafe { std::mem::zeroed() };
    stats.f_bsize = 9_007_199_254_740_995;
    stats.f_blocks = 9_007_199_254_740_997;
    stats.f_bfree = 9_007_199_254_740_999;
    stats.f_bavail = 9_007_199_254_741_001;
    stats.f_files = 9_007_199_254_741_003;
    stats.f_ffree = 9_007_199_254_741_005;
    let record = fs_statfs_record(stats, 9_007_199_254_740_993);
    for (key, expected) in [
        ("type", "9007199254740993"), ("bsize", "9007199254740995"),
        ("blocks", "9007199254740997"), ("bfree", "9007199254740999"),
        ("bavail", "9007199254741001"), ("files", "9007199254741003"),
        ("ffree", "9007199254741005"),
    ] {
        assert_eq!(record["exact"][key], expected);
    }
    assert!(record["blocks"].is_number());
}

#[cfg(all(test, unix))]
#[test]
fn fs_copy_relation_resolves_alias_parents_before_descendant_mutation() {
    use std::os::unix::fs::symlink;
    let root = std::env::temp_dir().join(format!("thaw_fs_copy_relation_{}", std::process::id()));
    let source = root.join("source");
    let outside = root.join("outside");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::create_dir_all(outside.join("deep")).unwrap();
    symlink(&source, root.join("source-alias")).unwrap();
    symlink(outside.join("deep"), root.join("outside-alias")).unwrap();
    assert!(fs_copy_unsafe_relation(&source, &root.join("source-alias/new"), false).unwrap());
    assert!(fs_copy_unsafe_relation(&source, &root.join("missing/../source/new"), false).unwrap());
    assert!(!fs_copy_unsafe_relation(&source, &root.join("outside-alias/../source/new"), false).unwrap());
    assert!(fs_copy_recursive(&source, &root.join("source-alias/new")).is_err());
    assert!(!root.join("missing").exists());
    assert!(!source.join("new").exists());
    std::fs::remove_dir_all(root).unwrap();
}
