//! Safe wrapper over the vendored SQLite 3.51.3 amalgamation (build.rs).
//! Used by the thaw-js `node:sqlite` natives; engine-neutral, no thaw-js types.

mod backup;
mod db;
mod ffi;
mod session;
mod stmt;

#[cfg(test)]
mod tests;

pub use backup::backup;
pub use db::{Db, OpenOptions, OPEN_CREATE, OPEN_READONLY, OPEN_READWRITE};
pub use session::Session;
pub use stmt::{ColumnMeta, Stmt};

use std::ffi::{c_char, CStr};
use std::fmt;

/// A SQLite column or bound parameter value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Int(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

/// A SQLite failure. `errcode` is the extended result code (e.g. 787 for an
/// FK violation), `errstr` is `sqlite3_errstr` of the primary code (e.g.
/// "constraint failed"), `message` is `sqlite3_errmsg` at the failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub errcode: i32,
    pub errstr: String,
    pub message: String,
}

impl Error {
    /// The primary result code (`errcode & 0xff`), e.g. 19 for constraints.
    pub fn primary(&self) -> i32 {
        self.errcode & 0xff
    }

    /// Error for a failure this crate detects itself (no SQLite message).
    pub(crate) fn local(code: i32, message: &str) -> Error {
        Error {
            errcode: code,
            errstr: errstr(code),
            message: message.to_string(),
        }
    }

    /// Error from a handle (may be null, e.g. when open failed) and the rc.
    pub(crate) fn from_handle(db: *mut ffi::sqlite3, rc: i32) -> Error {
        if db.is_null() {
            return Error {
                errcode: rc,
                errstr: errstr(rc),
                message: errstr(rc),
            };
        }
        // SAFETY: db is a live handle owned by the caller; the C strings it
        // returns are valid until the next call on the handle, copied here.
        unsafe {
            let code = ffi::sqlite3_extended_errcode(db);
            Error {
                errcode: code,
                errstr: errstr(code),
                message: cstr(ffi::sqlite3_errmsg(db)),
            }
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// `sqlite3_errstr` of the primary code of `code`.
pub fn errstr(code: i32) -> String {
    // SAFETY: sqlite3_errstr returns a static NUL-terminated string.
    unsafe { cstr(ffi::sqlite3_errstr(code & 0xff)) }
}

/// Copies a NUL-terminated C string (null becomes "").
///
/// # Safety
/// `p` must be null or point to a NUL-terminated string valid for this call.
pub(crate) unsafe fn cstr(p: *const c_char) -> String {
    if p.is_null() {
        String::new()
    } else {
        CStr::from_ptr(p).to_string_lossy().into_owned()
    }
}

/// Length as the C `int` SQLite takes, or a range error.
pub(crate) fn c_len(n: usize) -> Result<i32> {
    i32::try_from(n).map_err(|_| Error::local(ffi::SQLITE_RANGE, "value too large"))
}
