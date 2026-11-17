use std::ffi::{c_int, CString};
use std::ptr;
use std::rc::Rc;

use crate::ffi;
use crate::{Error, Result, Stmt};

pub const OPEN_READONLY: i32 = ffi::SQLITE_OPEN_READONLY;
pub const OPEN_READWRITE: i32 = ffi::SQLITE_OPEN_READWRITE;
pub const OPEN_CREATE: i32 = ffi::SQLITE_OPEN_CREATE;

/// Connection options; defaults match node:sqlite.
#[derive(Clone, Debug)]
pub struct OpenOptions {
    pub foreign_keys: bool,
    pub double_quoted_strings: bool,
    /// Busy timeout in milliseconds (0 = fail immediately).
    pub timeout_ms: i32,
    pub allow_extension: bool,
}

impl Default for OpenOptions {
    fn default() -> Self {
        OpenOptions {
            foreign_keys: true,
            double_quoted_strings: false,
            timeout_ms: 0,
            allow_extension: false,
        }
    }
}

/// Owns the connection. Shared (Rc) with every live `Stmt` and `Session`, so
/// the connection closes only after they are all dropped.
pub struct Db {
    pub(crate) inner: Rc<Inner>,
}

pub(crate) struct Inner {
    pub(crate) raw: *mut ffi::sqlite3,
}

impl Drop for Inner {
    fn drop(&mut self) {
        // SAFETY: raw came from sqlite3_open_v2 and is closed exactly once,
        // after every Stmt/Session holding this Rc has finalized/deleted.
        unsafe { ffi::sqlite3_close_v2(self.raw) };
    }
}

impl Db {
    pub fn open(path: &str, flags: i32, opts: &OpenOptions) -> Result<Db> {
        let c =
            CString::new(path).map_err(|_| Error::local(ffi::SQLITE_ERROR, "path contains NUL"))?;
        let mut raw = ptr::null_mut();
        // SAFETY: c is NUL-terminated and alive for the call; raw is a valid out slot.
        let rc = unsafe { ffi::sqlite3_open_v2(c.as_ptr(), &mut raw, flags, ptr::null()) };
        if rc != ffi::SQLITE_OK {
            let err = Error::from_handle(raw, rc);
            // SAFETY: a failed open may still return a handle that must be closed.
            unsafe { ffi::sqlite3_close_v2(raw) };
            return Err(err);
        }
        let db = Db {
            inner: Rc::new(Inner { raw }),
        };
        db.config(ffi::SQLITE_DBCONFIG_ENABLE_FKEY, opts.foreign_keys)?;
        db.config(ffi::SQLITE_DBCONFIG_DQS_DML, opts.double_quoted_strings)?;
        db.config(ffi::SQLITE_DBCONFIG_DQS_DDL, opts.double_quoted_strings)?;
        // SAFETY: raw is live; sqlite3_enable_load_extension takes plain ints.
        unsafe { ffi::sqlite3_enable_load_extension(db.raw(), opts.allow_extension as c_int) };
        // SAFETY: raw is live.
        unsafe { ffi::sqlite3_busy_timeout(db.raw(), opts.timeout_ms) };
        Ok(db)
    }

    pub(crate) fn raw(&self) -> *mut ffi::sqlite3 {
        self.inner.raw
    }

    fn config(&self, op: c_int, on: bool) -> Result<()> {
        let mut out: c_int = 0;
        // SAFETY: variadic call with the (int onoff, int *out) pair SQLite expects for these ops.
        let rc =
            unsafe { ffi::sqlite3_db_config(self.raw(), op, on as c_int, &mut out as *mut c_int) };
        if rc != ffi::SQLITE_OK {
            return Err(self.error(rc));
        }
        Ok(())
    }

    pub(crate) fn error(&self, rc: i32) -> Error {
        Error::from_handle(self.raw(), rc)
    }

    /// Runs one or more SQL statements with no result rows.
    pub fn exec(&self, sql: &str) -> Result<()> {
        let c =
            CString::new(sql).map_err(|_| Error::local(ffi::SQLITE_ERROR, "sql contains NUL"))?;
        // SAFETY: c is NUL-terminated; no callback, no error-out string.
        let rc = unsafe {
            ffi::sqlite3_exec(
                self.raw(),
                c.as_ptr(),
                None,
                ptr::null_mut(),
                ptr::null_mut(),
            )
        };
        if rc != ffi::SQLITE_OK {
            return Err(self.error(rc));
        }
        Ok(())
    }

    /// Prepares the first statement in `sql`; trailing statements are ignored.
    /// Whitespace-only or comment-only SQL yields a statement that steps to done.
    pub fn prepare(&self, sql: &str) -> Result<Stmt> {
        let c =
            CString::new(sql).map_err(|_| Error::local(ffi::SQLITE_ERROR, "sql contains NUL"))?;
        let mut raw = ptr::null_mut();
        // SAFETY: c is NUL-terminated; raw is a valid out slot; tail is unused.
        let rc = unsafe {
            ffi::sqlite3_prepare_v3(self.raw(), c.as_ptr(), -1, 0, &mut raw, ptr::null_mut())
        };
        if rc != ffi::SQLITE_OK {
            return Err(self.error(rc));
        }
        Ok(Stmt {
            raw,
            db: Rc::clone(&self.inner),
        })
    }

    pub fn changes(&self) -> i32 {
        // SAFETY: raw is live.
        unsafe { ffi::sqlite3_changes(self.raw()) }
    }

    pub fn total_changes(&self) -> i32 {
        // SAFETY: raw is live.
        unsafe { ffi::sqlite3_total_changes(self.raw()) }
    }

    pub fn last_insert_rowid(&self) -> i64 {
        // SAFETY: raw is live.
        unsafe { ffi::sqlite3_last_insert_rowid(self.raw()) }
    }

    pub fn in_transaction(&self) -> bool {
        // SAFETY: raw is live.
        unsafe { ffi::sqlite3_get_autocommit(self.raw()) == 0 }
    }

    /// Loads an extension; needs `allow_extension` at open time.
    pub fn load_extension(&self, file: &str, entry: Option<&str>) -> Result<()> {
        let f =
            CString::new(file).map_err(|_| Error::local(ffi::SQLITE_ERROR, "path contains NUL"))?;
        let e = entry
            .map(CString::new)
            .transpose()
            .map_err(|_| Error::local(ffi::SQLITE_ERROR, "entry contains NUL"))?;
        let entry_ptr = e.as_ref().map_or(ptr::null(), |e| e.as_ptr());
        // SAFETY: all strings are NUL-terminated and alive; no error-out string requested.
        let rc = unsafe {
            ffi::sqlite3_load_extension(self.raw(), f.as_ptr(), entry_ptr, ptr::null_mut())
        };
        if rc != ffi::SQLITE_OK {
            return Err(self.error(rc));
        }
        Ok(())
    }
}
