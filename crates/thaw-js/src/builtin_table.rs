//! The Node 22 builtin descriptor table applied to a freshly built realm. `builtin_table_embed.tsv` is
//! the rows thaw-js can use (`tools/parity/filter_builtin_table.js` filters `builtin_table.tsv`, which
//! `tools/parity/gen_builtin_table.js` generates under Node 22). A row is one own property of a builtin
//! object: its attributes, and for a function its real `length` and `name`. An alias row (one object at
//! two paths) makes both slots hold one object again. Rows whose owner or key thaw-js lacks are skipped.

use crate::{find_own, Interp, Kind, Obj, Prop, Slot, Value};
use std::collections::HashMap;
use std::rc::Rc;

/// The rows applied at startup.
pub(crate) const EMBED: &str = include_str!("builtin_table_embed.tsv");

/// One row of the table, parsed. Cells are in the order of the generator's header.
pub(crate) struct Row<'a> {
    pub owner: &'a str,
    pub key: &'a str,
    pub accessor: bool,
    pub writable: bool,
    pub enumerable: bool,
    pub configurable: bool,
    /// `typeof` of the value (`-` for an accessor).
    pub vtype: &'a str,
    /// A function's `length`.
    pub length: Option<f64>,
    /// A function's `name`; an accessor's is `get x|set x`, either half `-` when absent.
    pub name: &'a str,
    /// A function's own `prototype` (Node has one or not).
    pub has_prototype: Option<bool>,
    pub alias: Option<u32>,
    /// A `Symbol.toStringTag` string value.
    pub tag: Option<&'a str>,
}

pub(crate) fn parse_rows(text: &str) -> Vec<Row<'_>> {
    text.lines()
        .skip(1)
        .filter(|l| !l.is_empty())
        .map(|line| {
            let c: [&str; 10] = line
                .split('\t')
                .collect::<Vec<_>>()
                .try_into()
                .unwrap_or_else(|_| panic!("builtin table row has 10 cells: {line}"));
            let flags = c[3].as_bytes();
            Row {
                owner: c[0],
                key: c[1],
                accessor: c[2] == "accessor",
                writable: flags[0] == b'w',
                enumerable: flags[1] == b'e',
                configurable: flags[2] == b'c',
                vtype: c[4],
                length: dash(c[5]).map(|n| n.parse().expect("numeric length")),
                name: c[6],
                has_prototype: match c[7] {
                    "1" => Some(true),
                    "0" => Some(false),
                    _ => None,
                },
                alias: dash(c[8]).map(|n| n.parse().expect("numeric alias")),
                tag: dash(c[9]),
            }
        })
        .collect()
}

/// A table cell, or `None` for the `-` placeholder.
fn dash(cell: &str) -> Option<&str> {
    (cell != "-").then_some(cell)
}

/// Owner path -> the object it names (`None` when thaw-js lacks it). Memoised per application.
type Owners = HashMap<String, Option<Obj>>;

/// Set a function's own `length` and `name` as the first two properties, as V8 orders them, both
/// non-writable, non-enumerable, configurable. Removes any previous ones.
fn set_fn_head(f: &Obj, name: &str, length: f64) {
    f.remove_prop("length");
    f.remove_prop("name");
    let mut props = f.raw_props.borrow_mut();
    let head = |key: &str, v: Value| Prop {
        key: key.into(),
        slot: Slot::Data(v),
        enumerable: false,
        writable: false,
        configurable: true,
    };
    props.insert(0, head("name", Value::str(name)));
    props.insert(0, head("length", Value::Number(length)));
    drop(props);
    *f.index.borrow_mut() = None;
}

/// Owner path -> (its own `length`, its own `name`), from the `length`/`name` rows of the table.
fn own_heads<'a>(rows: &[Row<'a>]) -> HashMap<&'a str, (Option<f64>, Option<&'a str>)> {
    let mut heads: HashMap<&str, (Option<f64>, Option<&str>)> = HashMap::new();
    for r in rows {
        if r.key == "length" && r.vtype == "number" {
            heads.entry(r.owner).or_default().0 = r.length;
        }
        if r.key == "name" && r.vtype == "string" {
            heads.entry(r.owner).or_default().1 = Some(r.name);
        }
    }
    heads
}

fn is_function(o: &Obj) -> bool {
    matches!(&*o.kind.borrow(), Kind::Closure { .. } | Kind::Native(_))
}

impl Interp {
    /// Apply the embedded table to this realm. Called last in `build`, after every prelude.
    pub(crate) fn apply_builtin_table(&mut self) {
        let mut owners = Owners::new();
        let mut canon: HashMap<u32, Obj> = HashMap::new();
        let rows = parse_rows(EMBED);
        let heads = own_heads(&rows);
        for row in &rows {
            let Some(owner) = self.table_owner(row.owner, &mut owners) else {
                continue;
            };
            let Some(key) = self.table_key(row.key) else {
                continue;
            };
            // a root builtin function (`setTimeout`, `Array`) has no function row of its own: its
            // `length`/`name` rows carry the values, so they are created here when absent
            if matches!(row.key, "length" | "name")
                && matches!(
                    &*owner.kind.borrow(),
                    Kind::Closure { .. } | Kind::Native(_)
                )
                && find_own(&owner, "length").is_none()
                && find_own(&owner, "name").is_none()
            {
                if let Some(&(Some(length), Some(name))) = heads.get(row.owner) {
                    set_fn_head(&owner, name, length);
                }
            }
            if !self.table_row_present(&owner, &key, row) {
                continue;
            }
            owner.force_key(&key);
            let Some(pos) = owner.position(&key) else {
                continue;
            };
            let slot = owner.raw_props.borrow()[pos].slot.clone();
            {
                let mut props = owner.raw_props.borrow_mut();
                let p = &mut props[pos];
                if !row.accessor {
                    p.writable = row.writable;
                }
                p.enumerable = row.enumerable;
                p.configurable = row.configurable;
            }
            match slot {
                Slot::Accessor { get, set } => {
                    let (g, s) = row.name.split_once('|').unwrap_or((row.name, "-"));
                    for (f, name, len) in [(get, g, 0.0), (set, s, 1.0)] {
                        if let (Some(Value::Object(f)), true) = (f, name != "-") {
                            if is_function(&f) {
                                set_fn_head(&f, name, len);
                            }
                        }
                    }
                }
                Slot::Data(Value::Object(o)) => {
                    // A later path to an object already seen is the same object: take the first path's.
                    if let Some(first) = row.alias.and_then(|id| canon.get(&id)) {
                        owner.raw_props.borrow_mut()[pos].slot =
                            Slot::Data(Value::Object(first.clone()));
                        continue;
                    }
                    if let Some(id) = row.alias {
                        canon.insert(id, o.clone());
                    }
                    if is_function(&o) {
                        o.native_text.set(true);
                        let length = row.length.unwrap_or(0.0);
                        set_fn_head(&o, row.name, length);
                        if row.has_prototype == Some(false) {
                            o.remove_prop("prototype");
                        }
                    }
                }
                Slot::Data(_) => {
                    if let (Some(tag), "@@toStringTag") = (row.tag, row.key) {
                        owner.raw_props.borrow_mut()[pos].slot = Slot::Data(Value::str(tag));
                    }
                }
                Slot::LazyProto { .. } => {}
            }
        }
    }

    /// The object at owner path `path` (`Array.prototype`, `%TypedArray%`, `process.env`), or `None`.
    pub(crate) fn table_owner(&self, path: &str, owners: &mut Owners) -> Option<Obj> {
        if let Some(hit) = owners.get(path) {
            return hit.clone();
        }
        let found = if !path.contains('.') {
            self.table_root(path)
        } else {
            let mut found = None;
            for (i, _) in path.match_indices('.') {
                let Some(parent) = self.table_owner(&path[..i], owners) else {
                    continue;
                };
                let Some(key) = self.table_key(&path[i + 1..]) else {
                    continue;
                };
                if let Some((Slot::Data(Value::Object(o)), _)) = find_own(&parent, &key) {
                    found = Some(o);
                    break;
                }
            }
            found
        };
        owners.insert(path.to_string(), found.clone());
        found
    }

    /// A root of the table: a global binding, or the one intrinsic thaw-js has a name for.
    fn table_root(&self, name: &str) -> Option<Obj> {
        if name == "%TypedArray%" {
            let Some(Value::Object(u)) = self.lookup(&self.global, "Uint8Array") else {
                return None;
            };
            return u.proto.borrow().clone();
        }
        if name.starts_with('%') {
            return None;
        }
        match self.lookup(&self.global, name) {
            Some(Value::Object(o)) => Some(o),
            _ => None,
        }
    }

    /// The thaw-js property key of a table key: `@@x` is the well-known symbol `Symbol.x`.
    pub(crate) fn table_key(&self, key: &str) -> Option<Rc<str>> {
        let Some(name) = key.strip_prefix("@@") else {
            return Some(key.into());
        };
        let Some(Value::Object(symbol)) = self.lookup(&self.global, "Symbol") else {
            return None;
        };
        match find_own(&symbol, name) {
            Some((Slot::Data(Value::Object(s)), _)) => match &*s.kind.borrow() {
                Kind::Symbol { id, .. } => Some(format!("@@sym:{id}").into()),
                _ => None,
            },
            _ => None,
        }
    }

    /// True when `owner` has own property `key` of the row's kind (data or accessor).
    pub(crate) fn table_row_present(&self, owner: &Obj, key: &str, row: &Row<'_>) -> bool {
        match find_own(owner, key) {
            Some((Slot::Accessor { .. }, _)) => row.accessor,
            // a function row needs a function (`Error.prepareStackTrace` is a host value, not one)
            Some((Slot::Data(Value::Object(o)), _)) => {
                !row.accessor && (row.vtype != "function" || is_function(&o))
            }
            Some((Slot::Data(_), _)) => !row.accessor && row.vtype != "function",
            _ => false,
        }
    }
}
