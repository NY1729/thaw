//! Hand-written declarations for the subset of the SQLite C API this crate
//! uses. Signatures follow vendor/sqlite3.h (3.51.3).
// Declares the full PR1 subset up front; UDF callbacks, backup progress and
// loadExtension callers land in later PRs, so their items are unused for now.
#![allow(non_camel_case_types, dead_code)]

use std::ffi::{c_char, c_double, c_int, c_void};

pub enum sqlite3 {}
pub enum sqlite3_stmt {}
pub enum sqlite3_context {}
pub enum sqlite3_value {}
pub enum sqlite3_backup {}
pub enum sqlite3_session {}
pub enum sqlite3_changeset_iter {}

pub const SQLITE_OK: c_int = 0;
pub const SQLITE_ERROR: c_int = 1;
pub const SQLITE_ABORT: c_int = 4;
pub const SQLITE_RANGE: c_int = 25;
pub const SQLITE_ROW: c_int = 100;
pub const SQLITE_DONE: c_int = 101;

pub const SQLITE_INTEGER: c_int = 1;
pub const SQLITE_FLOAT: c_int = 2;
pub const SQLITE_TEXT: c_int = 3;
pub const SQLITE_BLOB: c_int = 4;
pub const SQLITE_NULL: c_int = 5;

pub const SQLITE_OPEN_READONLY: c_int = 0x0000_0001;
pub const SQLITE_OPEN_READWRITE: c_int = 0x0000_0002;
pub const SQLITE_OPEN_CREATE: c_int = 0x0000_0004;

pub const SQLITE_UTF8: c_int = 1;
pub const SQLITE_DETERMINISTIC: c_int = 0x0000_0800;
pub const SQLITE_DIRECTONLY: c_int = 0x0008_0000;

pub const SQLITE_DBCONFIG_ENABLE_FKEY: c_int = 1002;
pub const SQLITE_DBCONFIG_DQS_DML: c_int = 1013;
pub const SQLITE_DBCONFIG_DQS_DDL: c_int = 1014;

pub const SQLITE_CHANGESET_ABORT: c_int = 2;

/// Callback type for `sqlite3_create_function_v2` and friends.
pub type XFunc = unsafe extern "C" fn(*mut sqlite3_context, c_int, *mut *mut sqlite3_value);
/// Callback type for aggregate `xStep` / `xFinal`.
pub type XStep = unsafe extern "C" fn(*mut sqlite3_context, c_int, *mut *mut sqlite3_value);
pub type XFinal = unsafe extern "C" fn(*mut sqlite3_context);
pub type XDestroy = unsafe extern "C" fn(*mut c_void);
pub type XExec =
    unsafe extern "C" fn(*mut c_void, c_int, *mut *mut c_char, *mut *mut c_char) -> c_int;
pub type XFilter = unsafe extern "C" fn(*mut c_void, *const c_char) -> c_int;
pub type XConflict = unsafe extern "C" fn(*mut c_void, c_int, *mut sqlite3_changeset_iter) -> c_int;

extern "C" {
    pub fn sqlite3_open_v2(
        filename: *const c_char,
        pp: *mut *mut sqlite3,
        flags: c_int,
        vfs: *const c_char,
    ) -> c_int;
    pub fn sqlite3_close_v2(db: *mut sqlite3) -> c_int;
    pub fn sqlite3_db_config(db: *mut sqlite3, op: c_int, ...) -> c_int;
    pub fn sqlite3_busy_timeout(db: *mut sqlite3, ms: c_int) -> c_int;
    pub fn sqlite3_enable_load_extension(db: *mut sqlite3, onoff: c_int) -> c_int;
    pub fn sqlite3_load_extension(
        db: *mut sqlite3,
        file: *const c_char,
        proc_: *const c_char,
        err: *mut *mut c_char,
    ) -> c_int;
    pub fn sqlite3_exec(
        db: *mut sqlite3,
        sql: *const c_char,
        cb: Option<XExec>,
        arg: *mut c_void,
        err: *mut *mut c_char,
    ) -> c_int;
    pub fn sqlite3_errmsg(db: *mut sqlite3) -> *const c_char;
    pub fn sqlite3_errcode(db: *mut sqlite3) -> c_int;
    pub fn sqlite3_extended_errcode(db: *mut sqlite3) -> c_int;
    pub fn sqlite3_errstr(rc: c_int) -> *const c_char;
    pub fn sqlite3_changes(db: *mut sqlite3) -> c_int;
    pub fn sqlite3_total_changes(db: *mut sqlite3) -> c_int;
    pub fn sqlite3_last_insert_rowid(db: *mut sqlite3) -> i64;
    pub fn sqlite3_get_autocommit(db: *mut sqlite3) -> c_int;
    pub fn sqlite3_free(p: *mut c_void);

    pub fn sqlite3_prepare_v3(
        db: *mut sqlite3,
        sql: *const c_char,
        n: c_int,
        flags: u32,
        pp: *mut *mut sqlite3_stmt,
        tail: *mut *const c_char,
    ) -> c_int;
    pub fn sqlite3_step(stmt: *mut sqlite3_stmt) -> c_int;
    pub fn sqlite3_reset(stmt: *mut sqlite3_stmt) -> c_int;
    pub fn sqlite3_finalize(stmt: *mut sqlite3_stmt) -> c_int;
    pub fn sqlite3_stmt_readonly(stmt: *mut sqlite3_stmt) -> c_int;
    pub fn sqlite3_sql(stmt: *mut sqlite3_stmt) -> *const c_char;
    pub fn sqlite3_expanded_sql(stmt: *mut sqlite3_stmt) -> *mut c_char;
    pub fn sqlite3_bind_parameter_index(stmt: *mut sqlite3_stmt, name: *const c_char) -> c_int;
    pub fn sqlite3_bind_null(stmt: *mut sqlite3_stmt, i: c_int) -> c_int;
    pub fn sqlite3_bind_int64(stmt: *mut sqlite3_stmt, i: c_int, v: i64) -> c_int;
    pub fn sqlite3_bind_double(stmt: *mut sqlite3_stmt, i: c_int, v: c_double) -> c_int;
    /// `destructor` is `SQLITE_TRANSIENT` (-1) or `SQLITE_STATIC`/a free fn.
    pub fn sqlite3_bind_text(
        stmt: *mut sqlite3_stmt,
        i: c_int,
        p: *const c_char,
        n: c_int,
        destructor: *const c_void,
    ) -> c_int;
    pub fn sqlite3_bind_blob(
        stmt: *mut sqlite3_stmt,
        i: c_int,
        p: *const c_void,
        n: c_int,
        destructor: *const c_void,
    ) -> c_int;
    pub fn sqlite3_column_count(stmt: *mut sqlite3_stmt) -> c_int;
    pub fn sqlite3_column_name(stmt: *mut sqlite3_stmt, i: c_int) -> *const c_char;
    pub fn sqlite3_column_type(stmt: *mut sqlite3_stmt, i: c_int) -> c_int;
    pub fn sqlite3_column_int64(stmt: *mut sqlite3_stmt, i: c_int) -> i64;
    pub fn sqlite3_column_double(stmt: *mut sqlite3_stmt, i: c_int) -> c_double;
    pub fn sqlite3_column_text(stmt: *mut sqlite3_stmt, i: c_int) -> *const u8;
    pub fn sqlite3_column_blob(stmt: *mut sqlite3_stmt, i: c_int) -> *const c_void;
    pub fn sqlite3_column_bytes(stmt: *mut sqlite3_stmt, i: c_int) -> c_int;

    pub fn sqlite3_create_function_v2(
        db: *mut sqlite3,
        name: *const c_char,
        n_arg: c_int,
        enc: c_int,
        app: *mut c_void,
        x_func: Option<XFunc>,
        x_step: Option<XStep>,
        x_final: Option<XFinal>,
        x_destroy: Option<XDestroy>,
    ) -> c_int;
    pub fn sqlite3_aggregate_context(ctx: *mut sqlite3_context, n: c_int) -> *mut c_void;
    pub fn sqlite3_user_data(ctx: *mut sqlite3_context) -> *mut c_void;
    pub fn sqlite3_value_type(v: *mut sqlite3_value) -> c_int;
    pub fn sqlite3_value_int64(v: *mut sqlite3_value) -> i64;
    pub fn sqlite3_value_double(v: *mut sqlite3_value) -> c_double;
    pub fn sqlite3_value_text(v: *mut sqlite3_value) -> *const u8;
    pub fn sqlite3_value_blob(v: *mut sqlite3_value) -> *const c_void;
    pub fn sqlite3_value_bytes(v: *mut sqlite3_value) -> c_int;
    pub fn sqlite3_result_null(ctx: *mut sqlite3_context);
    pub fn sqlite3_result_int64(ctx: *mut sqlite3_context, v: i64);
    pub fn sqlite3_result_double(ctx: *mut sqlite3_context, v: c_double);
    pub fn sqlite3_result_text(
        ctx: *mut sqlite3_context,
        p: *const c_char,
        n: c_int,
        destructor: *const c_void,
    );
    pub fn sqlite3_result_blob(
        ctx: *mut sqlite3_context,
        p: *const c_void,
        n: c_int,
        destructor: *const c_void,
    );
    pub fn sqlite3_result_error(ctx: *mut sqlite3_context, p: *const c_char, n: c_int);
    pub fn sqlite3_result_error_code(ctx: *mut sqlite3_context, code: c_int);

    pub fn sqlite3_backup_init(
        dst: *mut sqlite3,
        dst_name: *const c_char,
        src: *mut sqlite3,
        src_name: *const c_char,
    ) -> *mut sqlite3_backup;
    pub fn sqlite3_backup_step(b: *mut sqlite3_backup, pages: c_int) -> c_int;
    pub fn sqlite3_backup_finish(b: *mut sqlite3_backup) -> c_int;
    pub fn sqlite3_backup_remaining(b: *mut sqlite3_backup) -> c_int;
    pub fn sqlite3_backup_pagecount(b: *mut sqlite3_backup) -> c_int;

    pub fn sqlite3session_create(
        db: *mut sqlite3,
        db_name: *const c_char,
        pp: *mut *mut sqlite3_session,
    ) -> c_int;
    pub fn sqlite3session_delete(s: *mut sqlite3_session);
    pub fn sqlite3session_attach(s: *mut sqlite3_session, table: *const c_char) -> c_int;
    pub fn sqlite3session_changeset(
        s: *mut sqlite3_session,
        n: *mut c_int,
        p: *mut *mut c_void,
    ) -> c_int;
    pub fn sqlite3session_patchset(
        s: *mut sqlite3_session,
        n: *mut c_int,
        p: *mut *mut c_void,
    ) -> c_int;
    pub fn sqlite3changeset_apply(
        db: *mut sqlite3,
        n: c_int,
        changeset: *mut c_void,
        x_filter: Option<XFilter>,
        x_conflict: Option<XConflict>,
        ctx: *mut c_void,
    ) -> c_int;
}
