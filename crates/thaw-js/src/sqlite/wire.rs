//! JSON wire between the `__thaw_sqlite_*` natives and the node:sqlite shim.
//!
//! Cell (a result value, or a bound value):
//! - `null`, a number (REAL; INTEGER results too when in the safe range and not in bigint mode), a string,
//! - `{"i":"<decimal>"}` for an integer that is bigint-mode or outside +-(2^53-1) (decoded as a BigInt),
//! - `{"b":"<hex>"}` for a blob, `{"f":"inf"|"-inf"}` for a non-finite real.
//!
//! Row: JSON array of cells (`null` from step/get means no row). Params: JSON array of `[key, cell]`,
//! key = 1-based index (number) or parameter name as written (string).

use std::fmt::Write as _;

use thaw_sqlite::{ColumnMeta, Stmt, Value};

use crate::host::Native;
use crate::json::parse_native;
use crate::zlib::{hex_decode, hex_encode};

/// Largest integer a JS number holds exactly.
const MAX_SAFE: i64 = (1 << 53) - 1;

/// One bound parameter key: positional index or name as the caller wrote it.
#[derive(Debug, PartialEq)]
pub(crate) enum Key {
    Index(i32),
    Name(String),
}

pub(crate) fn encode_cell(v: &Value, big_ints: bool, out: &mut String) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Int(i) if big_ints || !(-MAX_SAFE..=MAX_SAFE).contains(i) => {
            let _ = write!(out, "{{\"i\":\"{i}\"}}");
        }
        Value::Int(i) => {
            let _ = write!(out, "{i}");
        }
        Value::Real(f) if f.is_nan() => out.push_str("null"),
        Value::Real(f) if f.is_finite() => {
            let _ = write!(out, "{f:?}");
        }
        Value::Real(f) if *f > 0.0 => out.push_str("{\"f\":\"inf\"}"),
        Value::Real(_) => out.push_str("{\"f\":\"-inf\"}"),
        Value::Text(s) => crate::builtins::json_quote(s, out),
        Value::Blob(b) => {
            let _ = write!(out, "{{\"b\":\"{}\"}}", hex_encode(b));
        }
    }
}

/// The current row of `stmt` as a JSON array of cells.
pub(crate) fn encode_row(stmt: &Stmt, big_ints: bool) -> String {
    let mut out = String::from("[");
    for i in 0..stmt.column_count() {
        if i > 0 {
            out.push(',');
        }
        encode_cell(&stmt.column(i), big_ints, &mut out);
    }
    out.push(']');
    out
}

/// `stmt.columns()` as a JSON array of `{column, database, name, table, type}`.
pub(crate) fn encode_columns(stmt: &Stmt) -> String {
    fn opt(s: &Option<String>, out: &mut String) {
        match s {
            Some(s) => crate::builtins::json_quote(s, out),
            None => out.push_str("null"),
        }
    }
    let mut out = String::from("[");
    for i in 0..stmt.column_count() {
        if i > 0 {
            out.push(',');
        }
        let ColumnMeta {
            database,
            table,
            origin,
            decltype,
        } = stmt.column_meta(i);
        out.push_str("{\"column\":");
        opt(&origin, &mut out);
        out.push_str(",\"database\":");
        opt(&database, &mut out);
        out.push_str(",\"name\":");
        crate::builtins::json_quote(&stmt.column_name(i), &mut out);
        out.push_str(",\"table\":");
        opt(&table, &mut out);
        out.push_str(",\"type\":");
        opt(&decltype, &mut out);
        out.push('}');
    }
    out.push(']');
    out
}

/// A bound value from its cell. Numbers always bind as REAL, as node:sqlite does.
fn decode_cell(n: &Native) -> Result<Value, String> {
    match n {
        Native::Null | Native::Undefined => Ok(Value::Null),
        Native::Number(f) => Ok(Value::Real(*f)),
        Native::Str(s) => Ok(Value::Text(s.clone())),
        Native::Object(pairs) => {
            let field = |k: &str| {
                pairs.iter().find_map(|(key, v)| match v {
                    Native::Str(s) if key == k => Some(s.as_str()),
                    _ => None,
                })
            };
            if let Some(i) = field("i") {
                return i
                    .parse::<i64>()
                    .map(Value::Int)
                    .map_err(|_| "BigInt value is too large to bind.".to_string());
            }
            if let Some(b) = field("b") {
                return Ok(Value::Blob(hex_decode(b)));
            }
            match field("f") {
                Some("inf") => Ok(Value::Real(f64::INFINITY)),
                Some("-inf") => Ok(Value::Real(f64::NEG_INFINITY)),
                _ => Err("Provided value cannot be bound to SQLite parameter.".to_string()),
            }
        }
        _ => Err("Provided value cannot be bound to SQLite parameter.".to_string()),
    }
}

/// Decodes a params wire (empty text = no parameters).
pub(crate) fn decode_params(json: &str) -> Result<Vec<(Key, Value)>, String> {
    if json.is_empty() {
        return Ok(Vec::new());
    }
    let Native::Array(pairs) = parse_native(json)? else {
        return Err("params must be an array".to_string());
    };
    pairs
        .iter()
        .map(|pair| match pair {
            Native::Array(kv) if kv.len() == 2 => {
                let key = match &kv[0] {
                    Native::Number(n) => Key::Index(*n as i32),
                    Native::Str(s) => Key::Name(s.clone()),
                    _ => return Err("parameter key must be a number or string".to_string()),
                };
                Ok((key, decode_cell(&kv[1])?))
            }
            _ => Err("parameter must be a [key, value] pair".to_string()),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integers_are_tagged_only_when_bigint_or_unsafe() {
        let mut out = String::new();
        encode_cell(&Value::Int(42), false, &mut out);
        out.push(' ');
        encode_cell(&Value::Int(42), true, &mut out);
        out.push(' ');
        encode_cell(&Value::Int(9007199254740993), false, &mut out);
        assert_eq!(out, r#"42 {"i":"42"} {"i":"9007199254740993"}"#);
    }

    #[test]
    fn non_finite_and_blob_cells() {
        let mut out = String::new();
        encode_cell(&Value::Real(f64::NEG_INFINITY), false, &mut out);
        out.push(' ');
        encode_cell(&Value::Blob(vec![0xde, 0xad]), false, &mut out);
        out.push(' ');
        encode_cell(&Value::Real(1.5), false, &mut out);
        assert_eq!(out, r#"{"f":"-inf"} {"b":"dead"} 1.5"#);
    }

    #[test]
    fn params_decode_keys_and_tagged_values() {
        let got = decode_params(r#"[[1,"x"],["id",{"i":"-7"}],[3,{"b":"0102"}],[4,{"f":"inf"}]]"#)
            .expect("valid params");
        assert_eq!(
            got,
            vec![
                (Key::Index(1), Value::Text("x".into())),
                (Key::Name("id".into()), Value::Int(-7)),
                (Key::Index(3), Value::Blob(vec![1, 2])),
                (Key::Index(4), Value::Real(f64::INFINITY)),
            ]
        );
        assert!(decode_params(r#"[[1,true]]"#).is_err());
        assert!(decode_params("").expect("empty").is_empty());
    }
}
