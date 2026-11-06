//! The N-API side of the graph wire (PR 6a of the N-API seam). `decode_napi_wire` reads the wire thaw-napi
//! writes: the shared token and node decoder of host.rs with `GraphCx::napi` set, plus the lease lists.
//! The native tags (`TAGS`), the native pair proofs and the lone `nh`/`nfn`/`nsy` tokens are in the second half.
//! ponytail: the encoder (6a-3) is below; the `nfn`/`nh` emission and `napiLeases` come in 6a-4.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::host::{
    classify, graph_field, graph_token, handle_is_live, thaw_tjs_release_handle, wire_owner,
    with_package, GraphCx, HostError, Native,
};
use crate::{Interp, Obj, Value};

/// Decode an N-API graph wire into `(owning package, value)`. With a `leases` list, exactly those handle
/// references are released once decoding ends, on success and on error; a wire without the list (the
/// plain wire's shape) releases its `hdl` references in-node, as `decode_wire` does. `napiLeases`
/// references are released through the bridge in both cases.
#[allow(dead_code)] // wired up by the JS-visible globals in PR 6a-5
pub(crate) fn decode_napi_wire(wire: &str) -> Result<(usize, Value), String> {
    let Native::Object(top) = Native::from_json(wire).map_err(|e| e.to_string())? else {
        return Err("malformed graph wire".into());
    };
    let root = graph_field(&top, "root").ok_or("graph wire has no root")?;
    let nodes = match graph_field(&top, "nodes") {
        Some(Native::Array(nodes)) => nodes.as_slice(),
        _ => &[],
    };
    // ponytail: a malformed lease list returns before any release; the encoder's references then leak.
    let leases = match graph_field(&top, "leases") {
        None => None,
        Some(Native::Array(ids)) => Some(
            ids.iter()
                .map(|id| match id {
                    Native::Number(n) if n.fract() == 0.0 && *n >= 1.0 => Ok(*n as i64),
                    _ => Err("malformed graph lease".to_string()),
                })
                .collect::<Result<Vec<i64>, String>>()?,
        ),
        Some(_) => return Err("malformed graph leases".into()),
    };
    let napi_leases = match graph_field(&top, "napiLeases") {
        None => Vec::new(),
        Some(Native::Array(refs)) => refs
            .iter()
            .map(|r| match r {
                Native::Str(s) => s
                    .parse::<u64>()
                    .map_err(|_| "malformed graph lease".to_string()),
                _ => Err("malformed graph lease".to_string()),
            })
            .collect::<Result<Vec<u64>, String>>()?,
        Some(_) => return Err("malformed graph napi leases".into()),
    };
    let owner = wire_owner(nodes);
    let decoded = with_package(owner, |p| {
        let mut cx = GraphCx::new(nodes.len(), true, leases.is_none(), owner);
        graph_token(&mut p.interp, root, nodes, &mut cx).map_err(HostError::Bundle)
    });
    for id in leases.iter().flatten() {
        thaw_tjs_release_handle(*id);
    }
    for reference in napi_leases {
        crate::napi_bridge::release_reference(reference);
    }
    decoded.map(|v| (owner, v)).map_err(|e| e.to_string())
}

// ---- Native tags, native pair proofs and lone tokens (PR 6a-2) ----

/// What a tagged object is: a native Function wrapper (`nfn`) or a native-handle proxy (`nh`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Tag {
    Function,
    Handle,
}

thread_local! {
    /// The native identity of the JS objects the proxy layer made, keyed by address. The entry holds its
    /// `Obj`, so the address is not reused while the entry lives.
    /// ponytail: entries leak until the thread ends; a weak guard replaces the strong `Obj` when WeakRef gets real weakness (tj90 Q3).
    static TAGS: RefCell<HashMap<usize, (Obj, Tag, String)>> = RefCell::new(HashMap::new());
}

/// Record `object` as the native `kind` named by the decimal handle `native`.
pub(crate) fn tag(object: &Obj, kind: Tag, native: &str) {
    TAGS.with(|t| {
        t.borrow_mut().insert(
            Rc::as_ptr(object) as usize,
            (object.clone(), kind, native.to_string()),
        )
    });
}

/// Whether `text` is a positive decimal (`^[1-9][0-9]*$`), the form the wire spells native handles in.
pub(crate) fn is_decimal(text: &str) -> bool {
    !text.is_empty()
        && !text.starts_with('0')
        && text.bytes().all(|b| b.is_ascii_digit())
        && text.parse::<u64>().is_ok()
}

/// Whether `value` is proven to be the native `native` of `kind`: kind 1 a tagged native Function
/// (`thawGraphNapiFunctionHandles`), kind 2 a Symbol whose native id the bridge's metadata for `native`
/// confirms (runtime.js:979-998). `owner` names the Symbol registry for kind 2.
pub(crate) fn native_pair_matches(
    owner: Option<u64>,
    value: &Value,
    native: &str,
    kind: u8,
) -> bool {
    if !is_decimal(native) {
        return false;
    }
    match (kind, value) {
        (1, Value::Object(o)) => TAGS.with(|t| {
            t.borrow()
                .get(&(Rc::as_ptr(o) as usize))
                .is_some_and(|(_, tag, dec)| *tag == Tag::Function && dec == native)
        }),
        (2, _) => {
            let Some(id) =
                owner.and_then(|owner| crate::napi_symbols::native_id_of_value(owner, value))
            else {
                return false;
            };
            let Ok(reply) = crate::napi_bridge::handle_op("symbol_metadata", native, "", "[]")
            else {
                return false;
            };
            let Some(v) = reply.get("value") else {
                return false;
            };
            v.get("id").and_then(|i| i.as_str()) == Some(id.to_string().as_str())
                && v.get("handle").and_then(|h| h.as_str()) == Some(native)
        }
        _ => false,
    }
}

/// The reply's `value` of a `symbol_metadata` operation, checked as runtime.js:372-384 checks it.
struct SymbolMeta {
    id: String,
    units: Vec<u16>,
    registered: bool,
    js_origin: bool,
}

fn symbol_metadata(native: &str) -> Result<SymbolMeta, String> {
    const INVALID: &str = "Invalid native Symbol metadata";
    let reply = crate::napi_bridge::handle_op("symbol_metadata", native, "", "[]")?;
    let v = reply.get("value").ok_or(INVALID)?;
    let id = v
        .get("id")
        .and_then(|i| i.as_str())
        .filter(|i| is_decimal(i));
    let units = v.get("units").and_then(|u| u.as_array()).and_then(|units| {
        units
            .iter()
            .map(|u| u.as_u64().and_then(|n| u16::try_from(n).ok()))
            .collect::<Option<Vec<u16>>>()
    });
    match (
        id,
        units,
        v.get("handle").and_then(|h| h.as_str()),
        v.get("registered").and_then(|r| r.as_bool()),
        v.get("jsOrigin").and_then(|j| j.as_bool()),
    ) {
        (Some(id), Some(units), Some(handle), Some(registered), Some(js_origin))
            if handle == native =>
        {
            Ok(SymbolMeta {
                id: id.to_string(),
                units,
                registered,
                js_origin,
            })
        }
        _ => Err(INVALID.into()),
    }
}

/// The value of a live handle `handle` (a host handle id the wire names), with its reference still held.
fn live_value(handle: i64) -> Option<Value> {
    if !handle_is_live(handle) {
        return None;
    }
    crate::napi_ops::handle_value(handle as u64).ok()
}

/// A `{"nfn","hdl"}` node (`kind` 1) or a `{"nsy","hdl"}` token (`kind` 2): the value the handle names,
/// once `native_pair_matches` proves the pair (runtime.js:1964-1976, 2036-2044). The handle's reference is
/// released when `release` is set, as the in-node `hdl` arm does.
pub(crate) fn native_pair(
    owner: usize,
    fields: &[(String, Native)],
    kind: u8,
    release: bool,
) -> Result<Value, String> {
    let (field, invalid, mismatch) = if kind == 1 {
        (
            "nfn",
            "Invalid paired native Function graph node",
            "Mismatched native Function graph node",
        )
    } else {
        (
            "nsy",
            "Invalid paired native Symbol graph token",
            "Mismatched native Symbol graph token",
        )
    };
    let native = match graph_field(fields, field) {
        Some(Native::Str(s)) if is_decimal(s) => s.clone(),
        _ => return Err(invalid.into()),
    };
    let handle = match graph_field(fields, "hdl") {
        Some(Native::Number(n)) if n.fract() == 0.0 && *n >= 1.0 => *n as i64,
        _ => return Err(invalid.into()),
    };
    let value = live_value(handle).ok_or(invalid)?;
    if !native_pair_matches(Some(owner as u64), &value, &native, kind) {
        return Err(mismatch.into());
    }
    if release {
        thaw_tjs_release_handle(handle);
    }
    Ok(value)
}

/// A lone `{"nh"}` or `{"nfn"}` node: what the realm's proxy layer makes for the native handle `native`
/// (`__thaw_json_graph_proxy_for_handle` or `__thaw_napi_graph_function_for_handle`, installed by PR 6b).
pub(crate) fn lone_handle(it: &mut Interp, field: &str, native: &str) -> Result<Value, String> {
    let global_name = if field == "nh" {
        "__thaw_json_graph_proxy_for_handle"
    } else {
        "__thaw_napi_graph_function_for_handle"
    };
    let global = it.global.clone();
    let make = it
        .lookup(&global, global_name)
        .filter(Value::is_callable)
        .ok_or("Native addon proxy layer is not installed")?;
    it.call(&make, &Value::Undefined, &[Value::str(native)])
        .map_err(|c| classify(it, c).to_string())
}

/// A lone `{"nsy"}` token: the Symbol native id `native` names in `owner`, created from the bridge's metadata
/// when it is not known yet (`Symbol.for` for a registered one). Pinned unless it has a JavaScript origin or
/// is registered (runtime.js:2042-2050, 1244-1262).
pub(crate) fn lone_symbol(it: &mut Interp, owner: usize, native: &str) -> Result<Value, String> {
    let id = match native.parse::<u64>() {
        Ok(id) if is_decimal(native) => id,
        _ => return Err("Invalid native JSON graph token".into()),
    };
    let meta = symbol_metadata(native)?;
    if meta.id != native {
        return Err("Invalid native Symbol metadata".into());
    }
    let symbol = match crate::napi_symbols::symbol_of(owner as u64, id) {
        Some(symbol) => symbol,
        None => {
            // ponytail: a JavaScript-origin Symbol needs the proxy layer (PR 6b).
            if meta.js_origin {
                return Err(
                    "native Symbol with a JavaScript origin needs the N-API proxy layer".into(),
                );
            }
            let global = it.global.clone();
            let ctor = it
                .lookup(&global, "Symbol")
                .ok_or("Symbol is not defined")?;
            let description = Value::str(&crate::builtins::units_str(&meta.units));
            let symbol = if meta.registered {
                let symbol_for = it
                    .get(&ctor, "for")
                    .map_err(|c| classify(it, c).to_string())?;
                it.call(&symbol_for, &Value::Undefined, &[description])
            } else {
                it.call(&ctor, &Value::Undefined, &[description])
            }
            .map_err(|c| classify(it, c).to_string())?;
            let Value::Object(object) = &symbol else {
                return Err("native Symbol did not create a Symbol".into());
            };
            crate::napi_symbols::bind(owner as u64, id, object);
            symbol
        }
    };
    if !meta.js_origin && !meta.registered {
        crate::napi_symbols::thaw_tjs_napi_symbol_set_pin(owner as u64, id, 1);
    }
    Ok(symbol)
}

// ---- The N-API encoder (PR 6a-3): the data part of runtime.js `thawGraphEncode`, live mode. ----
// ponytail: no `nfn`/`nh` tags and no `napiLeases` (PR 6a-4); no plain-result mode (6a-5 keeps it on WireOut).
// ponytail: a Proxy, Promise, Generator, wrapper or global object is a handle, not a copy (QuickJS copies its own keys).

use crate::builtins::{json_quote, str_units};
use crate::host::{live_handle_of, store_handle, thaw_tjs_retain_handle};
use crate::napi_props::own_entries;
use crate::napi_symbols::is_symbol;
use crate::{number_to_string, strict_equals, unhole, Kind, ObjData, Slot};

/// A queued node: a value to copy, or a carrier handle (a masked root item).
enum Source {
    Value(Value),
    Carrier(u64),
}

/// The encoder state of one graph wire. `sources[i]` is node `i`. A value's token is `{"r":i}` (or an
/// inline `{"hdl"}`, `{"v"}`...); a repeated object reuses its node, so cycles and shared references hold.
pub(crate) struct NapiWireOut {
    owner: usize,
    live: bool,
    force: Option<Value>,
    leases: Vec<u64>,
    sources: Vec<Source>,
    nodes: Vec<String>,
    ids: HashMap<*const ObjData, usize>,
    symbol_nodes: HashMap<*const ObjData, usize>,
    symbol_handles: HashMap<*const ObjData, u64>,
}

/// The Error a value with no graph form raises (runtime.js `Unsupported native callback graph value`).
fn unsupported_value() -> HostError {
    HostError::Thrown("TypeError: Unsupported native callback graph value".into())
}

/// The UTF-16 units of `s` when it holds an unpaired surrogate (`su` token), else `None`.
fn malformed_units(s: &str) -> Option<Vec<u16>> {
    let units = str_units(s);
    let mut malformed = false;
    let mut i = 0;
    while i < units.len() {
        let unit = units[i];
        if (0xD800..=0xDBFF).contains(&unit) {
            if units
                .get(i + 1)
                .is_some_and(|n| (0xDC00..=0xDFFF).contains(n))
            {
                i += 2;
                continue;
            }
            malformed = true;
        } else if (0xDC00..=0xDFFF).contains(&unit) {
            malformed = true;
        }
        i += 1;
    }
    malformed.then_some(units)
}

fn units_json(units: &[u16]) -> String {
    let items: Vec<String> = units.iter().map(u16::to_string).collect();
    format!("[{}]", items.join(","))
}

/// The current value of own data property `key`: an array slot, `length`, or a stored slot.
fn own_value(o: &Obj, key: &str) -> Value {
    if let Kind::Array(items) = &*o.kind.borrow() {
        if let Ok(slot) = key.parse::<usize>() {
            return unhole(items.get(slot));
        }
        if key == "length" {
            return Value::Number(items.len() as f64);
        }
    }
    match o.position(key) {
        Some(i) => match &o.props.borrow()[i].slot {
            Slot::Data(v) => v.clone(),
            Slot::Accessor { .. } => Value::Undefined,
        },
        None => Value::Undefined,
    }
}

/// Whether `o` has an own accessor property (such an object stays a live handle).
fn has_accessor(o: &Obj) -> bool {
    o.props
        .borrow()
        .iter()
        .any(|p| matches!(p.slot, Slot::Accessor { .. }))
}

/// The Symbol behind an internal `@@sym:<id>` property key.
fn symbol_of_key(it: &Interp, key: &str) -> Result<Value, HostError> {
    key.strip_prefix("@@sym:")
        .and_then(|id| id.parse::<u64>().ok())
        .and_then(|id| it.rt.symbols.get(&id))
        .map(|o| Value::Object(o.clone()))
        .ok_or_else(unsupported_value)
}

impl NapiWireOut {
    #[allow(dead_code)] // wired up in PR 6a-5
    fn new(owner: usize, live: bool, force: Option<Value>) -> Self {
        NapiWireOut {
            owner,
            live,
            force,
            leases: Vec::new(),
            sources: Vec::new(),
            nodes: Vec::new(),
            ids: HashMap::new(),
            symbol_nodes: HashMap::new(),
            symbol_handles: HashMap::new(),
        }
    }

    /// One more reference to `v` for the wire: the canonical handle of an object, else a new one.
    fn lease(&mut self, v: &Value) -> u64 {
        let id = match live_handle_of(v) {
            Some(id) if thaw_tjs_retain_handle(id as i64) == 1 => id,
            _ => store_handle(self.owner, v.clone()) as u64,
        };
        self.leases.push(id);
        id
    }

    fn hdl(&mut self, v: &Value) -> String {
        format!("{{\"hdl\":{}}}", self.lease(v))
    }

    fn source(&mut self, s: Source) -> usize {
        self.sources.push(s);
        self.nodes.push(String::new());
        self.sources.len() - 1
    }

    /// The token of `v`: an inline scalar or handle, or `{"r":i}` for an object or symbol node.
    fn token(&mut self, v: &Value) -> Result<String, HostError> {
        if self.force.as_ref().is_some_and(|f| strict_equals(f, v)) {
            return Ok(self.hdl(v));
        }
        Ok(match v {
            Value::Undefined | Value::Hole => "{\"u\":1}".into(),
            Value::Null => "{\"v\":null}".into(),
            Value::Bool(b) => format!("{{\"v\":{b}}}"),
            Value::Number(n) if *n == 0.0 && n.is_sign_negative() => "{\"nf\":\"-0\"}".into(),
            Value::Number(n) if n.is_nan() => "{\"nf\":\"NaN\"}".into(),
            Value::Number(n) if n.is_infinite() => {
                format!("{{\"nf\":\"{}Infinity\"}}", if *n < 0.0 { "-" } else { "" })
            }
            Value::Number(n) => format!("{{\"v\":{}}}", number_to_string(*n)),
            Value::Str(s) => match malformed_units(s) {
                Some(units) => format!("{{\"su\":{}}}", units_json(&units)),
                None => {
                    let mut q = String::new();
                    json_quote(s, &mut q);
                    format!("{{\"v\":{q}}}")
                }
            },
            Value::BigInt(b) => format!("{{\"bi\":\"{b}\"}}"),
            Value::Object(o) => {
                let symbol = is_symbol(v);
                if symbol && !self.live {
                    return Err(unsupported_value());
                }
                let key = Rc::as_ptr(o);
                let known = if symbol {
                    self.symbol_nodes.get(&key)
                } else {
                    self.ids.get(&key)
                };
                let index = match known {
                    Some(i) => *i,
                    None => {
                        let i = self.source(Source::Value(v.clone()));
                        if symbol {
                            self.symbol_nodes.insert(key, i);
                        } else {
                            self.ids.insert(key, i);
                        }
                        i
                    }
                };
                format!("{{\"r\":{index}}}")
            }
        })
    }

    /// The wire key of an own property: its raw text, a `su` token, or the token of a Symbol.
    fn key_token(&mut self, it: &Interp, key: &str) -> Result<String, HostError> {
        if key.starts_with("@@sym:") {
            let symbol = symbol_of_key(it, key)?;
            return self.token(&symbol);
        }
        Ok(match malformed_units(key) {
            Some(units) => format!("{{\"su\":{}}}", units_json(&units)),
            None => {
                let mut q = String::new();
                json_quote(key, &mut q);
                q
            }
        })
    }

    /// Property entries `[key, token, flags]` in own-key order. A masked root slot is its carrier.
    fn props(
        &mut self,
        it: &Interp,
        o: &Obj,
        masks: &HashMap<usize, u64>,
        carriers: &mut HashMap<usize, usize>,
    ) -> Result<Vec<String>, HostError> {
        let mut parts = Vec::new();
        for (key, flags) in own_entries(o) {
            let key_token = self.key_token(it, &key)?;
            let masked = key
                .parse::<usize>()
                .ok()
                .and_then(|slot| masks.get(&slot).map(|h| (slot, *h)));
            let value = match masked {
                Some((slot, handle)) => self.carrier(slot, handle, carriers),
                None => self.token(&own_value(o, &key))?,
            };
            parts.push(format!("[{key_token},{value},{flags}]"));
        }
        Ok(parts)
    }

    /// The `{"r":i}` token of the carrier node for root slot `slot` (one node per slot).
    fn carrier(
        &mut self,
        slot: usize,
        handle: u64,
        carriers: &mut HashMap<usize, usize>,
    ) -> String {
        let index = match carriers.get(&slot) {
            Some(i) => *i,
            None => {
                let i = self.source(Source::Carrier(handle));
                carriers.insert(slot, i);
                i
            }
        };
        format!("{{\"r\":{index}}}")
    }

    /// The node body of queued value `index`: a handle, a byte/date/collection/RegExp node, or a copy.
    fn node(
        &mut self,
        it: &mut Interp,
        index: usize,
        masks: &HashMap<usize, u64>,
    ) -> Result<String, HostError> {
        let value = match &self.sources[index] {
            Source::Carrier(handle) => return Ok(format!("{{\"hdl\":{handle}}}")),
            Source::Value(v) => v.clone(),
        };
        let Value::Object(o) = &value else {
            return Err(unsupported_value());
        };
        if is_symbol(&value) {
            let key = Rc::as_ptr(o);
            let id = match self.symbol_handles.get(&key) {
                Some(id) => *id,
                None => {
                    let id = self.lease(&value);
                    self.symbol_handles.insert(key, id);
                    id
                }
            };
            return Ok(format!("{{\"hdl\":{id}}}"));
        }
        // Only the root is copied in live mode: every other object stays behind its handle.
        if (self.live && index != 0) || value.is_callable() {
            return Ok(self.hdl(&value));
        }
        let bytes = match &*o.kind.borrow() {
            Kind::ArrayBuffer(bytes) => Some(bytes.borrow().clone()),
            Kind::Typed(t) => {
                let end = t.off + t.len * t.ty.size();
                Some(t.data.borrow()[t.off..end].to_vec())
            }
            _ => None,
        };
        if let Some(bytes) = bytes {
            let items: Vec<String> = bytes.iter().map(u8::to_string).collect();
            return Ok(format!("{{\"b\":[{}]}}", items.join(",")));
        }
        let is_plain = matches!(&*o.kind.borrow(), Kind::Plain);
        if is_plain {
            if let Value::Number(time) = own_value(o, "__time") {
                let time = if time.is_finite() {
                    number_to_string(time)
                } else {
                    "null".into()
                };
                return Ok(format!("{{\"d\":{time}}}"));
            }
        }
        let map = match &*o.kind.borrow() {
            Kind::Map(m) => Some(m.entries.clone()),
            _ => None,
        };
        if let Some(entries) = map {
            let mut pairs = Vec::with_capacity(entries.len());
            for (k, v) in entries {
                pairs.push(it.array(vec![k, v]));
            }
            let outer = it.array(pairs);
            let t = self.token(&outer)?;
            return Ok(format!("{{\"m\":{t}}}"));
        }
        let set = match &*o.kind.borrow() {
            Kind::Set(s) => Some(s.items.clone()),
            _ => None,
        };
        if let Some(items) = set {
            let outer = it.array(items);
            let t = self.token(&outer)?;
            return Ok(format!("{{\"s\":{t}}}"));
        }
        if matches!(&*o.kind.borrow(), Kind::RegExp(_)) {
            let text = |v: Value| match v {
                Value::Str(s) => s.to_string(),
                _ => String::new(),
            };
            let source = text(own_value(o, "source"));
            let flags = text(own_value(o, "flags"));
            let last = match own_value(o, "lastIndex") {
                Value::Number(n) if n.is_finite() => number_to_string(n),
                _ => "0".into(),
            };
            let (mut s, mut f) = (String::new(), String::new());
            json_quote(&source, &mut s);
            json_quote(&flags, &mut f);
            return Ok(format!("{{\"re\":[{s},{f},{last}]}}"));
        }
        let is_array = matches!(&*o.kind.borrow(), Kind::Array(_));
        if !(is_array || is_plain) || has_accessor(o) {
            return Ok(self.hdl(&value));
        }
        let empty = HashMap::new();
        // Only the root array can carry masked items (the root is node 0).
        let masks = if index == 0 { masks } else { &empty };
        let mut carriers = HashMap::new();
        if is_array {
            let items = match &*o.kind.borrow() {
                Kind::Array(items) => items.clone(),
                _ => Vec::new(),
            };
            let mut entries = Vec::with_capacity(items.len());
            for (slot, item) in items.iter().enumerate() {
                entries.push(match item {
                    Value::Hole => "{\"h\":1}".to_string(),
                    _ => match masks.get(&slot) {
                        Some(handle) => self.carrier(slot, *handle, &mut carriers),
                        None => self.token(item)?,
                    },
                });
            }
            let props = self.props(it, o, masks, &mut carriers)?;
            return Ok(format!(
                "{{\"a\":[{}],\"p\":[{}]}}",
                entries.join(","),
                props.join(",")
            ));
        }
        let props = self.props(it, o, masks, &mut carriers)?;
        Ok(format!("{{\"o\":[{}]}}", props.join(",")))
    }

    /// The whole wire for `root`: masked root items are leased first, then nodes are queued and filled.
    fn encode(
        &mut self,
        it: &mut Interp,
        root: &Value,
        handle_mask: u32,
    ) -> Result<String, HostError> {
        let root_forced = self.force.as_ref().is_some_and(|f| strict_equals(f, root));
        if handle_mask == 0 && self.live && root_forced && matches!(root, Value::Object(_)) {
            let id = self.lease(root);
            return Ok(format!(
                "{{\"root\":{{\"r\":0}},\"nodes\":[{{\"hdl\":{id}}}],\"leases\":[{id}],\"napiLeases\":[]}}"
            ));
        }
        let mut masks = HashMap::new();
        if let Value::Object(o) = root {
            let items = match &*o.kind.borrow() {
                Kind::Array(items) if handle_mask != 0 && !root_forced => items.clone(),
                _ => Vec::new(),
            };
            for (slot, item) in items.iter().enumerate() {
                if (handle_mask >> (slot % 32)) & 1 != 0 && !matches!(item, Value::Hole) {
                    let handle = self.lease(item);
                    masks.insert(slot, handle);
                }
            }
        }
        let root_token = self.token(root)?;
        let mut index = 0;
        while index < self.sources.len() {
            let node = self.node(it, index, &masks)?;
            self.nodes[index] = node;
            index += 1;
        }
        let leases: Vec<String> = self.leases.iter().map(u64::to_string).collect();
        Ok(format!(
            "{{\"root\":{root_token},\"nodes\":[{}],\"leases\":[{}],\"napiLeases\":[]}}",
            self.nodes.join(","),
            leases.join(",")
        ))
    }
}

/// Encode `root` as an N-API graph wire (live: the result keeps its objects behind handles). `handle_mask`
/// carries root array items by handle; `force` is the value that must stay a handle (the call's receiver).
/// On error every lease taken so far is released.
#[allow(dead_code)] // wired up in PR 6a-5
pub(crate) fn encode_napi_graph(
    it: &mut Interp,
    owner: usize,
    root: &Value,
    handle_mask: u32,
    live: bool,
    force: Option<Value>,
) -> Result<String, HostError> {
    let mut out = NapiWireOut::new(owner, live, force);
    match out.encode(it, root, handle_mask) {
        Ok(wire) => Ok(wire),
        Err(e) => {
            for id in out.leases.drain(..) {
                thaw_tjs_release_handle(id as i64);
            }
            Err(e)
        }
    }
}
