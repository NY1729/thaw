use std::ffi::{c_int, c_void, CString};
use std::rc::Rc;
use std::slice;

use crate::db::Inner;
use crate::ffi;
use crate::{c_len, cstr, Error, Result, Value};

/// SQLITE_TRANSIENT: SQLite copies bound text/blob before the call returns.
/// The C macro is the integer -1 cast to a destructor pointer; only the
/// address value matters to SQLite, so pass it as a plain pointer.
fn transient() -> *const c_void {
    usize::MAX as *const c_void
}

/// A prepared statement. Holds the connection alive; finalized on drop.
pub struct Stmt {
    pub(crate) raw: *mut ffi::sqlite3_stmt,
    pub(crate) db: Rc<Inner>,
}

impl Drop for Stmt {
    fn drop(&mut self) {
        // SAFETY: raw is finalized exactly once here; finalize(NULL) is a no-op.
        unsafe { ffi::sqlite3_finalize(self.raw) };
    }
}

/// Copies `n` bytes at `p`; null or empty yields an empty slice.
///
/// # Safety
/// If non-null and `n > 0`, `p` must be valid for `n` bytes for this call.
unsafe fn bytes<'a>(p: *const u8, n: c_int) -> &'a [u8] {
    if p.is_null() || n <= 0 {
        &[]
    } else {
        slice::from_raw_parts(p, n as usize)
    }
}

impl Stmt {
    fn err(&self, rc: i32) -> Error {
        Error::from_handle(self.db.raw, rc)
    }

    /// Binds a value to a 1-based parameter index.
    pub fn bind(&mut self, index: i32, v: &Value) -> Result<()> {
        if self.raw.is_null() {
            return Err(Error::local(ffi::SQLITE_RANGE, "statement is empty"));
        }
        // SAFETY: raw is a live statement; text/blob pointers are valid for the
        // call and SQLITE_TRANSIENT makes SQLite copy them.
        let rc = unsafe {
            match v {
                Value::Null => ffi::sqlite3_bind_null(self.raw, index),
                Value::Int(i) => ffi::sqlite3_bind_int64(self.raw, index, *i),
                Value::Real(f) => ffi::sqlite3_bind_double(self.raw, index, *f),
                Value::Text(s) => ffi::sqlite3_bind_text(
                    self.raw,
                    index,
                    s.as_ptr().cast(),
                    c_len(s.len())?,
                    transient(),
                ),
                Value::Blob(b) => ffi::sqlite3_bind_blob(
                    self.raw,
                    index,
                    b.as_ptr().cast(),
                    c_len(b.len())?,
                    transient(),
                ),
            }
        };
        if rc != ffi::SQLITE_OK {
            return Err(self.err(rc));
        }
        Ok(())
    }

    /// Binds by exact parameter name including its prefix (`:a`, `$a`, `@a`).
    pub fn bind_name(&mut self, name: &str, v: &Value) -> Result<()> {
        let c = CString::new(name)
            .map_err(|_| Error::local(ffi::SQLITE_RANGE, "unknown parameter name"))?;
        // SAFETY: raw may be null (sqlite returns 0 then); c is NUL-terminated.
        let idx = unsafe { ffi::sqlite3_bind_parameter_index(self.raw, c.as_ptr()) };
        if idx == 0 {
            return Err(Error::local(ffi::SQLITE_RANGE, "unknown parameter name"));
        }
        self.bind(idx, v)
    }

    /// Advances one row. `Ok(true)` = a row is ready, `Ok(false)` = done.
    pub fn step(&mut self) -> Result<bool> {
        if self.raw.is_null() {
            return Ok(false);
        }
        // SAFETY: raw is a live statement.
        match unsafe { ffi::sqlite3_step(self.raw) } {
            ffi::SQLITE_ROW => Ok(true),
            ffi::SQLITE_DONE => Ok(false),
            rc => Err(self.err(rc)),
        }
    }

    /// Resets to the state before the first step (bindings are kept).
    /// Returns the error of the last failed step, if any.
    pub fn reset(&mut self) -> Result<()> {
        if self.raw.is_null() {
            return Ok(());
        }
        // SAFETY: raw is a live statement.
        let rc = unsafe { ffi::sqlite3_reset(self.raw) };
        if rc != ffi::SQLITE_OK {
            return Err(self.err(rc));
        }
        Ok(())
    }

    /// Clears every binding (unbound parameters become NULL again).
    pub fn clear_bindings(&mut self) {
        if !self.raw.is_null() {
            // SAFETY: raw is a live statement.
            unsafe { ffi::sqlite3_clear_bindings(self.raw) };
        }
    }

    /// 1-based index of a named parameter (`:a`, `$a`, `@a`, `?1`), if the statement has it.
    pub fn param_index(&self, name: &str) -> Option<i32> {
        let c = std::ffi::CString::new(name).ok()?;
        // SAFETY: raw may be null (sqlite returns 0); c is NUL-terminated.
        match unsafe { ffi::sqlite3_bind_parameter_index(self.raw, c.as_ptr()) } {
            0 => None,
            i => Some(i),
        }
    }

    /// Origin metadata of column `i` (needs SQLITE_ENABLE_COLUMN_METADATA, built in).
    pub fn column_meta(&self, i: i32) -> ColumnMeta {
        // SAFETY: raw is live; each name is copied out, null becomes None.
        let opt = |p: *const std::ffi::c_char| unsafe {
            if p.is_null() {
                None
            } else {
                Some(cstr(p))
            }
        };
        unsafe {
            ColumnMeta {
                database: opt(ffi::sqlite3_column_database_name(self.raw, i)),
                table: opt(ffi::sqlite3_column_table_name(self.raw, i)),
                origin: opt(ffi::sqlite3_column_origin_name(self.raw, i)),
                decltype: opt(ffi::sqlite3_column_decltype(self.raw, i)),
            }
        }
    }

    pub fn column_count(&self) -> i32 {
        if self.raw.is_null() {
            return 0;
        }
        // SAFETY: raw is a live statement.
        unsafe { ffi::sqlite3_column_count(self.raw) }
    }

    /// Column name (or alias) at 0-based index `i`.
    pub fn column_name(&self, i: i32) -> String {
        // SAFETY: raw is live; the returned name is copied immediately.
        unsafe { cstr(ffi::sqlite3_column_name(self.raw, i)) }
    }

    /// Value of column `i` in the current row (0-based).
    pub fn column(&self, i: i32) -> Value {
        // SAFETY: called only on a live statement after a row was returned.
        // Pointers are read right after their type check and copied out.
        unsafe {
            match ffi::sqlite3_column_type(self.raw, i) {
                ffi::SQLITE_INTEGER => Value::Int(ffi::sqlite3_column_int64(self.raw, i)),
                ffi::SQLITE_FLOAT => Value::Real(ffi::sqlite3_column_double(self.raw, i)),
                ffi::SQLITE_TEXT => {
                    let p = ffi::sqlite3_column_text(self.raw, i);
                    let n = ffi::sqlite3_column_bytes(self.raw, i);
                    Value::Text(String::from_utf8_lossy(bytes(p, n)).into_owned())
                }
                ffi::SQLITE_BLOB => {
                    let p = ffi::sqlite3_column_blob(self.raw, i).cast::<u8>();
                    let n = ffi::sqlite3_column_bytes(self.raw, i);
                    Value::Blob(bytes(p, n).to_vec())
                }
                _ => Value::Null,
            }
        }
    }

    /// The SQL text as given to prepare.
    pub fn sql(&self) -> Option<String> {
        // SAFETY: raw is live; the string is copied immediately.
        let p = unsafe { ffi::sqlite3_sql(self.raw) };
        if p.is_null() {
            None
        } else {
            // SAFETY: non-null, NUL-terminated, valid until the next call on raw.
            Some(unsafe { cstr(p) })
        }
    }

    /// The SQL with bound parameter values substituted.
    pub fn expanded_sql(&self) -> Option<String> {
        // SAFETY: sqlite3_expanded_sql returns a malloc'd copy we copy and free once.
        unsafe {
            let p = ffi::sqlite3_expanded_sql(self.raw);
            if p.is_null() {
                return None;
            }
            let s = cstr(p);
            ffi::sqlite3_free(p.cast());
            Some(s)
        }
    }

    pub fn readonly(&self) -> bool {
        // SAFETY: raw is live.
        unsafe { ffi::sqlite3_stmt_readonly(self.raw) != 0 }
    }
}

/// Where a result column comes from (`sqlite3_column_*_name`); None where SQLite has none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColumnMeta {
    pub database: Option<String>,
    pub table: Option<String>,
    pub origin: Option<String>,
    pub decltype: Option<String>,
}
