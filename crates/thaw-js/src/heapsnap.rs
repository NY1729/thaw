//! The heap snapshot behind `v8.getHeapSnapshot()` and `v8.writeHeapSnapshot()`: Node's `.heapsnapshot` JSON over this
//! thread's registered object graph (`gc_registry`, thread-local, so it runs only on the owning thread).
//!
//! Nodes: a synthetic root (node 0, name "") plus one node per live registered object. Edges: the object-edge
//! enumeration of `gc::obj_edges` (the one the cycle collector uses), plus the bindings of a closure's captured
//! scope (`context` edges). A root edge goes to every object no other registered object references. Counts come
//! from the arrays built here, so they agree with them by construction.
//!
//! The graph is the registry's, not V8's: no hidden, code or string nodes, so node counts are far below Node's
//! (D3). Output is ASCII only (non-ASCII escaped as `\uXXXX`), so byte and character offsets agree for chunking.
// ponytail: the whole text is built in memory before the caller chunks it. Upgrade path: stream the arrays when a
// heap is too big to hold twice.
// ponytail: edges carry no property names (`internal` type, empty name), except context edges. Upgrade path: name
// the data-property edges from `raw_props` keys.

use crate::gc::{obj_edges, Edge};
use crate::{estimate_bytes, gc_registry, Kind, ObjData, Value};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::rc::Rc;

/// Node's `snapshot.meta`, verbatim from Node 22.23.1.
const META: &str = r#"{"node_fields":["type","name","id","self_size","edge_count","trace_node_id","detachedness"],"node_types":[["hidden","array","string","object","code","closure","regexp","number","native","synthetic","concatenated string","sliced string","symbol","bigint","object shape"],"string","number","number","number","number","number"],"edge_fields":["type","name_or_index","to_node"],"edge_types":[["context","element","property","internal","hidden","shortcut","weak"],"string_or_number","node"],"trace_function_info_fields":["function_id","name","script_name","script_id","line","column"],"trace_node_fields":["id","function_info_index","count","size","children"],"sample_fields":["timestamp_us","last_assigned_id"],"location_fields":["object_index","script_id","line","column"]}"#;

/// Node type indices (`meta.node_types[0]`).
const T_OBJECT: u64 = 3;
const T_ARRAY: u64 = 1;
const T_CLOSURE: u64 = 5;
const T_REGEXP: u64 = 6;
const T_NATIVE: u64 = 8;
const T_SYNTHETIC: u64 = 9;
const T_SYMBOL: u64 = 12;
/// Edge type indices (`meta.edge_types[0]`).
const E_CONTEXT: u64 = 0;
const E_INTERNAL: u64 = 3;
/// Numbers per node and per edge (`meta.node_fields`, `meta.edge_fields`).
const NODE_FIELDS: u64 = 7;
const EDGE_FIELDS: u64 = 3;

/// The string table, deduplicated. Index 0 is `<dummy>` and index 1 is `""`, as in Node's.
struct Strings {
    list: Vec<String>,
    index: HashMap<String, u64>,
}

impl Strings {
    fn new() -> Self {
        let mut s = Strings {
            list: Vec::new(),
            index: HashMap::new(),
        };
        s.intern("<dummy>");
        s.intern("");
        s
    }

    fn intern(&mut self, text: &str) -> u64 {
        if let Some(&i) = self.index.get(text) {
            return i;
        }
        let i = self.list.len() as u64;
        self.list.push(text.to_string());
        self.index.insert(text.to_string(), i);
        i
    }
}

/// The node type and name of an object (`Kind` mapped to V8's node types; closures are named by their function).
fn describe(kind: &Kind) -> (u64, &str) {
    match kind {
        Kind::Plain | Kind::Wrapper(_) | Kind::Promise(_) | Kind::Proxy(_) | Kind::Generator(_) => {
            (T_OBJECT, "Object")
        }
        Kind::Global => (T_OBJECT, "global"),
        Kind::Array(_) => (T_ARRAY, "Array"),
        Kind::Closure { def, .. } => (T_CLOSURE, def.name.as_deref().unwrap_or("")),
        Kind::Native(_) => (T_NATIVE, "Function"),
        Kind::RegExp(_) => (T_REGEXP, "RegExp"),
        Kind::Map(_) => (T_OBJECT, "Map"),
        Kind::Set(_) => (T_OBJECT, "Set"),
        Kind::CollIter(_) => (T_OBJECT, "Iterator"),
        Kind::Symbol { .. } => (T_SYMBOL, "Symbol"),
        Kind::ArrayBuffer(_) => (T_OBJECT, "ArrayBuffer"),
        Kind::Typed(_) => (T_OBJECT, "TypedArray"),
        Kind::RawJson(_) => (T_OBJECT, "RawJSON"),
    }
}

/// The whole snapshot as Node's JSON text, for the owning thread's registered objects.
pub(crate) fn snapshot_json() -> String {
    let objs = gc_registry::live_objs();
    // Node 0 is the root; object `i` (0-based in `objs`) is node `i + 1`.
    let node_of: HashMap<*const ObjData, u64> = objs
        .iter()
        .enumerate()
        .map(|(i, o)| (Rc::as_ptr(o), i as u64 + 1))
        .collect();
    let mut strings = Strings::new();
    let mut nodes: Vec<u64> = Vec::with_capacity(NODE_FIELDS as usize * (objs.len() + 1));
    // Object edges, grouped by source node in node order; `edges` holds `[type, name, to_node]` triples.
    let mut edges: Vec<u64> = Vec::new();
    let mut inbound = vec![false; objs.len() + 1];
    let root_name = strings.intern("");
    for (i, o) in objs.iter().enumerate() {
        let (ty, name) = match o.kind.try_borrow() {
            Ok(kind) => {
                let (ty, name) = describe(&kind);
                (ty, strings.intern(name))
            }
            // A busy kind (mutably borrowed by a running frame): an anonymous placeholder, no edges.
            Err(_) => (T_OBJECT, strings.intern("")),
        };
        // Like `estimate_bytes`, but a busy kind or property table counts only the record.
        let self_size = if o.kind.try_borrow().is_ok() && o.raw_props.try_borrow().is_ok() {
            estimate_bytes(o)
        } else {
            std::mem::size_of::<ObjData>()
        };
        let before = edges.len();
        obj_edges(o, &mut |edge| match edge {
            Edge::Obj(target) => {
                if let Some(&to) = node_of.get(&Rc::as_ptr(target)) {
                    inbound[to as usize] = true;
                    edges.extend([E_INTERNAL, root_name, to * NODE_FIELDS]);
                }
            }
            Edge::Scope(scope) => {
                let Ok(vars) = scope.vars.try_borrow() else {
                    return;
                };
                for (key, binding) in &vars.list {
                    if let Value::Object(target) = &binding.value {
                        if let Some(&to) = node_of.get(&Rc::as_ptr(target)) {
                            inbound[to as usize] = true;
                            let name = strings.intern(key);
                            edges.extend([E_CONTEXT, name, to * NODE_FIELDS]);
                        }
                    }
                }
            }
        });
        let count = (edges.len() - before) as u64 / EDGE_FIELDS;
        nodes.extend([
            ty,
            name,
            2 * (i as u64 + 1) + 1,
            self_size as u64,
            count,
            0,
            0,
        ]);
    }
    // The root's edges come first in the edge array: one per object that nothing registered references.
    let mut root_edges: Vec<u64> = Vec::new();
    let mut root_count = 0u64;
    for (i, referenced) in inbound.iter().enumerate().skip(1) {
        if !referenced {
            root_edges.extend([E_INTERNAL, root_name, i as u64 * NODE_FIELDS]);
            root_count += 1;
        }
    }
    let mut root = vec![T_SYNTHETIC, root_name, 1, 0, root_count, 0, 0];
    root.extend(nodes);
    let node_count = root.len() as u64 / NODE_FIELDS;
    let edge_count = (root_edges.len() + edges.len()) as u64 / EDGE_FIELDS;
    root_edges.extend(edges);
    let mut out = String::with_capacity(64 * (root.len() + root_edges.len()));
    out.push_str("{\"snapshot\":{\"meta\":");
    out.push_str(META);
    let _ = write!(
        out,
        ",\"node_count\":{node_count},\"edge_count\":{edge_count},\"trace_function_count\":0}},\"nodes\":["
    );
    push_numbers(&mut out, &root);
    out.push_str("],\"edges\":[");
    push_numbers(&mut out, &root_edges);
    out.push_str("],\"trace_function_infos\":[],\"trace_tree\":[],\"samples\":[],\"locations\":[],\"strings\":[");
    for (i, text) in strings.list.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        push_json_string(&mut out, text);
    }
    out.push_str("]}");
    out
}

fn push_numbers(out: &mut String, values: &[u64]) {
    for (i, v) in values.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(out, "{v}");
    }
}

/// A JSON string literal with only ASCII characters: control characters, quotes, backslashes and every non-ASCII
/// character (as UTF-16 code units) escaped.
fn push_json_string(out: &mut String, text: &str) {
    out.push('"');
    for unit in text.encode_utf16() {
        match unit {
            0x22 => out.push_str("\\\""),
            0x5c => out.push_str("\\\\"),
            0x0a => out.push_str("\\n"),
            0x0d => out.push_str("\\r"),
            0x09 => out.push_str("\\t"),
            0x20..=0x7e => out.push(unit as u8 as char),
            _ => out.push_str(&format!("\\u{unit:04x}")),
        }
    }
    out.push('"');
}
