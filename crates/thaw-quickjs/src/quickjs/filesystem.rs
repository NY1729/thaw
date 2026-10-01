fn fs_error(operation: &str, path: &str, error: io::Error) -> String {
    let code = if error.raw_os_error() == Some(libc::EBADF) { "EBADF" } else { match error.kind() {
        io::ErrorKind::NotFound => "ENOENT",
        io::ErrorKind::PermissionDenied => "EACCES",
        io::ErrorKind::AlreadyExists => "EEXIST",
        io::ErrorKind::InvalidInput => "EINVAL",
        io::ErrorKind::IsADirectory => "EISDIR",
        io::ErrorKind::NotADirectory => "ENOTDIR",
        _ => "EIO",
    } };
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
    let parse_id = |value: Option<&str>| -> io::Result<libc::uid_t> {
        let value = value.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing owner ID"))?
            .parse::<i64>().map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid owner ID"))?;
        if value == -1 { return Ok(libc::uid_t::MAX); }
        libc::uid_t::try_from(value).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid owner ID"))
    };
    let uid = parse_id(values.next())?;
    let gid = parse_id(values.next())?;
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

fn fs_copy_file(source: &std::path::Path, destination: &std::path::Path) -> io::Result<u64> {
    let mut input = std::fs::File::open(source)?;
    let source_meta = input.metadata()?;
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

fn fs_create_empty(path: &str, value: &str, exclusive: bool) -> io::Result<()> {
    let mode = fs_parse_mode(value)?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(!exclusive).create_new(exclusive);
    #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; options.mode(mode); }
    #[cfg(not(unix))] let _ = mode;
    options.open(path).map(|_| ())
}

#[cfg(unix)]
fn fs_write_with_mode(path: &str, value: &str, append: bool, exclusive: bool) -> io::Result<()> {
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
fn fs_write_with_mode(path: &str, value: &str, append: bool, exclusive: bool) -> io::Result<()> {
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
fn fs_mkdir_with_mode(path: &str, value: &str, recursive: bool) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    let mut builder = std::fs::DirBuilder::new();
    builder
        .recursive(recursive)
        .mode(fs_parse_mode(value)?)
        .create(path)
}

#[cfg(not(unix))]
fn fs_mkdir_with_mode(path: &str, value: &str, recursive: bool) -> io::Result<()> {
    fs_parse_mode(value)?;
    std::fs::DirBuilder::new().recursive(recursive).create(path)
}

fn system_time_millis(time: io::Result<std::time::SystemTime>) -> f64 {
    time.ok().map(|value| match value.duration_since(std::time::UNIX_EPOCH) {
        Ok(duration) => duration.as_secs_f64() * 1000.0,
        Err(error) => -error.duration().as_secs_f64() * 1000.0,
    }).unwrap_or(0.0)
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

fn parse_fs_time(value: Option<&str>) -> io::Result<f64> {
    let seconds = value.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing timestamp"))?
        .parse::<f64>().map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid timestamp"))?;
    if !seconds.is_finite() { return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid timestamp")); }
    Ok(seconds)
}

fn fs_utimes(path: &str, value: &str) -> io::Result<()> {
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
fn fs_lutimes(path: &str, value: &str) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let mut values = value.split(',');
    let to_timespec = |value| -> io::Result<libc::timespec> {
        let seconds = parse_fs_time(value)?;
        let whole = seconds.floor();
        if whole < libc::time_t::MIN as f64 || whole >= libc::time_t::MAX as f64 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "timestamp out of range"));
        }
        Ok(libc::timespec {
            tv_sec: whole as libc::time_t,
            tv_nsec: ((seconds - whole) * 1_000_000_000.0).floor() as libc::c_long,
        })
    };
    let times = [to_timespec(values.next())?, to_timespec(values.next())?];
    let path = CString::new(std::ffi::OsStr::new(path).as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains a NUL byte"))?;
    if unsafe { libc::utimensat(libc::AT_FDCWD, path.as_ptr(), times.as_ptr(), libc::AT_SYMLINK_NOFOLLOW) } == 0 {
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

fn fs_open_fd(path: &str, value: &str, table: &mut FsHandleTable) -> io::Result<u32> {
    let (flag, mode) = value.split_once(',').ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing open mode"))?;
    let mode = fs_parse_mode(mode)?;
    let fd = table.next;
    if fd == u32::MAX { return Err(io::Error::new(io::ErrorKind::Other, "too many file handles")); }
    let file = if let Some(bits) = flag.strip_prefix('#') {
        let bits = bits.parse::<i32>().map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid open flags"))?;
        #[cfg(unix)] {
            use std::os::fd::FromRawFd;
            use std::os::unix::ffi::OsStrExt;
            let path = CString::new(std::ffi::OsStr::new(path).as_bytes())
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
                Ok(serde_json::json!({ "ok": true, "type": 0, "bsize": stats.f_bsize, "blocks": stats.f_blocks, "bfree": stats.f_bfree, "bavail": stats.f_bavail, "files": stats.f_files, "ffree": stats.f_ffree }))
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
    if operation == "fd_open" {
        return fs_open_fd(&path, &value, table)
            .map(|fd| serde_json::json!({ "ok": true, "fd": fd }).to_string())
            .unwrap_or_else(|error| fs_error(&operation, &path, error));
    }
    if operation.starts_with("fd_") {
        let result = path.parse::<u32>().map_err(|_| io::Error::from_raw_os_error(libc::EBADF))
            .and_then(|fd| fs_fd_operation(&operation, fd, &value, table));
        return result.map(|value| value.to_string()).unwrap_or_else(|error| fs_error(&operation, &path, error));
    }
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
        "create_empty" => fs_create_empty(&path, &value, false).map(|_| serde_json::json!({ "ok": true })),
        "create_empty_excl" => fs_create_empty(&path, &value, true).map(|_| serde_json::json!({ "ok": true })),
        "write_existing" => std::fs::OpenOptions::new().write(true).open(&path)
            .and_then(|mut file| file.write_all(&hex_decode(&value)))
            .map(|_| serde_json::json!({ "ok": true })),
        "write_mode" => fs_write_with_mode(&path, &value, false, false).map(|_| serde_json::json!({ "ok": true })),
        "append_mode" => fs_write_with_mode(&path, &value, true, false).map(|_| serde_json::json!({ "ok": true })),
        "write_mode_excl" => fs_write_with_mode(&path, &value, false, true).map(|_| serde_json::json!({ "ok": true })),
        "append_mode_excl" => fs_write_with_mode(&path, &value, true, true).map(|_| serde_json::json!({ "ok": true })),
        "same_file" => fs_same_file(std::path::Path::new(&path), std::path::Path::new(&value)).map(|same| serde_json::json!({ "ok": true, "same": same })),
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
        "copy" => fs_copy_file(std::path::Path::new(&path), std::path::Path::new(&value)).map(|bytes| serde_json::json!({ "ok": true, "length": bytes })),
        "copy_excl" => fs_copy_exclusive(std::path::Path::new(&path), std::path::Path::new(&value)).map(|bytes| serde_json::json!({ "ok": true, "length": bytes })),
        "cp" => if recursive { fs_copy_recursive(std::path::Path::new(&path), std::path::Path::new(&value)) } else { fs_copy_file(std::path::Path::new(&path), std::path::Path::new(&value)) }.map(|bytes| serde_json::json!({ "ok": true, "length": bytes })),
        "realpath" => std::fs::canonicalize(&path).map(|resolved| serde_json::json!({ "ok": true, "path": resolved.to_string_lossy() })),
        "mkdtemp" => (|| -> io::Result<serde_json::Value> { let mut random = [0u8; 6]; getrandom::getrandom(&mut random).map_err(|error| io::Error::other(error.to_string()))?; let created = format!("{}{}", path, hex_encode(&random)); #[cfg(unix)] { use std::os::unix::fs::DirBuilderExt; std::fs::DirBuilder::new().mode(0o700).create(&created)?; } #[cfg(not(unix))] std::fs::create_dir(&created)?; Ok(serde_json::json!({ "ok": true, "path": created })) })(),
        "truncate" => value.parse::<u64>().map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid truncate length"))
            .and_then(|length| std::fs::OpenOptions::new().write(true).open(&path)?.set_len(length))
            .map(|_| serde_json::json!({ "ok": true })),
        "link" => std::fs::hard_link(&path, &value).map(|_| serde_json::json!({ "ok": true })),
        "symlink" => fs_symlink(&path, &value).map(|_| serde_json::json!({ "ok": true })),
        "readlink" => std::fs::read_link(&path).map(|target| serde_json::json!({ "ok": true, "path": target.to_string_lossy() })),
        "chmod" => fs_parse_mode(&value)
            .and_then(|mode| fs_chmod(&path, mode)).map(|_| serde_json::json!({ "ok": true })),
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
    static HOST_STDIN: RefCell<Option<HostStdin>> = const { RefCell::new(None) };
    #[cfg(feature = "wasm")]
    static WASM: RefCell<WasmTable> = RefCell::new(WasmTable::default());
    #[cfg(feature = "wasm")]
    static WASM_JS_IMPORTS: RefCell<(u32, HashMap<u32, WasmJsImport>)> = RefCell::new((1, HashMap::new()));
    #[cfg(feature = "wasm")]
    static WASM_JS_VALUES: RefCell<(u32, HashMap<u32, WasmJsValue>)> = RefCell::new((1, HashMap::new()));
}
