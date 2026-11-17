//! `node:sqlite` natives: connections (`__thaw_sqlite_open|close|is_open|is_transaction|location|exec|prepare`)
//! and statements (`__thaw_sqlite_stmt_*`). Results are JSON text (see wire.rs); failures throw through `throw`.
//! Every native takes the handle id first. Bindings are cleared before each run, as node:sqlite does.

use thaw_sqlite::{Db, OpenOptions, OPEN_CREATE, OPEN_READONLY, OPEN_READWRITE};

use super::wire::{decode_params, encode_cell, encode_columns, encode_row, Key};
use super::*;
use crate::host::Native;
use crate::json::parse_native;

/// The options object the shim passes to `open`; absent fields keep node:sqlite's defaults.
fn open_options(json: &str) -> (i32, OpenOptions) {
    let fields = match parse_native(json) {
        Ok(Native::Object(pairs)) => pairs,
        _ => Vec::new(),
    };
    let get = |k: &str| fields.iter().find(|(key, _)| key == k).map(|(_, v)| v);
    let flag = |k: &str, default: bool| match get(k) {
        Some(Native::Bool(b)) => *b,
        _ => default,
    };
    let defaults = OpenOptions::default();
    let opts = OpenOptions {
        foreign_keys: flag("foreignKeys", defaults.foreign_keys),
        double_quoted_strings: flag("doubleQuotedStrings", defaults.double_quoted_strings),
        allow_extension: flag("allowExtension", defaults.allow_extension),
        timeout_ms: match get("timeout") {
            Some(Native::Number(n)) if *n >= 0.0 => *n as i32,
            _ => defaults.timeout_ms,
        },
    };
    let flags = if flag("readOnly", false) {
        OPEN_READONLY
    } else {
        OPEN_READWRITE | OPEN_CREATE
    };
    (flags, opts)
}

/// Handle id argument `i` (a missing or non-number argument is id 0, which no table holds).
fn id(it: &mut Interp, a: &[Value], i: usize) -> u32 {
    it.num(&arg(a, i)) as u32
}

fn text(it: &mut Interp, a: &[Value], i: usize) -> String {
    it.to_str(&arg(a, i)).to_string()
}

/// The JSON text argument `i` (`""` when absent).
fn json_arg(it: &mut Interp, a: &[Value], i: usize) -> String {
    match arg(a, i) {
        Value::Undefined => String::new(),
        v => it.to_str(&v).to_string(),
    }
}

fn with_db<T>(db: u32, f: impl FnOnce(&DbEntry) -> T) -> NR<T> {
    DBS.with(|t| t.borrow().get(&db).map(f))
        .ok_or_else(|| Fail::State("database is not open".into()))
}

fn with_stmt<T>(stmt: u32, f: impl FnOnce(&mut StmtEntry) -> NR<T>) -> NR<T> {
    STMTS.with(|t| match t.borrow_mut().get_mut(&stmt) {
        Some(e) => f(e),
        None => Err(Fail::State("statement has been finalized".into())),
    })
}

/// `$name`, `:name`, `@name` (bare names only when allowed); a name with a prefix matches exactly.
fn resolve_name(stmt: &Stmt, name: &str, allow_bare: bool) -> Option<i32> {
    if name.starts_with(['$', ':', '@', '?']) {
        return stmt.param_index(name);
    }
    if !allow_bare {
        return None;
    }
    ["$", ":", "@"]
        .iter()
        .find_map(|p| stmt.param_index(&format!("{p}{name}")))
}

/// Clears the bindings and binds `params`; the statement is reset first.
fn bind(e: &mut StmtEntry, params: &str) -> NR<()> {
    let _ = e.stmt.reset();
    e.stmt.clear_bindings();
    for (key, value) in decode_params(params).map_err(Fail::Arg)? {
        match key {
            Key::Index(i) => e.stmt.bind(i, &value)?,
            Key::Name(name) => match resolve_name(&e.stmt, &name, e.allow_bare) {
                Some(i) => e.stmt.bind(i, &value)?,
                None if e.allow_unknown => {}
                None => {
                    return Err(Fail::State(format!("Unknown named parameter '{name}'")));
                }
            },
        }
    }
    Ok(())
}

/// Steps the statement and resets it again, whether or not the step failed.
fn step_then_reset<T>(e: &mut StmtEntry, f: impl FnOnce(&mut Stmt) -> NR<T>) -> NR<T> {
    let out = f(&mut e.stmt);
    let _ = e.stmt.reset();
    out
}

pub(super) fn open(it: &mut Interp, a: &[Value]) -> NR<Value> {
    let path = text(it, a, 0);
    let (flags, opts) = open_options(&json_arg(it, a, 1));
    let db = Db::open(&path, flags, &opts)?;
    let handle = next_id();
    DBS.with(|t| t.borrow_mut().insert(handle, DbEntry { db, path }));
    Ok(Value::Number(f64::from(handle)))
}

pub(super) fn close(it: &mut Interp, a: &[Value]) -> NR<Value> {
    let db = id(it, a, 0);
    // statements first: each finalizes before the connection closes (node:sqlite finalizes them on close)
    STMTS.with(|t| t.borrow_mut().retain(|_, e| e.db != db));
    DBS.with(|t| t.borrow_mut().remove(&db));
    Ok(Value::Undefined)
}

pub(super) fn is_open(it: &mut Interp, a: &[Value]) -> NR<Value> {
    let db = id(it, a, 0);
    Ok(Value::Bool(DBS.with(|t| t.borrow().contains_key(&db))))
}

pub(super) fn is_transaction(it: &mut Interp, a: &[Value]) -> NR<Value> {
    let db = id(it, a, 0);
    Ok(Value::Bool(with_db(db, |e| e.db.in_transaction())?))
}

pub(super) fn location(it: &mut Interp, a: &[Value]) -> NR<Value> {
    let db = id(it, a, 0);
    let path = with_db(db, |e| e.path.clone())?;
    if path.is_empty() || path == ":memory:" {
        return Ok(Value::Null);
    }
    let absolute = std::path::absolute(&path).map_or(path, |p| p.to_string_lossy().into_owned());
    Ok(Value::str(&absolute))
}

pub(super) fn exec(it: &mut Interp, a: &[Value]) -> NR<Value> {
    let db = id(it, a, 0);
    let sql = text(it, a, 1);
    with_db(db, |e| e.db.exec(&sql))??;
    Ok(Value::Undefined)
}

pub(super) fn prepare(it: &mut Interp, a: &[Value]) -> NR<Value> {
    let db = id(it, a, 0);
    let sql = text(it, a, 1);
    let stmt = with_db(db, |e| e.db.prepare(&sql))??;
    let handle = next_id();
    let entry = StmtEntry {
        db,
        stmt,
        big_ints: false,
        allow_bare: true,
        allow_unknown: false,
    };
    STMTS.with(|t| t.borrow_mut().insert(handle, entry));
    Ok(Value::Number(f64::from(handle)))
}

pub(super) fn stmt_start(it: &mut Interp, a: &[Value]) -> NR<Value> {
    let stmt = id(it, a, 0);
    let params = json_arg(it, a, 1);
    with_stmt(stmt, |e| bind(e, &params))?;
    Ok(Value::Undefined)
}

pub(super) fn stmt_step(it: &mut Interp, a: &[Value]) -> NR<Value> {
    let stmt = id(it, a, 0);
    with_stmt(stmt, |e| {
        if !e.stmt.step()? {
            return Ok(Value::Null);
        }
        Ok(Value::str(&encode_row(&e.stmt, e.big_ints)))
    })
}

pub(super) fn stmt_reset(it: &mut Interp, a: &[Value]) -> NR<Value> {
    let stmt = id(it, a, 0);
    with_stmt(stmt, |e| {
        e.stmt.reset()?;
        Ok(Value::Undefined)
    })
}

/// `run`: binds, steps to the end, returns `{"changes":cell,"lastInsertRowid":cell}` (JSON text).
pub(super) fn stmt_run(it: &mut Interp, a: &[Value]) -> NR<Value> {
    let stmt = id(it, a, 0);
    let params = json_arg(it, a, 1);
    with_stmt(stmt, |e| {
        bind(e, &params)?;
        step_then_reset(e, |s| {
            while s.step()? {}
            Ok(())
        })?;
        let db = e.db;
        let (changes, rowid) = with_db(db, |d| (d.db.changes(), d.db.last_insert_rowid()))?;
        let (mut c, mut r) = (String::new(), String::new());
        encode_cell(
            &thaw_sqlite::Value::Int(i64::from(changes)),
            e.big_ints,
            &mut c,
        );
        encode_cell(&thaw_sqlite::Value::Int(rowid), e.big_ints, &mut r);
        Ok(Value::str(&format!(
            "{{\"changes\":{c},\"lastInsertRowid\":{r}}}"
        )))
    })
}

/// `get`: binds, returns the first row (JSON array) or `null` when there is none.
pub(super) fn stmt_get(it: &mut Interp, a: &[Value]) -> NR<Value> {
    let stmt = id(it, a, 0);
    let params = json_arg(it, a, 1);
    with_stmt(stmt, |e| {
        bind(e, &params)?;
        let big = e.big_ints;
        step_then_reset(e, |s| {
            if !s.step()? {
                return Ok(Value::Null);
            }
            Ok(Value::str(&encode_row(s, big)))
        })
    })
}

/// `all`: binds, returns every row (JSON array of arrays).
pub(super) fn stmt_all(it: &mut Interp, a: &[Value]) -> NR<Value> {
    let stmt = id(it, a, 0);
    let params = json_arg(it, a, 1);
    with_stmt(stmt, |e| {
        bind(e, &params)?;
        let big = e.big_ints;
        step_then_reset(e, |s| {
            let mut out = String::from("[");
            let mut first = true;
            while s.step()? {
                if !first {
                    out.push(',');
                }
                first = false;
                out.push_str(&encode_row(s, big));
            }
            out.push(']');
            Ok(Value::str(&out))
        })
    })
}

pub(super) fn stmt_columns(it: &mut Interp, a: &[Value]) -> NR<Value> {
    let stmt = id(it, a, 0);
    with_stmt(stmt, |e| Ok(Value::str(&encode_columns(&e.stmt))))
}

pub(super) fn stmt_source_sql(it: &mut Interp, a: &[Value]) -> NR<Value> {
    let stmt = id(it, a, 0);
    with_stmt(stmt, |e| {
        Ok(e.stmt.sql().map_or(Value::Null, |s| Value::str(&s)))
    })
}

pub(super) fn stmt_expanded_sql(it: &mut Interp, a: &[Value]) -> NR<Value> {
    let stmt = id(it, a, 0);
    with_stmt(stmt, |e| {
        Ok(e.stmt
            .expanded_sql()
            .map_or(Value::Null, |s| Value::str(&s)))
    })
}

/// `flag` is `readBigInts`, `allowBareNamedParameters` or `allowUnknownNamedParameters`.
pub(super) fn stmt_set_flag(it: &mut Interp, a: &[Value]) -> NR<Value> {
    let stmt = id(it, a, 0);
    let flag = text(it, a, 1);
    let on = arg(a, 2).truthy();
    with_stmt(stmt, |e| {
        match flag.as_str() {
            "readBigInts" => e.big_ints = on,
            "allowBareNamedParameters" => e.allow_bare = on,
            "allowUnknownNamedParameters" => e.allow_unknown = on,
            other => return Err(Fail::Arg(format!("unknown statement flag {other}"))),
        }
        Ok(Value::Undefined)
    })
}
