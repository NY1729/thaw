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

/// Intl rows the polyfill does not serve yet under `--features intl` (`owner<TAB>key`). Each row is
/// removed when it is served; the test fails while a listed row is served but still here.
const INTL_PENDING: &[&str] = &[
    "Intl\tsupportedValuesOf",
    "Intl.supportedValuesOf\tlength",
    "Intl.supportedValuesOf\tname",
    "Intl.ListFormat.prototype\tformatToParts",
    "Intl.ListFormat.prototype\tresolvedOptions",
    "Intl.Locale.prototype\tcalendars",
    "Intl.Locale.prototype\tcaseFirst",
    "Intl.Locale.prototype\tcollations",
    "Intl.Locale.prototype\thourCycle",
    "Intl.Locale.prototype\thourCycles",
    "Intl.Locale.prototype\tnumberingSystems",
    "Intl.Locale.prototype\tnumeric",
    "Intl.Locale.prototype\ttextInfo",
    "Intl.Locale.prototype\ttimeZones",
    "Intl.Locale.prototype\tweekInfo",
];

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

/// `name`/`length` of closure `f`: its own prop when it has one, else the value synthesized on read
/// (`synth_fn_own`), which is what a JS read of the closure sees.
fn own_or_synth(f: &Obj, key: &str) -> Option<Value> {
    own(f, key)
        .as_ref()
        .and_then(data)
        .or_else(|| synth_fn_own(f, key))
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
        // a closure's `name`/`length` is present when it is synthesized on read (`synth_fn_own`); the
        // value itself is checked through the function row that owns it
        // an Array exotic's `length` is derived from its elements, so it has no raw prop either
        let array_length = &*k == "length" && matches!(&*o.kind.borrow(), Kind::Array(_));
        let synthesized = !row.accessor
            && ((matches!(&*k, "name" | "length") && synth_fn_own(&o, &k).is_some())
                || array_length);
        if !(synthesized || it.table_row_present(&o, &k, &row)) {
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
                    let got_name = own_or_synth(f, "name").as_ref().and_then(text);
                    let got_len = own_or_synth(f, "length");
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
            let len = own_or_synth(&f, "length").and_then(|v| match v {
                Value::Number(n) => Some(n),
                _ => None,
            });
            if len != row.length {
                failures.push(format!("{what}: length {len:?}, table {:?}", row.length));
            }
            let name = own_or_synth(&f, "name").as_ref().and_then(text);
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
        let intl = cfg!(feature = "intl");
        for absent in missing.difference(&listed) {
            failures.push(format!("absent row not listed: {absent:?}"));
        }
        if intl {
            // Under `intl` the Intl rows are served by the polyfill, so the list (built without Intl)
            // cannot be exact for them. Every Intl row still absent must be in INTL_PENDING, and every
            // INTL_PENDING row must still be absent: the list shrinks as rows are served.
            let pending: BTreeSet<String> = INTL_PENDING.iter().map(|r| r.to_string()).collect();
            for absent in missing.iter().filter(|r| r.starts_with("Intl")) {
                if !pending.contains(absent) {
                    failures.push(format!(
                        "Intl row absent and not in INTL_PENDING: {absent:?}"
                    ));
                }
            }
            for row in pending.difference(&missing) {
                failures.push(format!("INTL_PENDING row is served, remove it: {row:?}"));
            }
        }
        for present in listed.difference(&missing) {
            // The `wasm` feature installs the real WebAssembly namespace (web/webassembly.js): its
            // listed rows are present, and the descriptor loop above has already compared each one
            // with the table. The `intl` feature does the same for the Intl rows. The default build
            // keeps the list exact.
            if (cfg!(feature = "wasm") && present.starts_with("WebAssembly"))
                || (intl && present.starts_with("Intl"))
            {
                continue;
            }
            failures.push(format!("listed row is present: {present:?}"));
        }
        if cfg!(feature = "wasm") {
            // every WebAssembly row is served under the feature except the sloppy-function
            // `arguments`/`caller` own props, which the shim (strict mode) does not model
            for absent in missing.iter().filter(|r| r.starts_with("WebAssembly")) {
                let key = absent.split('\t').nth(1).unwrap_or("");
                if key != "arguments" && key != "caller" {
                    failures.push(format!(
                        "WebAssembly row absent under the wasm feature: {absent:?}"
                    ));
                }
            }
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

/// Intl constructors, statics and data methods: `name` and `length` against Node 22. The expected lines are
/// the output of `node intl_subset.js` under node 22.23.1, where the script is the JS run below with
/// `console.log` at the end. Accessors (`format`, `compare`, the `Locale` getters) are left out: thaw-js still
/// has them as data methods.
#[cfg(feature = "intl")]
#[test]
fn intl_names_and_lengths_match_node_22() {
    let mut it = Interp::new();
    it.run(
        r#"const paths = ['Intl.getCanonicalLocales', 'Intl.Collator', 'Intl.DateTimeFormat', 'Intl.DisplayNames',
  'Intl.ListFormat', 'Intl.Locale', 'Intl.NumberFormat', 'Intl.PluralRules', 'Intl.RelativeTimeFormat',
  'Intl.Segmenter', 'Intl.Collator.prototype.resolvedOptions', 'Intl.Collator.supportedLocalesOf',
  'Intl.DateTimeFormat.prototype.formatToParts', 'Intl.DateTimeFormat.prototype.formatRange',
  'Intl.DateTimeFormat.prototype.formatRangeToParts', 'Intl.DateTimeFormat.prototype.resolvedOptions',
  'Intl.DateTimeFormat.supportedLocalesOf', 'Intl.DisplayNames.prototype.of',
  'Intl.DisplayNames.prototype.resolvedOptions', 'Intl.DisplayNames.supportedLocalesOf',
  'Intl.ListFormat.prototype.format', 'Intl.ListFormat.supportedLocalesOf', 'Intl.Locale.prototype.toString',
  'Intl.Locale.prototype.maximize', 'Intl.Locale.prototype.minimize',
  'Intl.NumberFormat.prototype.formatToParts', 'Intl.NumberFormat.prototype.formatRange',
  'Intl.NumberFormat.prototype.formatRangeToParts', 'Intl.NumberFormat.prototype.resolvedOptions',
  'Intl.NumberFormat.supportedLocalesOf', 'Intl.PluralRules.prototype.select',
  'Intl.PluralRules.prototype.selectRange', 'Intl.PluralRules.prototype.resolvedOptions',
  'Intl.PluralRules.supportedLocalesOf', 'Intl.RelativeTimeFormat.prototype.format',
  'Intl.RelativeTimeFormat.prototype.formatToParts', 'Intl.RelativeTimeFormat.prototype.resolvedOptions',
  'Intl.RelativeTimeFormat.supportedLocalesOf', 'Intl.Segmenter.prototype.segment',
  'Intl.Segmenter.prototype.resolvedOptions', 'Intl.Segmenter.supportedLocalesOf'];
const out = paths.map(p => { const f = p.split('.').reduce((o, k) => o[k], globalThis); return `${p} ${f.name} ${f.length}`; });
console.log(out.join('\n'));"#,
    )
    .expect("probe runs");
    let expected = r#"Intl.getCanonicalLocales getCanonicalLocales 1
Intl.Collator Collator 0
Intl.DateTimeFormat DateTimeFormat 0
Intl.DisplayNames DisplayNames 2
Intl.ListFormat ListFormat 0
Intl.Locale Locale 1
Intl.NumberFormat NumberFormat 0
Intl.PluralRules PluralRules 0
Intl.RelativeTimeFormat RelativeTimeFormat 0
Intl.Segmenter Segmenter 0
Intl.Collator.prototype.resolvedOptions resolvedOptions 0
Intl.Collator.supportedLocalesOf supportedLocalesOf 1
Intl.DateTimeFormat.prototype.formatToParts formatToParts 1
Intl.DateTimeFormat.prototype.formatRange formatRange 2
Intl.DateTimeFormat.prototype.formatRangeToParts formatRangeToParts 2
Intl.DateTimeFormat.prototype.resolvedOptions resolvedOptions 0
Intl.DateTimeFormat.supportedLocalesOf supportedLocalesOf 1
Intl.DisplayNames.prototype.of of 1
Intl.DisplayNames.prototype.resolvedOptions resolvedOptions 0
Intl.DisplayNames.supportedLocalesOf supportedLocalesOf 1
Intl.ListFormat.prototype.format format 1
Intl.ListFormat.supportedLocalesOf supportedLocalesOf 1
Intl.Locale.prototype.toString toString 0
Intl.Locale.prototype.maximize maximize 0
Intl.Locale.prototype.minimize minimize 0
Intl.NumberFormat.prototype.formatToParts formatToParts 1
Intl.NumberFormat.prototype.formatRange formatRange 2
Intl.NumberFormat.prototype.formatRangeToParts formatRangeToParts 2
Intl.NumberFormat.prototype.resolvedOptions resolvedOptions 0
Intl.NumberFormat.supportedLocalesOf supportedLocalesOf 1
Intl.PluralRules.prototype.select select 1
Intl.PluralRules.prototype.selectRange selectRange 2
Intl.PluralRules.prototype.resolvedOptions resolvedOptions 0
Intl.PluralRules.supportedLocalesOf supportedLocalesOf 1
Intl.RelativeTimeFormat.prototype.format format 2
Intl.RelativeTimeFormat.prototype.formatToParts formatToParts 2
Intl.RelativeTimeFormat.prototype.resolvedOptions resolvedOptions 0
Intl.RelativeTimeFormat.supportedLocalesOf supportedLocalesOf 1
Intl.Segmenter.prototype.segment segment 1
Intl.Segmenter.prototype.resolvedOptions resolvedOptions 0
Intl.Segmenter.supportedLocalesOf supportedLocalesOf 1"#;
    assert_eq!(it.out.join("\n"), expected);
}

/// Intl constructor shapes against Node 22.23.1: call without `new`, bound `format`/`compare` accessors,
/// `Symbol.toStringTag` and the absent `Intl.DurationFormat`. The expected text is the stdout of
/// `node shapes_probe.js` (the script below, which ends in `console.log`). Instance internals (`Object.keys`,
/// own `_` fields) are not checked here; they come with hiding the helpers.
#[cfg(feature = "intl")]
#[test]
fn intl_constructor_and_accessor_shapes_match_node_22() {
    let mut it = Interp::new();
    it.run(
        r#"const out = [];
const push = (...a) => out.push(a.map(x => typeof x === 'string' ? x : JSON.stringify(x)).join(' '));
const nf = new Intl.NumberFormat('en');
const dtf = new Intl.DateTimeFormat('en', { timeZone: 'UTC' });
const col = new Intl.Collator('en');
push('call', Intl.NumberFormat('en').format(1234.5), Intl.Collator('en').compare('a', 'b'), Intl.DateTimeFormat('en', { timeZone: 'UTC' }).format(0));
push('instanceof', Intl.NumberFormat('en') instanceof Intl.NumberFormat, Intl.Collator() instanceof Intl.Collator);
for (const n of ['PluralRules', 'ListFormat', 'RelativeTimeFormat', 'Segmenter', 'Locale']) {
  let r; try { Intl[n]('en'); r = 'no throw'; } catch (e) { r = e.constructor.name; }
  push('nonew', n, r);
}
push('map', [1, 2.5, 1000].map(nf.format).join('|'));
push('sort', ['b', 'a'].sort(col.compare).join());
push('tag', Object.prototype.toString.call(nf), Object.prototype.toString.call(dtf), Object.prototype.toString.call(col),
  Object.prototype.toString.call(new Intl.PluralRules()), Object.prototype.toString.call(new Intl.ListFormat()),
  Object.prototype.toString.call(new Intl.RelativeTimeFormat()), Object.prototype.toString.call(new Intl.Segmenter()),
  Object.prototype.toString.call(new Intl.Locale('en')), Object.prototype.toString.call(new Intl.DisplayNames('en', { type: 'region' })));
push('identity', nf.format === nf.format, col.compare === col.compare, dtf.format === dtf.format, nf.format === new Intl.NumberFormat('en').format);
push('fnshape', nf.format.name, nf.format.length, col.compare.name, col.compare.length, dtf.format.name, dtf.format.length, typeof nf.format);
push('noown', Object.prototype.hasOwnProperty.call(nf, 'format'), Object.prototype.hasOwnProperty.call(dtf, 'format'), Object.prototype.hasOwnProperty.call(col, 'compare'));
const acc = Object.getOwnPropertyDescriptor(Intl.NumberFormat.prototype, 'format');
push('accessor', typeof acc.get, typeof acc.set, acc.enumerable, acc.configurable, acc.get.name, acc.get.length);
push('protodesc', Object.getOwnPropertyDescriptor(Intl.NumberFormat, 'prototype'), Object.getOwnPropertyDescriptor(Intl.NumberFormat.prototype, 'constructor').enumerable);
push('ctor', Intl.NumberFormat.prototype.constructor === Intl.NumberFormat, Intl.Collator.prototype.constructor === Intl.Collator);
let r1; try { acc.get.call({}); r1 = 'no throw'; } catch (e) { r1 = e.constructor.name; }
push('brand', r1);
push('durationformat', typeof Intl.DurationFormat);
push('tagdesc', Object.getOwnPropertyDescriptor(Intl.Collator.prototype, Symbol.toStringTag));
push('detached', [3, 1].map(col.compare), new Intl.NumberFormat('de-DE').format(1234.5));
console.log(out.join('\n'));
"#,
    )
    .expect("probe runs");
    let expected = r#"call 1,234.5 -1 1/1/1970
instanceof true true
nonew PluralRules TypeError
nonew ListFormat TypeError
nonew RelativeTimeFormat TypeError
nonew Segmenter TypeError
nonew Locale TypeError
map 1|2.5|1,000
sort a,b
tag [object Intl.NumberFormat] [object Intl.DateTimeFormat] [object Intl.Collator] [object Intl.PluralRules] [object Intl.ListFormat] [object Intl.RelativeTimeFormat] [object Intl.Segmenter] [object Intl.Locale] [object Intl.DisplayNames]
identity true true true false
fnshape  1  2  1 function
noown false false false
accessor function undefined false true get format 0
protodesc {"value":{},"writable":false,"enumerable":false,"configurable":false} false
ctor true true
brand TypeError
durationformat undefined
tagdesc {"value":"Intl.Collator","writable":false,"enumerable":false,"configurable":true}
detached [1,0] 1.234,5"#;
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

/// Every realm applies the table, but the embedded text is parsed once per process: the parse is
/// cached, so a second and later realm pays no parse cost.
#[test]
fn builtin_table_is_parsed_once_for_all_realms() {
    let _ = Interp::new_vm_realm();
    let _ = Interp::new();
    let parses = crate::builtin_table::PARSES.load(std::sync::atomic::Ordering::Relaxed);
    assert_eq!(parses, 1, "the embedded table is parsed once per process");
    for _ in 0..3 {
        drop(Interp::new_vm_realm());
        drop(Interp::new());
    }
    assert_eq!(
        crate::builtin_table::PARSES.load(std::sync::atomic::Ordering::Relaxed),
        1,
        "later realms re-parsed the embedded table"
    );
}

/// The `@@toStringTag` and `@@species` rows that thaw-js lacks on an owner it has are defined as Node 22
/// defines them: the tag string with the table's flags, and a species getter that returns `this`.
#[test]
fn builtin_table_defines_missing_tags_and_species_as_node() {
    let mut it = Interp::new();
    it.run(
        "const out = [];\
         const tag = (o) => Object.prototype.toString.call(o);\
         out.push(tag(Promise.prototype), tag(Symbol.prototype), tag(URL.prototype), tag(ArrayBuffer.prototype), tag(Atomics));\
         out.push(tag(new Map().entries()), tag(new Set().values()), tag(Math), tag(WeakRef.prototype));\
         for (const C of [Array, ArrayBuffer, Map, Promise, RegExp, Set, SharedArrayBuffer]) {\
           const d = Object.getOwnPropertyDescriptor(C, Symbol.species);\
           out.push(!d ? C.name + ' none' : [C.name, typeof d.get, d.get.name, d.get.length, d.set, d.enumerable, d.configurable, C[Symbol.species] === C].join(' '));\
         }\
         out.push(JSON.stringify(Object.getOwnPropertyDescriptor(Symbol.prototype, Symbol.toStringTag)));\
         out.push(JSON.stringify(Object.getOwnPropertyDescriptor(Promise.prototype, Symbol.toStringTag)));\
         console.log(out.join('\\n'));",
    )
    .expect("probe runs");
    let expected = r#"[object Promise]
[object Symbol]
[object URL]
[object ArrayBuffer]
[object Atomics]
[object Map Iterator]
[object Set Iterator]
[object Math]
[object WeakRef]
Array function get [Symbol.species] 0  false true true
ArrayBuffer function get [Symbol.species] 0  false true true
Map function get [Symbol.species] 0  false true true
Promise function get [Symbol.species] 0  false true true
RegExp function get [Symbol.species] 0  false true true
Set function get [Symbol.species] 0  false true true
SharedArrayBuffer function get [Symbol.species] 0  false true true
{"value":"Symbol","writable":false,"enumerable":false,"configurable":true}
{"value":"Promise","writable":false,"enumerable":false,"configurable":true}"#;
    assert_eq!(it.out.join("\n"), expected);
}

/// Iterator, async-iterator, console.Console and timer shapes that Node 22 defines (`node probe.js` output is
/// the expected text; the probe is the same JS run under thaw-js).
#[test]
fn misc_shapes_match_node() {
    let mut it = Interp::new();
    if let Err(why) = it.run(r##"const out = [];
const f = (x) => typeof x === 'function' ? x.name + '/' + x.length : typeof x === 'string' ? JSON.stringify(x) : String(x);
const d = (o, k) => {
  const p = Object.getOwnPropertyDescriptor(o, k);
  if (!p) return 'NONE';
  const parts = [];
  if (p.get) parts.push('get:' + f(p.get));
  if (p.set) parts.push('set:' + f(p.set));
  if ('value' in p) parts.push('v:' + f(p.value));
  parts.push('w' + (p.writable ? 1 : 0) + 'e' + (p.enumerable ? 1 : 0) + 'c' + (p.configurable ? 1 : 0));
  return parts.join(' ');
};
const t = (fn) => { try { return String(fn()); } catch (e) { return 'THREW ' + e.name; } };
const AIP = Object.getPrototypeOf([][Symbol.iterator]());
const AsyncIP = Object.getPrototypeOf(Object.getPrototypeOf(Object.getPrototypeOf((async function* () {})())));
out.push('AIP tag ' + d(AIP, Symbol.toStringTag));
out.push('AIP next ' + d(AIP, 'next'));
out.push('AIP.next own prototype ' + d(AIP.next, 'prototype'));
out.push('AIP.next call on non-iterator ' + t(() => AIP.next.call({})));
out.push('AIP proto is Iterator.prototype ' + (Object.getPrototypeOf(AIP) === Iterator.prototype));
out.push('array iter next ' + JSON.stringify([...[7, 8].values()]) + ' ' + JSON.stringify(Object.getOwnPropertyDescriptor([].values(), 'next')));
out.push('string iter tag ' + Object.prototype.toString.call(''[Symbol.iterator]()));
out.push('AsyncIP @@asyncIterator ' + d(AsyncIP, Symbol.asyncIterator));
out.push('AsyncIP fn name/len ' + f(AsyncIP[Symbol.asyncIterator]));
out.push('AsyncIP fn returns this ' + (AsyncIP[Symbol.asyncIterator].call(7) === 7));
out.push('Iterator tag ' + d(Iterator.prototype, Symbol.toStringTag));
out.push('Iterator ctor ' + d(Iterator.prototype, 'constructor'));
out.push('Iterator tag value ' + Iterator.prototype[Symbol.toStringTag] + ' ' + Object.prototype.toString.call(Iterator.prototype));
out.push('Iterator ctor value ' + (Iterator.prototype.constructor === Iterator));
out.push('Iterator home tag set ' + t(() => { Iterator.prototype[Symbol.toStringTag] = 'x'; return 'ok'; }));
out.push('Iterator home ctor set ' + t(() => { Iterator.prototype.constructor = 1; return 'ok'; }));
out.push('Iterator derived ctor set ' + t(() => { const o = Object.create(Iterator.prototype); o.constructor = 5; return d(o, 'constructor'); }));
out.push('Iterator derived tag set ' + t(() => { const o = Object.create(Iterator.prototype); o[Symbol.toStringTag] = 'Z'; return Object.prototype.toString.call(o) + ' ' + d(o, Symbol.toStringTag); }));
out.push('Console hasInstance ' + d(console.Console, Symbol.hasInstance));
out.push('Console instanceof ' + [console instanceof console.Console, Object.create(console) instanceof console.Console, ({}) instanceof console.Console, new console.Console({ stdout: { write() {} } }) instanceof console.Console].join(' '));
for (const n of ['setTimeout', 'setInterval', 'setImmediate', 'clearTimeout', 'clearInterval', 'clearImmediate', 'queueMicrotask']) {
  const g = globalThis[n];
  out.push(n + ' prototype ' + d(g, 'prototype') + ' ctor ' + d(g.prototype, 'constructor') + ' same ' + (g.prototype.constructor === g));
}
console.log(out.join('\n'));"##) {
        let why = match why {
            crate::Ctl::Throw(v) => it.to_str(&v).to_string(),
            _ => "non-throw completion".into(),
        };
        panic!("probe threw: {why}");
    }
    let expected = r##"AIP tag v:"Array Iterator" w0e0c1
AIP next v:next/0 w1e0c1
AIP.next own prototype NONE
AIP.next call on non-iterator THREW TypeError
AIP proto is Iterator.prototype true
array iter next [7,8] undefined
string iter tag [object String Iterator]
AsyncIP @@asyncIterator v:[Symbol.asyncIterator]/0 w1e0c1
AsyncIP fn name/len [Symbol.asyncIterator]/0
AsyncIP fn returns this true
Iterator tag get:get [Symbol.toStringTag]/0 set:set [Symbol.toStringTag]/1 w0e0c1
Iterator ctor get:get constructor/0 set:set constructor/1 w0e0c1
Iterator tag value Iterator [object Iterator]
Iterator ctor value true
Iterator home tag set THREW TypeError
Iterator home ctor set THREW TypeError
Iterator derived ctor set v:5 w1e1c1
Iterator derived tag set [object Z] v:"Z" w1e1c1
Console hasInstance v:value/1 w0e0c0
Console instanceof true true false true
setTimeout prototype v:[object Object] w1e0c0 ctor v:setTimeout/2 w1e0c1 same true
setInterval prototype v:[object Object] w1e0c0 ctor v:setInterval/2 w1e0c1 same true
setImmediate prototype v:[object Object] w1e0c0 ctor v:setImmediate/1 w1e0c1 same true
clearTimeout prototype v:[object Object] w1e0c0 ctor v:clearTimeout/1 w1e0c1 same true
clearInterval prototype v:[object Object] w1e0c0 ctor v:clearInterval/1 w1e0c1 same true
clearImmediate prototype v:[object Object] w1e0c0 ctor v:clearImmediate/1 w1e0c1 same true
queueMicrotask prototype v:[object Object] w1e0c0 ctor v:queueMicrotask/1 w1e0c1 same true"##
        .trim_end_matches('\n');
    assert_eq!(it.out.join("\n"), expected);
}

/// Stream, text, Blob, Request and FormData/Headers shape rows: the accessors are prototype getters with
/// Node's flags, `Blob.prototype.stream` reads the blob, and the FormData/Headers methods carry `prototype`.
/// The expected text was recorded with `node` 22 from the same script.
const SHAPE_PROBE: &str = r##"const out = [];
const d = (o, k) => {
  const label = typeof k === 'symbol' ? 'sym' : k;
  const x = Object.getOwnPropertyDescriptor(o, k);
  if (!x) return label + ' none';
  if ('get' in x) return [label, 'accessor', x.enumerable, x.configurable, x.get ? x.get.name : '-', x.get ? x.get.length : '-'].join(' ');
  return [label, 'data', x.writable, x.enumerable, x.configurable, typeof x.value, typeof x.value === 'function' ? '' : String(x.value)].join(' ');
};
out.push(d(TextDecoder.prototype, 'encoding'), d(TextDecoder.prototype, 'fatal'), d(TextDecoder.prototype, 'ignoreBOM'));
out.push(d(TextDecoderStream.prototype, 'readable'), d(TextDecoderStream.prototype, 'writable'), d(TextDecoderStream.prototype, 'fatal'), d(TextDecoderStream.prototype, 'encoding'), d(TextDecoderStream.prototype, 'ignoreBOM'));
out.push(d(TextEncoderStream.prototype, 'readable'), d(TextEncoderStream.prototype, 'writable'), d(TextEncoderStream.prototype, 'encoding'));
out.push(d(TransformStream.prototype, 'readable'), d(TransformStream.prototype, 'writable'));
out.push(d(CompressionStream.prototype, 'readable'), d(DecompressionStream.prototype, 'writable'));
out.push(d(WritableStreamDefaultController.prototype, 'signal'));
out.push(d(ByteLengthQueuingStrategy.prototype, 'size'), d(CountQueuingStrategy.prototype, 'size'));
out.push(d(Request.prototype, 'attribute'), d(Request.prototype, 'isHistoryNavigation'), d(Request.prototype, 'isReloadNavigation'));
out.push(d(Blob.prototype, 'stream'), d(FormData.prototype, Symbol.toStringTag));
out.push(d(FormData.prototype, Symbol.iterator), d(FormData.prototype, 'keys'), d(Headers.prototype, 'values'));
out.push('keys proto ' + typeof FormData.prototype.keys.prototype + ' ' + typeof Headers.prototype.forEach.prototype + ' ' + typeof FormData.prototype[Symbol.iterator].prototype);
out.push('fatal ' + new TextDecoder('utf-8', { fatal: true, ignoreBOM: true }).fatal + ' ' + new TextDecoder('utf-8', { ignoreBOM: true }).ignoreBOM + ' ' + new TextDecoder('latin1').encoding);
out.push('tes ' + new TextEncoderStream().encoding + ' ' + new TextDecoderStream('utf-8', { fatal: true }).fatal);
out.push('ts ' + (new TransformStream().readable instanceof ReadableStream) + ' ' + (new TransformStream().writable instanceof WritableStream));
out.push('size ' + new ByteLengthQueuingStrategy({ highWaterMark: 1 }).size(new Uint8Array(3)) + ' ' + new CountQueuingStrategy({ highWaterMark: 1 }).size('x'));
out.push('sizeFn ' + (new CountQueuingStrategy({ highWaterMark: 2 }).size === new CountQueuingStrategy({ highWaterMark: 3 }).size));
out.push('fdtag ' + Object.prototype.toString.call(new FormData()));
out.push('wsdc ' + typeof Object.getOwnPropertyDescriptor(WritableStreamDefaultController.prototype, 'signal').get);
const probeStream = new Blob(['ab', 'cd']).stream();
out.push('blobstream ' + (probeStream instanceof ReadableStream));
(async () => {
  const reader = new Blob(['ab', 'cd']).stream().getReader();
  let text = '';
  for (;;) { const r = await reader.read(); if (r.done) break; text += new TextDecoder().decode(r.value); }
  out.push('blobtext ' + text);
  console.log(out.join('\n'));
})();"##;
const SHAPE_EXPECTED: &str = r##"encoding accessor true true get encoding 0
fatal accessor true true get fatal 0
ignoreBOM accessor true true get ignoreBOM 0
readable accessor true true get readable 0
writable accessor true true get writable 0
fatal accessor true true get fatal 0
encoding accessor true true get encoding 0
ignoreBOM accessor true true get ignoreBOM 0
readable accessor true true get readable 0
writable accessor true true get writable 0
encoding accessor true true get encoding 0
readable accessor true true get readable 0
writable accessor true true get writable 0
readable accessor true true get readable 0
writable accessor true true get writable 0
signal accessor true true get signal 0
size accessor true true get size 0
size accessor true true get size 0
attribute data false true false undefined undefined
isHistoryNavigation accessor true true get isHistoryNavigation 0
isReloadNavigation accessor true true get isReloadNavigation 0
stream data true true true function 
sym data false false true string FormData
sym data true false true function 
keys data true true true function 
values data true true true function 
keys proto object object object
fatal true true windows-1252
tes utf-8 true
ts true true
size 3 1
sizeFn true
fdtag [object FormData]
wsdc function
blobstream true
blobtext abcd"##;

#[test]
fn stream_text_blob_request_and_formdata_shapes_match_node() {
    let mut it = Interp::new();
    it.run(SHAPE_PROBE).expect("probe runs");
    it.run_event_loop().expect("probe drains its promise");
    assert_eq!(it.out.join("\n"), SHAPE_EXPECTED);
}
