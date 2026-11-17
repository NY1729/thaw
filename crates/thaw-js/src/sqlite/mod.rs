//! `node:sqlite` host (cargo feature `sqlite`): the `__thaw_sqlite_*` natives over thaw-sqlite, which the
//! registry's node:sqlite shim calls. Handles are integers in thread-local tables (one realm per thread);
//! databases and statements never cross threads, so the non-Send SQLite handles stay on their thread.

mod db;
#[cfg(test)]
mod sqlite_tests;
mod wire;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use thaw_sqlite::{Db, Stmt};

use crate::*;

/// An open connection and the path it was opened with (`:memory:` / "" have no location).
pub(crate) struct DbEntry {
    db: Db,
    path: String,
}

/// A prepared statement, its database handle, and the per-statement read/bind options.
pub(crate) struct StmtEntry {
    db: u32,
    stmt: Stmt,
    big_ints: bool,
    allow_bare: bool,
    allow_unknown: bool,
}

thread_local! {
    static DBS: RefCell<HashMap<u32, DbEntry>> = RefCell::new(HashMap::new());
    static STMTS: RefCell<HashMap<u32, StmtEntry>> = RefCell::new(HashMap::new());
    static NEXT: Cell<u32> = const { Cell::new(1) };
}

// ponytail: handles live until their database closes or the thread exits; add a GC-driven finalize
// when the shim needs statements released before that.
fn next_id() -> u32 {
    NEXT.with(|n| {
        let id = n.get();
        n.set(id + 1);
        id
    })
}

/// A failed native: SQLite's own error, or a node:sqlite misuse (`ERR_INVALID_STATE`, `ERR_INVALID_ARG_TYPE`).
pub(crate) enum Fail {
    Sqlite(thaw_sqlite::Error),
    State(String),
    Arg(String),
}

impl From<thaw_sqlite::Error> for Fail {
    fn from(e: thaw_sqlite::Error) -> Self {
        Fail::Sqlite(e)
    }
}

pub(crate) type NR<T> = Result<T, Fail>;
type NativeFn = fn(&mut Interp, &[Value]) -> NR<Value>;

/// Throws an `Error` whose `code` is the node:sqlite code; SQLite failures also carry `errcode` and `errstr`.
fn throw<T>(it: &mut Interp, fail: Fail) -> R<T> {
    let (message, props) = match fail {
        Fail::Sqlite(e) => (
            e.message.clone(),
            vec![
                ("code", Value::str("ERR_SQLITE_ERROR")),
                ("errcode", Value::Number(f64::from(e.errcode))),
                ("errstr", Value::str(&e.errstr)),
            ],
        ),
        Fail::State(m) => (m, vec![("code", Value::str("ERR_INVALID_STATE"))]),
        Fail::Arg(m) => (m, vec![("code", Value::str("ERR_INVALID_ARG_TYPE"))]),
    };
    match it.throw_error::<T>("Error", &message) {
        Err(Ctl::Throw(err)) => {
            for (key, value) in props {
                it.set(&err, key, value)?;
            }
            Err(Ctl::Throw(err))
        }
        other => other,
    }
}

const NATIVES: &[(&str, NativeFn)] = &[
    ("__thaw_sqlite_open", db::open),
    ("__thaw_sqlite_close", db::close),
    ("__thaw_sqlite_is_open", db::is_open),
    ("__thaw_sqlite_is_transaction", db::is_transaction),
    ("__thaw_sqlite_location", db::location),
    ("__thaw_sqlite_exec", db::exec),
    ("__thaw_sqlite_prepare", db::prepare),
    ("__thaw_sqlite_stmt_start", db::stmt_start),
    ("__thaw_sqlite_stmt_step", db::stmt_step),
    ("__thaw_sqlite_stmt_reset", db::stmt_reset),
    ("__thaw_sqlite_stmt_run", db::stmt_run),
    ("__thaw_sqlite_stmt_get", db::stmt_get),
    ("__thaw_sqlite_stmt_all", db::stmt_all),
    ("__thaw_sqlite_stmt_columns", db::stmt_columns),
    ("__thaw_sqlite_stmt_source_sql", db::stmt_source_sql),
    ("__thaw_sqlite_stmt_expanded_sql", db::stmt_expanded_sql),
    ("__thaw_sqlite_stmt_set_flag", db::stmt_set_flag),
];

pub(crate) fn install(it: &mut Interp) {
    for (name, f) in NATIVES {
        let f = *f;
        let native = it.native(move |it, _, a| match f(it, a) {
            Ok(v) => Ok(v),
            Err(fail) => throw(it, fail),
        });
        it.global.clone().declare(name, native, true);
    }
}
