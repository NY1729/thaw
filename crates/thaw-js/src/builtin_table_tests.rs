//! Conformance of the builtin descriptor table (`builtin_table.tsv`, Node 22) against a live realm, and
//! the ratchet of `builtin_table_missing.txt`. No Node needed: the table is the oracle.
//! `BUILTIN_TABLE_WRITE_MISSING=1 cargo test ... builtin_table` rewrites the missing list.
#[cfg(test)]
use super::*;
use crate::builtin_table::{parse_rows, EMBED};
use std::collections::{BTreeSet, HashMap};

const FULL: &str = include_str!("builtin_table.tsv");
const MISSING_FILE: &str = "builtin_table_missing.txt";

fn missing_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join(MISSING_FILE)
}

/// The listed `owner<TAB>key` rows (comments and blank lines skipped).
fn listed_missing() -> BTreeSet<String> {
    std::fs::read_to_string(missing_path())
        .expect("builtin_table_missing.txt is checked in")
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(String::from)
        .collect()
}

fn own(o: &Obj, key: &str) -> Option<Prop> {
    o.force_key(key);
    o.raw_props
        .borrow()
        .iter()
        .find(|p| &*p.key == key)
        .cloned()
}

fn data(p: &Prop) -> Option<Value> {
    match &p.slot {
        Slot::Data(v) => Some(v.clone()),
        _ => None,
    }
}

fn text(v: &Value) -> Option<String> {
    match v {
        Value::Str(s) => Some(s.to_string()),
        _ => None,
    }
}

#[test]
fn builtin_table_descriptors_match_node_and_missing_list_is_exact() {
    let it = Interp::new();
    let mut owners = HashMap::new();
    let mut aliases: HashMap<u32, Obj> = HashMap::new();
    let mut missing = BTreeSet::new();
    let mut failures: Vec<String> = Vec::new();
    for row in parse_rows(FULL) {
        let owner = it.table_owner(row.owner, &mut owners);
        let key = owner.as_ref().and_then(|_| it.table_key(row.key));
        let (Some(o), Some(k)) = (owner, key) else {
            missing.insert(format!("{}\t{}", row.owner, row.key));
            continue;
        };
        if !it.table_row_present(&o, &k, &row) {
            missing.insert(format!("{}\t{}", row.owner, row.key));
            continue;
        }
        let what = format!("{}.{}", row.owner, row.key);
        let Some(p) = own(&o, &k) else { continue };
        if !row.accessor {
            let flags = (p.writable, p.enumerable, p.configurable);
            if flags != (row.writable, row.enumerable, row.configurable) {
                failures.push(format!(
                    "{what}: flags {flags:?}, table {row:?}",
                    row = (row.writable, row.enumerable, row.configurable)
                ));
            }
            if let Some(tag) = row.tag {
                if data(&p).as_ref().and_then(text).as_deref() != Some(tag) {
                    failures.push(format!("{what}: toStringTag is not {tag:?}"));
                }
            }
            if let Some(id) = row.alias {
                if let Some(Value::Object(v)) = data(&p) {
                    match aliases.get(&id) {
                        Some(first) if !Rc::ptr_eq(first, &v) => failures
                            .push(format!("{what}: alias {id} is not the first path's object")),
                        Some(_) => {}
                        None => {
                            aliases.insert(id, v);
                        }
                    }
                }
            }
        } else {
            if (p.enumerable, p.configurable) != (row.enumerable, row.configurable) {
                failures.push(format!("{what}: accessor flags differ"));
            }
        }
        let Some(value) = data(&p) else {
            // accessor: names of its getter and setter
            if let Slot::Accessor { get, set } = &p.slot {
                let (g, s) = row.name.split_once('|').unwrap_or((row.name, "-"));
                for (half, name, len) in [(get, g, 0.0), (set, s, 1.0)] {
                    let (Some(Value::Object(f)), true) = (half, name != "-") else {
                        continue;
                    };
                    let got_name = own(f, "name")
                        .as_ref()
                        .and_then(data)
                        .as_ref()
                        .and_then(text);
                    let got_len = own(f, "length").as_ref().and_then(data);
                    if got_name.as_deref() != Some(name)
                        || !matches!(got_len, Some(Value::Number(n)) if n == len)
                    {
                        failures.push(format!(
                            "{what}: accessor half {name:?} has name {got_name:?}"
                        ));
                    }
                }
            }
            continue;
        };
        if row.vtype == "function" {
            let Value::Object(f) = value else {
                failures.push(format!(
                    "{what}: table says function, thaw holds a non-object"
                ));
                continue;
            };
            let len = own(&f, "length")
                .as_ref()
                .and_then(data)
                .and_then(|v| match v {
                    Value::Number(n) => Some(n),
                    _ => None,
                });
            if len != row.length {
                failures.push(format!("{what}: length {len:?}, table {:?}", row.length));
            }
            let name = own(&f, "name")
                .as_ref()
                .and_then(data)
                .as_ref()
                .and_then(text);
            if name.as_deref() != Some(row.name) {
                failures.push(format!("{what}: name {name:?}, table {:?}", row.name));
            }
            // Node has no `prototype` here: thaw must not have one either. The converse (Node has one, thaw
            // does not) is a missing `prototype` row of this function, listed in builtin_table_missing.txt.
            if row.has_prototype == Some(false) && own(&f, "prototype").is_some() {
                failures.push(format!("{what}: has an own prototype, Node has none"));
            }
        }
    }
    if std::env::var_os("BUILTIN_TABLE_WRITE_MISSING").is_some() {
        let header: String = std::fs::read_to_string(missing_path())
            .unwrap_or_default()
            .lines()
            .filter(|l| l.starts_with('#'))
            .map(|l| format!("{l}\n"))
            .collect();
        let body: String = missing.iter().map(|l| format!("{l}\n")).collect();
        std::fs::write(missing_path(), header + &body).expect("write missing list");
        eprintln!("wrote {} missing rows", missing.len());
    } else {
        let listed = listed_missing();
        for absent in missing.difference(&listed) {
            failures.push(format!("absent row not listed: {absent:?}"));
        }
        for present in listed.difference(&missing) {
            failures.push(format!("listed row is present: {present:?}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} table mismatches, first 40:\n{}",
        failures.len(),
        failures
            .iter()
            .take(40)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// The embed is the full table minus the missing rows, in table order (`filter_builtin_table.js`).
#[test]
fn embed_is_the_full_table_minus_the_missing_rows() {
    let missing = listed_missing();
    let row_key = |l: &str| l.split('\t').take(2).collect::<Vec<_>>().join("\t");
    let want: Vec<&str> = FULL
        .lines()
        .skip(1)
        .filter(|l| !l.is_empty() && !missing.contains(&row_key(l)))
        .collect();
    let got: Vec<&str> = EMBED.lines().skip(1).filter(|l| !l.is_empty()).collect();
    assert!(
        got == want,
        "builtin_table_embed.tsv is stale: rerun node tools/parity/filter_builtin_table.js ({} rows, want {})",
        got.len(),
        want.len()
    );
}

/// Name, length and attribute values that the table and Node both define, through JS.
#[test]
fn builtin_names_lengths_and_native_text_match_node() {
    let mut it = Interp::new();
    it.run(
        "const out = [];\
         const b = Math.max.bind(null, 1);\
         out.push(b.name, b.length);\
         out.push(Object.getOwnPropertyDescriptor(Map.prototype, 'size').get.name);\
         out.push(Array.prototype[Symbol.iterator].name, Array.prototype[Symbol.iterator] === Array.prototype.values);\
         out.push(Map.prototype[Symbol.iterator].name);\
         out.push(Object.hasOwn.toString());\
         out.push(Object.getOwnPropertyNames(Array.from).join(','));\
         out.push(JSON.stringify(Object.getOwnPropertyDescriptor(Math.max, 'name')));\
         out.push(JSON.stringify(Object.getOwnPropertyDescriptor(Math.max, 'length')));\
         console.log(out.join('\\n'));",
    )
    .expect("probe runs");
    let expected = "bound max\n1\nget size\nvalues\ntrue\nentries\nfunction hasOwn() { [native code] }\nlength,name\n{\"value\":\"max\",\"writable\":false,\"enumerable\":false,\"configurable\":true}\n{\"value\":2,\"writable\":false,\"enumerable\":false,\"configurable\":true}";
    assert_eq!(it.out.join("\n"), expected);
}

/// Startup cost of `Interp::new()` (run with `--ignored --nocapture`; not a pass/fail check).
#[test]
#[ignore]
fn startup_cost_of_interp_new() {
    let n = 40;
    let t = std::time::Instant::now();
    for _ in 0..n {
        std::hint::black_box(Interp::new());
    }
    eprintln!(
        "startup: Interp::new() avg {:?} over {n} runs",
        t.elapsed() / n
    );
}
