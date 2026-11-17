use std::ffi::{c_int, c_void};
use std::ptr;
use std::rc::Rc;

use crate::db::{Db, Inner};
use crate::ffi;
use crate::{c_len, Error, Result};

/// A change-tracking session over every table of the "main" database.
/// Holds the connection alive; deleted on drop.
pub struct Session {
    raw: *mut ffi::sqlite3_session,
    _db: Rc<Inner>,
}

impl Drop for Session {
    fn drop(&mut self) {
        // SAFETY: raw is deleted exactly once here.
        unsafe { ffi::sqlite3session_delete(self.raw) };
    }
}

/// Copies and frees a buffer SQLite allocated for a changeset/patchset.
fn take_buffer(rc: i32, n: c_int, p: *mut c_void, db: *mut ffi::sqlite3) -> Result<Vec<u8>> {
    if rc != ffi::SQLITE_OK {
        // SAFETY: p is either null or sqlite-allocated; sqlite3_free accepts null.
        unsafe { ffi::sqlite3_free(p) };
        return Err(Error::from_handle(db, rc));
    }
    let out = if p.is_null() || n <= 0 {
        Vec::new()
    } else {
        // SAFETY: sqlite reports n bytes at p on success.
        unsafe { std::slice::from_raw_parts(p.cast::<u8>(), n as usize) }.to_vec()
    };
    // SAFETY: p was allocated by sqlite3session_changeset/patchset.
    unsafe { ffi::sqlite3_free(p) };
    Ok(out)
}

impl Session {
    pub fn changeset(&self) -> Result<Vec<u8>> {
        let (mut n, mut p) = (0, ptr::null_mut());
        // SAFETY: raw is live; out slots are valid.
        let rc = unsafe { ffi::sqlite3session_changeset(self.raw, &mut n, &mut p) };
        take_buffer(rc, n, p, self._db.raw)
    }

    pub fn patchset(&self) -> Result<Vec<u8>> {
        let (mut n, mut p) = (0, ptr::null_mut());
        // SAFETY: raw is live; out slots are valid.
        let rc = unsafe { ffi::sqlite3session_patchset(self.raw, &mut n, &mut p) };
        take_buffer(rc, n, p, self._db.raw)
    }
}

impl Db {
    /// Starts a session attached to every table of "main".
    pub fn create_session(&self) -> Result<Session> {
        let mut raw = ptr::null_mut();
        // SAFETY: handle is live; "main" is NUL-terminated; raw is a valid out slot.
        let rc = unsafe { ffi::sqlite3session_create(self.raw(), c"main".as_ptr(), &mut raw) };
        if rc != ffi::SQLITE_OK {
            return Err(self.error(rc));
        }
        let s = Session {
            raw,
            _db: Rc::clone(&self.inner),
        };
        // SAFETY: s.raw is a live session; NULL attaches all tables.
        let rc = unsafe { ffi::sqlite3session_attach(s.raw, ptr::null()) };
        if rc != ffi::SQLITE_OK {
            return Err(self.error(rc));
        }
        Ok(s)
    }

    /// Applies a changeset. Conflicts abort (node:sqlite's default); returns
    /// `Ok(false)` when a conflict aborted the apply.
    pub fn apply_changeset(&self, changeset: &[u8]) -> Result<bool> {
        let n = c_len(changeset.len())?;
        // SAFETY: the buffer is valid for n bytes; SQLite only reads it
        // (the C signature takes void* but never writes). xConflict is
        // required by SQLite and always aborts.
        let rc = unsafe {
            ffi::sqlite3changeset_apply(
                self.raw(),
                n,
                changeset.as_ptr() as *mut c_void,
                None,
                Some(conflict_abort),
                ptr::null_mut(),
            )
        };
        match rc {
            ffi::SQLITE_OK => Ok(true),
            ffi::SQLITE_ABORT => Ok(false),
            rc => Err(Error::from_handle(self.raw(), rc)),
        }
    }
}

unsafe extern "C" fn conflict_abort(
    _ctx: *mut c_void,
    _kind: c_int,
    _iter: *mut ffi::sqlite3_changeset_iter,
) -> c_int {
    ffi::SQLITE_CHANGESET_ABORT
}
