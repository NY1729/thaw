use crate::ffi;
use crate::{Db, Error, Result};

/// Copies all of `src`'s "main" database into `dst`'s "main", in one step.
/// Returns the page count copied.
// ponytail: single synchronous step, no BUSY/LOCKED retry; add a retry loop
// (with rate/progress) when node:sqlite backup() parity lands in PR4.
pub fn backup(src: &Db, dst: &Db) -> Result<i32> {
    let main = c"main".as_ptr();
    // SAFETY: both handles are live; "main" is NUL-terminated and outlives the call.
    let b = unsafe { ffi::sqlite3_backup_init(dst.raw(), main, src.raw(), main) };
    if b.is_null() {
        return Err(Error::from_handle(dst.raw(), ffi::SQLITE_ERROR));
    }
    // SAFETY: b is a live backup handle; -1 copies every remaining page.
    let step_rc = unsafe { ffi::sqlite3_backup_step(b, -1) };
    // SAFETY: b is live; the page count is valid after a step.
    let pages = unsafe { ffi::sqlite3_backup_pagecount(b) };
    // SAFETY: b is finished exactly once; finish reports the step's error too.
    let fin_rc = unsafe { ffi::sqlite3_backup_finish(b) };
    let rc = if step_rc == ffi::SQLITE_DONE {
        fin_rc
    } else {
        step_rc
    };
    if rc != ffi::SQLITE_OK {
        return Err(Error::from_handle(dst.raw(), rc));
    }
    Ok(pages)
}
