fn url_snapshot(url: &url::Url) -> String {
    serde_json::json!({
        "href": url.as_str(),
        "protocol": format!("{}:", url.scheme()),
        "username": url.username(),
        "password": url.password().unwrap_or(""),
        "hostname": url.host_str().unwrap_or(""),
        "port": url.port().map_or(String::new(), |port| port.to_string()),
        "pathname": url.path(),
        "search": url.query().filter(|query| !query.is_empty()).map_or(String::new(), |query| format!("?{query}")),
        "hash": url.fragment().filter(|fragment| !fragment.is_empty()).map_or(String::new(), |fragment| format!("#{fragment}")),
        "origin": url.origin().ascii_serialization(),
        "opaque": url.cannot_be_a_base(),
    }).to_string()
}

fn host_url_parse(input: String, base: String, has_base: bool) -> String {
    let result = if has_base {
        url::Url::parse(&base).and_then(|base| base.join(&input))
    } else {
        url::Url::parse(&input)
    };
    result.map_or_else(|_| "null".to_owned(), |url| url_snapshot(&url))
}

fn host_url_set(href: String, property: String, value: String) -> String {
    let Ok(mut url) = url::Url::parse(&href) else { return "null".to_owned() };
    let accepted = match property.as_str() {
        "protocol" => url.set_scheme(value.strip_suffix(':').unwrap_or(&value)).is_ok(),
        "username" => url.set_username(&value).is_ok(),
        "password" => url.set_password(Some(&value)).is_ok(),
        "hostname" => {
            if value.contains(':') && !(value.starts_with('[') && value.ends_with(']')) { false }
            else { url.set_host(Some(&value)).is_ok() }
        },
        "host" => {
            if value.bytes().any(|byte| matches!(byte, b'/' | b'\\' | b'?' | b'#' | b'@')) {
                false
            } else {
                let authority = format!("{}://{value}/", url.scheme());
                if let Ok(parsed) = url::Url::parse(&authority) {
                    if let Some(host) = parsed.host_str() {
                        let mut next = url.clone();
                        let has_port = if value.starts_with('[') {
                            value.find(']').is_some_and(|end| value.as_bytes().get(end + 1) == Some(&b':'))
                        } else {
                            value.contains(':')
                        };
                        if next.set_host(Some(host)).is_ok()
                            && (!has_port || next.set_port(parsed.port()).is_ok()) {
                            url = next;
                            true
                        } else { false }
                    } else { false }
                } else { false }
            }
        }
        "port" => {
            if value.is_empty() { url.set_port(None).is_ok() }
            else if value.bytes().all(|byte| byte.is_ascii_digit()) {
                value.parse::<u16>().is_ok_and(|port| url.set_port(Some(port)).is_ok())
            } else { false }
        }
        "pathname" => {
            if url.cannot_be_a_base() { false }
            else { url.set_path(&value); true }
        }
        "search" => {
            if value.is_empty() { url.set_query(None); }
            else { url.set_query(Some(value.strip_prefix('?').unwrap_or(&value))); }
            true
        }
        "hash" => {
            if value.is_empty() { url.set_fragment(None); }
            else { url.set_fragment(Some(value.strip_prefix('#').unwrap_or(&value))); }
            true
        }
        _ => false,
    };
    if accepted { url_snapshot(&url) } else { "null".to_owned() }
}

fn cli_eval(arguments: &[String]) -> Option<(usize, Option<&str>)> {
    let index = arguments
        .iter()
        .enumerate()
        .skip(1)
        .take_while(|(_, argument)| argument.starts_with('-'))
        .find_map(|(index, argument)| (argument == "-e").then_some(index))?;
    Some((index, arguments.get(index + 1).map(String::as_str)))
}

fn cli_script(arguments: &[String]) -> Option<(usize, &str)> {
    let index = arguments
        .iter()
        .enumerate()
        .skip(1)
        .find_map(|(index, argument)| (!argument.starts_with('-')).then_some(index))?;
    let script = arguments.get(index)?;
    matches!(std::path::Path::new(script).extension().and_then(|value| value.to_str()), Some("js" | "cjs"))
        .then_some((index, script.as_str()))
        .filter(|(_, script)| std::path::Path::new(script).is_file())
}

/// Configures QuickJS's growable-`SharedArrayBuffer` allocator hooks,
/// which quickjs-ng leaves unset by default (so `sab.grow(...)` throws
/// "growable SharedArrayBuffer requires SAB allocator hooks"). thaw's
/// runtime can hand this storage to worker agents, so the allocation is
/// zeroed and its ownership count is atomic.
fn install_shared_array_buffer_functions(ctx: &Ctx<'_>) {
    use std::{
        ffi::c_void,
        sync::atomic::{AtomicUsize, Ordering},
    };

    #[repr(C)]
    struct Header {
        refcount: AtomicUsize,
        size: usize,
    }

    unsafe extern "C" fn allocate(_opaque: *mut c_void, size: u64) -> *mut c_void {
        let Ok(size) = usize::try_from(size) else {
            return std::ptr::null_mut();
        };
        let header = std::mem::size_of::<Header>();
        let Some(allocation_size) = header.checked_add(size) else {
            return std::ptr::null_mut();
        };
        let Ok(layout) = std::alloc::Layout::from_size_align(allocation_size, 16) else {
            return std::ptr::null_mut();
        };
        let base = unsafe { std::alloc::alloc_zeroed(layout) };
        if base.is_null() {
            return std::ptr::null_mut();
        }
        unsafe {
            (base as *mut Header).write(Header {
                refcount: AtomicUsize::new(1),
                size,
            })
        };
        unsafe { base.add(header).cast() }
    }

    unsafe extern "C" fn release(_opaque: *mut c_void, pointer: *mut c_void) {
        if pointer.is_null() {
            return;
        }
        let header = std::mem::size_of::<Header>();
        let base = unsafe { (pointer as *mut u8).sub(header) };
        let entry = base as *mut Header;
        if unsafe { (*entry).refcount.fetch_sub(1, Ordering::AcqRel) } == 1 {
            let size = unsafe { (*entry).size };
            if let Some(allocation_size) = header.checked_add(size) {
                if let Ok(layout) = std::alloc::Layout::from_size_align(allocation_size, 16) {
                    unsafe { std::alloc::dealloc(base, layout) };
                }
            }
        }
    }

    unsafe extern "C" fn duplicate(_opaque: *mut c_void, pointer: *mut c_void) {
        if pointer.is_null() {
            return;
        }
        let base = unsafe { (pointer as *mut u8).sub(std::mem::size_of::<Header>()) };
        unsafe {
            (*(base as *mut Header))
                .refcount
                .fetch_add(1, Ordering::Relaxed)
        };
    }

    let functions = rquickjs::qjs::JSSharedArrayBufferFunctions {
        sab_alloc: Some(allocate),
        sab_free: Some(release),
        sab_dup: Some(duplicate),
        sab_opaque: std::ptr::null_mut(),
    };
    let raw = ctx.as_raw().as_ptr();
    let runtime = unsafe { rquickjs::qjs::JS_GetRuntime(raw) };
    unsafe { rquickjs::qjs::JS_SetSharedArrayBufferFunctions(runtime, &functions) };
}

// Source graphs and completion promises are owned by the QuickJS context.
// QuickJS itself owns declared/evaluated modules in its per-runtime cache.
struct NativeBundleSource<'js> {
    source: String,
    unmarked_source: String,
    imports: std::collections::HashMap<String, String>,
    sequence: u64,
    linked: bool,
    module: Option<rquickjs::Module<'js, rquickjs::module::Declared>>,
}

struct NativeBundleOpaqueEdge {
    sequence: u64,
    importer: String,
    specifier: String,
    target: String,
    factory: String,
    asynchronous: bool,
}

struct NativeBundleOriginModule {
    sequence: u64,
    owner: String,
}

struct NativeBundleOwner<'js> {
    next_sequence: Cell<u64>,
    sources: RefCell<HashMap<String, NativeBundleSource<'js>>>,
    markers: RefCell<HashMap<String, String>>,
    entries: RefCell<HashMap<String, rquickjs::Promise<'js>>>,
    mixed_callbacks: RefCell<HashMap<u64, (Function<'js>, Function<'js>)>>,
    opaque_edges: RefCell<HashMap<String, NativeBundleOpaqueEdge>>,
    origin_modules: RefCell<HashMap<String, NativeBundleOriginModule>>,
    then: Function<'js>,
    object_is: Function<'js>,
}

unsafe impl<'js> rquickjs::JsLifetime<'js> for NativeBundleOwner<'js> {
    // These module handles belong to this runtime/context. During runtime
    // teardown rquickjs clears userdata and its opaque JS values before
    // JS_FreeRuntime; no handle escapes to another runtime.
    type Changed<'to> = NativeBundleOwner<'to>;
}

#[cfg(test)]
#[test]
fn native_bundle_marker_preserves_source_prefix_and_line_position() {
    let marker = "thaw-linked:1:first";
    assert_eq!(native_bundle_marked_source("export const value = 1;", marker),
        "import \"thaw-linked:1:first\";export const value = 1;");
    assert_eq!(native_bundle_marked_source("\u{feff}#! /usr/bin/env thaw\nexport const value = 1;", marker),
        "\u{feff}#! /usr/bin/env thaw\nimport \"thaw-linked:1:first\";export const value = 1;");
    assert_eq!(native_bundle_marked_source("#! /usr/bin/env thaw", marker),
        "#! /usr/bin/env thaw\nimport \"thaw-linked:1:first\";");
}

fn native_bundle_marker(token: &str) -> String {
    format!("thaw-linked:{token}")
}

fn native_bundle_marked_source(source: &str, marker: &str) -> String {
    let import = format!("import {};", serde_json::to_string(marker).expect("marker is serializable"));
    let bom = if source.starts_with('\u{feff}') { '\u{feff}'.len_utf8() } else { 0 };
    let after_hashbang = if source[bom..].starts_with("#!") {
        let Some(at) = source[bom..].find('\n') else { return format!("{source}\n{import}"); };
        bom + at + 1
    } else { bom };
    format!("{}{}{}", &source[..after_hashbang], import, &source[after_hashbang..])
}

struct NativeBundleLinkedMarker;

impl rquickjs::module::ModuleDef for NativeBundleLinkedMarker {
    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &rquickjs::module::Exports<'js>) -> rquickjs::Result<()> {
        let marker: String = exports.module().name()?;
        let owner = ctx.userdata::<NativeBundleOwner<'js>>()
            .ok_or_else(|| native_bundle_error("missing native bundle owner"))?;
        let token = owner.markers.borrow().get(&marker).cloned()
            .ok_or_else(|| native_bundle_error("unknown linked marker"))?;
        let mut sources = owner.sources.borrow_mut();
        let source = sources.get_mut(&token)
            .ok_or_else(|| native_bundle_error("missing marked source"))?;
        source.linked = true;
        Ok(())
    }
}

// A separate module per importer/specifier edge publishes the CJS value at
// the engine's original dependency evaluation point. The underlying CJS cache
// still runs a shared target factory only once.
struct NativeBundleOpaqueShell;

impl rquickjs::module::ModuleDef for NativeBundleOpaqueShell {
    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &rquickjs::module::Exports<'js>) -> rquickjs::Result<()> {
        let name: String = exports.module().name()?;
        let (callback, importer, specifier, target, factory) = {
            let owner = ctx.userdata::<NativeBundleOwner<'js>>()
                .ok_or_else(|| native_bundle_error("missing native bundle owner"))?;
            let edges = owner.opaque_edges.borrow();
            let edge = edges.get(&name)
                .ok_or_else(|| native_bundle_error("unknown opaque import edge"))?;
            let callback = owner.mixed_callbacks.borrow().get(&edge.sequence)
                .map(|callbacks| callbacks.0.clone())
                .ok_or_else(|| native_bundle_error("missing opaque import callback"))?;
            (callback, edge.importer.clone(), edge.specifier.clone(),
                edge.target.clone(), edge.factory.clone())
        };
        // Never hold owner or RefCell guards while the CJS factory executes.
        let _: Value<'js> = callback.call((importer, specifier, target, factory, false))?;
        Ok(())
    }
}

// An asynchronous opaque edge has a JS module shell with top-level await.
// Its only import is this private capability, bound to the edge by the
// resolver. A synchronous opaque edge retains NativeBundleOpaqueShell.
struct NativeBundleOpaqueLoad;

impl rquickjs::module::ModuleDef for NativeBundleOpaqueLoad {
    fn declare<'js>(decl: &rquickjs::module::Declarations<'js>) -> rquickjs::Result<()> {
        decl.declare("load")?;
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &rquickjs::module::Exports<'js>) -> rquickjs::Result<()> {
        let name: String = exports.module().name()?;
        let shell = name.strip_suffix("/load")
            .ok_or_else(|| native_bundle_error("invalid opaque load capability"))?;
        let (callback, importer, specifier, target, factory) = {
            let owner = ctx.userdata::<NativeBundleOwner<'js>>()
                .ok_or_else(|| native_bundle_error("missing native bundle owner"))?;
            let edges = owner.opaque_edges.borrow();
            let edge = edges.get(shell).filter(|edge| edge.asynchronous)
                .ok_or_else(|| native_bundle_error("unknown asynchronous opaque edge"))?;
            let callback = owner.mixed_callbacks.borrow().get(&edge.sequence)
                .map(|callbacks| callbacks.0.clone())
                .ok_or_else(|| native_bundle_error("missing opaque import callback"))?;
            (callback, edge.importer.clone(), edge.specifier.clone(),
                edge.target.clone(), edge.factory.clone())
        };
        let load = Function::new(ctx.clone(), move || -> rquickjs::Result<Value<'js>> {
            callback.call((importer.clone(), specifier.clone(), target.clone(), factory.clone(), true))
        })?;
        exports.export("load", load)?;
        Ok(())
    }
}

struct NativeBundleOriginCapability;

impl rquickjs::module::ModuleDef for NativeBundleOriginCapability {
    fn declare<'js>(decl: &rquickjs::module::Declarations<'js>) -> rquickjs::Result<()> {
        decl.declare("origin")?;
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &rquickjs::module::Exports<'js>) -> rquickjs::Result<()> {
        let name: String = exports.module().name()?;
        let (callback, raw_key) = {
            let owner = ctx.userdata::<NativeBundleOwner<'js>>()
                .ok_or_else(|| native_bundle_error("missing native bundle owner"))?;
            let origins = owner.origin_modules.borrow();
            let record = origins.get(&name)
                .ok_or_else(|| native_bundle_error("unknown origin capability"))?;
            let callback = owner.mixed_callbacks.borrow().get(&record.sequence)
                .map(|callbacks| callbacks.1.clone())
                .ok_or_else(|| native_bundle_error("missing origin callback"))?;
            (callback, record.owner.clone())
        };
        let origin: Value<'js> = callback.call((raw_key,))?;
        exports.export("origin", origin)?;
        Ok(())
    }
}

fn read_native_bundle_export<'js>(ctx: Ctx<'js>, entry: String, key: String, name: rquickjs::function::Opt<String>) -> rquickjs::Result<Value<'js>> {
    let module = {
        let owner = ctx.userdata::<NativeBundleOwner<'js>>()
            .ok_or_else(|| native_bundle_error("missing native bundle owner"))?;
        let entry_source = owner.sources.borrow().get(&entry).map(|source| source.sequence)
            .ok_or_else(|| native_bundle_error("missing native entry"))?;
        // Registry origin graphs use canonical raw keys. Keep the old fully
        // minted form for existing host tests, but never trust its sequence:
        // the source record below must belong to this bound entry.
        let key = if key.starts_with("thaw-bundle:") { key }
            else { format!("thaw-bundle:{entry_source}:{key}") };
        let sources = owner.sources.borrow();
        let source = sources.get(&key)
            .filter(|source| source.sequence == entry_source && source.linked)
            .ok_or_else(|| native_bundle_error("unlinked or foreign native source"))?;
        source.module.clone().ok_or_else(|| native_bundle_error("missing native module"))?
    };
    // The marker runs after QuickJS has linked this source. An omitted name
    // returns its cached namespace; a named getter still enforces lexical TDZ.
    let namespace = module.namespace()?;
    match name.0 {
        Some(name) => namespace.get(name),
        None => Ok(namespace.into_value()),
    }
}

fn native_bundle_error(message: &str) -> rquickjs::Error {
    rquickjs::Error::new_from_js_message("native bundle", "valid bundle", message)
}

// A computed query/fragment has the same source as its registered base, but
// it is a distinct ESM instance. Keep its marker, origin and opaque imports
// distinct while ordinary static dependencies retain their registered keys.
fn native_bundle_instance_token<'js>(
    ctx: &Ctx<'js>, sequence: u64, key: &str, factory: &str,
) -> rquickjs::Result<String> {
    if key.contains('\0') || factory.contains('\0') {
        return Err(native_bundle_error("NUL in native module key"));
    }
    if key != factory && !key.strip_prefix(factory)
        .is_some_and(|suffix| suffix.starts_with('?') || suffix.starts_with('#')) {
        return Err(native_bundle_error("invalid native instance suffix"));
    }
    let prefix = format!("thaw-bundle:{sequence}:");
    let token = format!("{prefix}{key}");
    let owner = ctx.userdata::<NativeBundleOwner<'js>>()
        .ok_or_else(|| native_bundle_error("missing native bundle owner"))?;
    if let Some(existing) = owner.sources.borrow().get(&token) {
        return if existing.sequence == sequence { Ok(token) }
            else { Err(native_bundle_error("foreign native bundle target")) };
    }
    if owner.opaque_edges.borrow().contains_key(&token)
        || owner.origin_modules.borrow().contains_key(&token) {
        return Err(native_bundle_error("native instance key collision"));
    }
    let suffix = key.strip_prefix(factory)
        .filter(|suffix| suffix.starts_with('?') || suffix.starts_with('#'))
        .ok_or_else(|| native_bundle_error("uncollected native module instance"))?;
    if suffix.is_empty() || factory.is_empty() {
        return Err(native_bundle_error("uncollected native module instance"));
    }
    let base = format!("{prefix}{factory}");
    let (unmarked_source, mut imports) = {
        let sources = owner.sources.borrow();
        let source = sources.get(&base)
            .filter(|source| source.sequence == sequence)
            .ok_or_else(|| native_bundle_error("foreign native bundle factory"))?;
        (source.unmarked_source.clone(), source.imports.clone())
    };
    let marker = native_bundle_marker(&token);
    if imports.contains_key(&marker) || owner.markers.borrow().contains_key(&marker) {
        return Err(native_bundle_error("reserved native instance marker"));
    }
    let mut rewrites = HashMap::<String, String>::new();
    let mut opaque = Vec::<(String, NativeBundleOpaqueEdge)>::new();
    let mut origins = Vec::<(String, NativeBundleOriginModule)>::new();
    for (index, target) in imports.values_mut().enumerate() {
        if let Some(rewritten) = rewrites.get(target) {
            *target = rewritten.clone();
            continue;
        }
        let original = target.clone();
        let original_token = format!("{prefix}{original}");
        let alias = if let Some(edge) = owner.opaque_edges.borrow().get(&original_token) {
            if edge.sequence != sequence || edge.importer != factory {
                return Err(native_bundle_error("foreign opaque instance edge"));
            }
            let alias = format!("{key}\u{1f}opaque:{index}");
            opaque.push((format!("{prefix}{alias}"), NativeBundleOpaqueEdge {
                sequence, importer: key.to_owned(), specifier: edge.specifier.clone(),
                target: edge.target.clone(), factory: edge.factory.clone(),
                asynchronous: edge.asynchronous,
            }));
            Some(alias)
        } else if let Some(origin) = owner.origin_modules.borrow().get(&original_token) {
            if origin.sequence != sequence || origin.owner != factory {
                return Err(native_bundle_error("foreign native origin capability"));
            }
            let alias = format!("{key}\u{1f}origin:{index}");
            origins.push((format!("{prefix}{alias}"), NativeBundleOriginModule {
                sequence, owner: key.to_owned(),
            }));
            Some(alias)
        } else {
            None
        };
        if let Some(alias) = alias {
            rewrites.insert(original, alias.clone());
            *target = alias;
        }
    }
    let mut planned = std::collections::HashSet::new();
    for (target, _) in &opaque {
        if !planned.insert(target.clone()) || !planned.insert(format!("{target}/load")) {
            return Err(native_bundle_error("native instance edge collision"));
        }
    }
    for (target, _) in &origins {
        if !planned.insert(target.clone()) {
            return Err(native_bundle_error("native instance origin collision"));
        }
    }
    if planned.iter().any(|target|
        owner.sources.borrow().contains_key(target)
            || owner.opaque_edges.borrow().contains_key(target)
            || owner.origin_modules.borrow().contains_key(target)) {
        return Err(native_bundle_error("native instance target collision"));
    }
    owner.sources.borrow_mut().insert(token.clone(), NativeBundleSource {
        source: native_bundle_marked_source(&unmarked_source, &marker),
        unmarked_source, imports, sequence, linked: false, module: None,
    });
    owner.opaque_edges.borrow_mut().extend(opaque);
    owner.origin_modules.borrow_mut().extend(origins);
    owner.markers.borrow_mut().insert(marker, token.clone());
    Ok(token)
}

fn register_native_bundle<'js>(
    ctx: Ctx<'js>, payload: String,
    opaque_load: rquickjs::function::Opt<Function<'js>>,
    origin_for: rquickjs::function::Opt<Function<'js>>,
) -> rquickjs::Result<Function<'js>> {
    if opaque_load.0.is_some() != origin_for.0.is_some() {
        return Err(native_bundle_error("incomplete mixed bundle callbacks"));
    }
    let data: serde_json::Value = serde_json::from_str(&payload)
        .map_err(|_| native_bundle_error("invalid JSON"))?;
    let main = data.get("main").and_then(serde_json::Value::as_str)
        .ok_or_else(|| native_bundle_error("missing main"))?;
    let modules = data.get("modules").and_then(serde_json::Value::as_array)
        .ok_or_else(|| native_bundle_error("missing modules"))?;
    let mut records = HashMap::<String, (String, HashMap<String, String>)>::new();
    for module in modules {
        let key = module.get("key").and_then(serde_json::Value::as_str)
            .ok_or_else(|| native_bundle_error("missing module key"))?;
        if key.contains('\0') { return Err(native_bundle_error("NUL in module key")); }
        let source = module.get("source").and_then(serde_json::Value::as_str)
            .ok_or_else(|| native_bundle_error("missing module source"))?;
        let imports = module.get("imports").and_then(serde_json::Value::as_object)
            .ok_or_else(|| native_bundle_error("missing module imports"))?;
        let mut edges = HashMap::new();
        for (specifier, target) in imports {
            let target = target.as_str()
                .ok_or_else(|| native_bundle_error("invalid import target"))?;
            if specifier.contains('\0') || target.contains('\0') {
                return Err(native_bundle_error("NUL in import"));
            }
            if edges.insert(specifier.clone(), target.to_owned()).is_some() {
                return Err(native_bundle_error("duplicate import"));
            }
        }
        if records.insert(key.to_owned(), (source.to_owned(), edges)).is_some() {
            return Err(native_bundle_error("duplicate module key"));
        }
    }
    let mut opaque_keys = std::collections::HashSet::new();
    if let Some(keys) = data.get("opaqueKeys") {
        for value in keys.as_array().ok_or_else(|| native_bundle_error("invalid opaque keys"))? {
            let key = value.as_str().ok_or_else(|| native_bundle_error("invalid opaque key"))?;
            if key.contains('\0') || records.contains_key(key) || !opaque_keys.insert(key.to_owned()) {
                return Err(native_bundle_error("duplicate or invalid opaque key"));
            }
        }
    }
    let mut shells = HashMap::<String, (String, String, String, String, bool)>::new();
    if let Some(edges) = data.get("opaqueEdges") {
        for value in edges.as_array().ok_or_else(|| native_bundle_error("invalid opaque edges"))? {
            let field = |name| value.get(name).and_then(serde_json::Value::as_str)
                .ok_or_else(|| native_bundle_error("invalid opaque edge"));
            let key = field("key")?;
            let importer = field("importer")?;
            let specifier = field("specifier")?;
            let target = field("target")?;
            let factory = field("factory")?;
            let asynchronous = value.get("asynchronous").and_then(serde_json::Value::as_bool)
                .ok_or_else(|| native_bundle_error("invalid opaque edge async flag"))?;
            if [key, importer, specifier, target, factory].iter().any(|part| part.contains('\0'))
                || records.contains_key(key) || !records.contains_key(importer)
                || !opaque_keys.contains(target)
                || shells.insert(key.to_owned(), (importer.to_owned(), specifier.to_owned(),
                    target.to_owned(), factory.to_owned(), asynchronous)).is_some() {
                return Err(native_bundle_error("invalid opaque edge target"));
            }
        }
    }
    let mut origin_modules = HashMap::<String, String>::new();
    if let Some(origins) = data.get("origins") {
        for value in origins.as_array().ok_or_else(|| native_bundle_error("invalid origins"))? {
            let key = value.get("key").and_then(serde_json::Value::as_str)
                .ok_or_else(|| native_bundle_error("invalid origin key"))?;
            let owner = value.get("owner").and_then(serde_json::Value::as_str)
                .ok_or_else(|| native_bundle_error("invalid origin owner"))?;
            if key.contains('\0') || owner.contains('\0') || records.contains_key(key)
                || shells.contains_key(key) || !records.contains_key(owner)
                || origin_modules.insert(key.to_owned(), owner.to_owned()).is_some() {
                return Err(native_bundle_error("invalid origin module"));
            }
        }
    }
    if (!shells.is_empty() || !origin_modules.is_empty()) && opaque_load.0.is_none() {
        return Err(native_bundle_error("mixed bundle needs callbacks"));
    }
    if shells.iter().any(|(key, (importer, specifier, _, _, _))|
        records.get(importer).and_then(|(_, imports)| imports.get(specifier)) != Some(key)) {
        return Err(native_bundle_error("opaque edge does not match import map"));
    }
    if !records.contains_key(main) || records.values().any(|(_, edges)|
        edges.values().any(|target| !records.contains_key(target)
            && !shells.contains_key(target) && !origin_modules.contains_key(target))) {
        return Err(native_bundle_error("missing entry or import target"));
    }
    let owner = ctx.userdata::<NativeBundleOwner<'js>>()
        .ok_or_else(|| native_bundle_error("missing native bundle owner"))?;
    let sequence = owner.next_sequence.get().checked_add(1)
        .ok_or_else(|| native_bundle_error("bundle sequence overflow"))?;
    let prefix = format!("thaw-bundle:{sequence}:");
    let token = format!("{prefix}{main}");
    let marked = records.keys().map(|key| {
        let token = format!("{prefix}{key}");
        (native_bundle_marker(&token), token)
    }).collect::<HashMap<_, _>>();
    if records.values().any(|(_, edges)| edges.keys().any(|name|
        marked.contains_key(name))) {
        return Err(native_bundle_error("reserved marker import"));
    }
    // Reserve the name before constructing a QuickJS Function, and release
    // the userdata guard before that engine call.
    owner.next_sequence.set(sequence);
    drop(owner);
    // Return the bound evaluator itself, not a guessable entry-name string.
    let bound_token = token.clone();
    let evaluate = Function::new(ctx.clone(), move |ctx: Ctx<'js>| {
        eval_native_entry(ctx, bound_token.clone())
    })?;
    let reader_entry = token.clone();
    let reader = Function::new(ctx.clone(), move |ctx: Ctx<'js>, key: String, name: rquickjs::function::Opt<String>| {
        read_native_bundle_export(ctx, reader_entry.clone(), key, name)
    })?;
    evaluate.prop("readNative", reader)?;
    let native_sequence = sequence;
    let native_ready = Function::new(ctx.clone(), move |ctx: Ctx<'js>, key: String, factory: rquickjs::function::Opt<String>| {
        let factory = factory.0.unwrap_or_else(|| key.clone());
        let token = native_bundle_instance_token(&ctx, native_sequence, &key, &factory)?;
        eval_native_entry(ctx, token)
    })?;
    evaluate.prop("evalNative", native_ready)?;
    let sync_sequence = sequence;
    let native_sync = Function::new(ctx.clone(), move |ctx: Ctx<'js>, key: String, factory: rquickjs::function::Opt<String>| -> rquickjs::Result<Value<'js>> {
        let factory = factory.0.unwrap_or_else(|| key.clone());
        let token = native_bundle_instance_token(&ctx, sync_sequence, &key, &factory)?;
        let ready = eval_native_entry(ctx.clone(), token.clone())?;
        if ready.state() == rquickjs::promise::PromiseState::Rejected {
            return ready.result::<Value<'js>>().expect("rejected promise has a result");
        }
        let declared = {
            let owner = ctx.userdata::<NativeBundleOwner<'js>>()
                .ok_or_else(|| native_bundle_error("missing native bundle owner"))?;
            let sources = owner.sources.borrow();
            sources.get(&token).and_then(|source| source.module.clone())
                .ok_or_else(|| native_bundle_error("native module was not declared"))?
        };
        // QuickJS returns the cached evaluation promise for a module already
        // evaluating or evaluated; this does not run the module body twice.
        let (_, evaluation) = declared.eval()?;
        match evaluation.state() {
            rquickjs::promise::PromiseState::Resolved =>
                read_native_bundle_export(ctx, format!("thaw-bundle:{sync_sequence}:{}", key), key,
                    rquickjs::function::Opt(None)),
            rquickjs::promise::PromiseState::Rejected =>
                evaluation.result::<Value<'js>>().expect("rejected promise has a result"),
            rquickjs::promise::PromiseState::Pending =>
                Err(rquickjs::Exception::throw_type(&ctx, "Cannot synchronously require an async module")),
        }
    })?;
    evaluate.prop("evalNativeSync", native_sync)?;
    let owner = ctx.userdata::<NativeBundleOwner<'js>>()
        .ok_or_else(|| native_bundle_error("missing native bundle owner"))?;
    {
        let mut sources = owner.sources.borrow_mut();
        for (key, (source, imports)) in records {
            let token = format!("{prefix}{key}");
            sources.insert(token.clone(), NativeBundleSource {
                source: native_bundle_marked_source(&source, &native_bundle_marker(&token)),
                unmarked_source: source,
                imports, sequence, linked: false, module: None,
            });
        }
    }
    owner.opaque_edges.borrow_mut().extend(shells.into_iter().map(|(key, (importer, specifier, target, factory, asynchronous))| {
        (format!("{prefix}{key}"), NativeBundleOpaqueEdge {
            sequence, importer, specifier, target, factory, asynchronous,
        })
    }));
    owner.origin_modules.borrow_mut().extend(origin_modules.into_iter().map(|(key, module_owner)| {
        (format!("{prefix}{key}"), NativeBundleOriginModule { sequence, owner: module_owner })
    }));
    owner.markers.borrow_mut().extend(marked);
    if let (Some(load), Some(origin)) = (opaque_load.0, origin_for.0) {
        owner.mixed_callbacks.borrow_mut().insert(sequence, (load, origin));
    }
    Ok(evaluate)
}

fn reject_native_entry_error<'js>(
    ctx: &Ctx<'js>, reject: &Function<'js>, error: rquickjs::Error,
) -> rquickjs::Result<()> {
    if matches!(error, rquickjs::Error::Exception) {
        reject.call::<_, ()>((ctx.catch(),))
    } else {
        reject.call::<_, ()>((error.to_string(),))
    }
}

fn eval_native_entry<'js>(ctx: Ctx<'js>, token: String) -> rquickjs::Result<rquickjs::Promise<'js>> {
    let (source, declared_module, intrinsic_then) = {
        let owner = ctx.userdata::<NativeBundleOwner<'js>>()
            .ok_or_else(|| native_bundle_error("missing native bundle owner"))?;
        if let Some(ready) = owner.entries.borrow().get(&token) {
            return Ok(ready.clone());
        }
        let (source, declared_module) = {
            let sources = owner.sources.borrow();
            sources.get(&token).map(|record| (record.source.clone(), record.module.clone()))
                .ok_or_else(|| native_bundle_error("missing native entry source"))?
        };
        (source, declared_module, owner.then.clone())
    };
    let (ready, resolve, reject) = rquickjs::Promise::new(&ctx)?;
    {
        let owner = ctx.userdata::<NativeBundleOwner<'js>>()
            .ok_or_else(|| native_bundle_error("missing native bundle owner"))?;
        owner.entries.borrow_mut().insert(token.clone(), ready.clone());
    }
    // The ready promise is visible to reentrant entry calls before module
    // evaluation can run user code. Never hold userdata guards across JS calls.
    let setup = (|| -> rquickjs::Result<()> {
        let declared = if let Some(module) = declared_module {
            module
        } else {
            rquickjs::Module::declare(ctx.clone(), token.clone(), source)?
        };
        {
            let owner = ctx.userdata::<NativeBundleOwner<'js>>()
                .ok_or_else(|| native_bundle_error("missing native bundle owner"))?;
            let mut sources = owner.sources.borrow_mut();
            sources.get_mut(&token)
                .ok_or_else(|| native_bundle_error("missing native entry source"))?
                .module = Some(declared.clone());
        }
        let (evaluated, evaluation) = declared.eval()?;
        // Namespace creation does not read its bindings. Its object can be
        // retained in a callback without storing a typed Module in userdata.
        let namespace = evaluated.namespace()?;
        let fulfillment_reject = reject.clone();
        let fulfillment_ctx = ctx.clone();
        let on_fulfilled = Function::new(ctx.clone(), move || -> rquickjs::Result<()> {
            if let Err(error) = resolve.call::<_, ()>((namespace.clone(),)) {
                reject_native_entry_error(&fulfillment_ctx, &fulfillment_reject, error)?;
            }
            Ok(())
        })?;
        let callback_reject = reject.clone();
        let on_rejected = Function::new(ctx.clone(), move |reason: Value<'js>| -> rquickjs::Result<()> {
            callback_reject.call((reason,))
        })?;
        intrinsic_then.call::<_, Value>((rquickjs::function::This(evaluation.clone()), on_fulfilled, on_rejected))?;
        Ok(())
    })();
    if let Err(error) = setup {
        // Keep the original JavaScript exception value when one exists.
        reject_native_entry_error(&ctx, &reject, error)?;
    }
    Ok(ready)
}

struct BundleModuleResolver;

impl rquickjs::loader::Resolver for BundleModuleResolver {
    fn resolve<'js>(
        &mut self, ctx: &Ctx<'js>, base: &str, name: &str,
        _attributes: Option<rquickjs::loader::ImportAttributes<'js>>,
    ) -> rquickjs::Result<String> {
        let owner = ctx.userdata::<NativeBundleOwner<'js>>()
            .ok_or_else(|| rquickjs::Error::new_resolving(base, name))?;
        let sources = owner.sources.borrow();
        if let Some(token) = owner.markers.borrow().get(name) {
            if token == base && sources.contains_key(base) { return Ok(name.to_owned()); }
            return Err(rquickjs::Error::new_resolving(base, name));
        }
        if name.starts_with("thaw-linked:") {
            return Err(rquickjs::Error::new_resolving(base, name));
        }
        if name == "__thaw_private_opaque_load"
            && owner.opaque_edges.borrow().get(base).is_some_and(|edge| edge.asynchronous) {
            return Ok(format!("{base}/load"));
        }
        let parent = sources.get(base)
            .ok_or_else(|| rquickjs::Error::new_resolving(base, name))?;
        let target = parent.imports.get(name)
            .ok_or_else(|| rquickjs::Error::new_resolving(base, name))?;
        let resolved = format!("thaw-bundle:{}:{target}", parent.sequence);
        if !sources.contains_key(&resolved)
            && !owner.opaque_edges.borrow().contains_key(&resolved)
            && !owner.origin_modules.borrow().contains_key(&resolved) {
            return Err(rquickjs::Error::new_resolving(base, name));
        }
        Ok(resolved)
    }
}

struct BundleModuleLoader;

impl rquickjs::loader::Loader for BundleModuleLoader {
    fn load<'js>(
        &mut self, ctx: &Ctx<'js>, name: &str,
        _attributes: Option<rquickjs::loader::ImportAttributes<'js>>,
    ) -> rquickjs::Result<rquickjs::Module<'js, rquickjs::module::Declared>> {
        let source = {
            let owner = ctx.userdata::<NativeBundleOwner<'js>>()
                .ok_or_else(|| rquickjs::Error::new_loading(name))?;
            let marker = owner.markers.borrow().contains_key(name);
            if marker {
                drop(owner);
                return rquickjs::Module::declare_def::<NativeBundleLinkedMarker, _>(ctx.clone(), name);
            }
            let opaque_async = owner.opaque_edges.borrow().get(name).map(|edge| edge.asynchronous);
            if let Some(asynchronous) = opaque_async {
                drop(owner);
                if asynchronous {
                    let source = "import { load } from '__thaw_private_opaque_load'; await load(); export {};";
                    return rquickjs::Module::declare(ctx.clone(), name, source);
                }
                return rquickjs::Module::declare_def::<NativeBundleOpaqueShell, _>(ctx.clone(), name);
            }
            if let Some(shell) = name.strip_suffix("/load") {
                let load_capability = owner.opaque_edges.borrow().get(shell)
                    .is_some_and(|edge| edge.asynchronous);
                if load_capability {
                    drop(owner);
                    return rquickjs::Module::declare_def::<NativeBundleOpaqueLoad, _>(ctx.clone(), name);
                }
            }
            if owner.origin_modules.borrow().contains_key(name) {
                drop(owner);
                return rquickjs::Module::declare_def::<NativeBundleOriginCapability, _>(ctx.clone(), name);
            }
            let sources = owner.sources.borrow();
            sources.get(name).map(|record| record.source.clone())
                .ok_or_else(|| rquickjs::Error::new_loading(name))?
        };
        let module = rquickjs::Module::declare(ctx.clone(), name, source)?;
        {
            let owner = ctx.userdata::<NativeBundleOwner<'js>>()
                .ok_or_else(|| rquickjs::Error::new_loading(name))?;
            let mut sources = owner.sources.borrow_mut();
            sources.get_mut(name)
                .ok_or_else(|| rquickjs::Error::new_loading(name))?
                .module = Some(module.clone());
        }
        Ok(module)
    }
}

// These functions and their WeakMap belong to this QuickJS runtime. Keeping
// them in userdata releases both before JS_FreeRuntime, unlike a separate TLS.
struct NativeCallbackIdentityBridge<'js> {
    register: Function<'js>,
    same: Function<'js>,
    intern_object: Function<'js>,
    object_pointer: Function<'js>,
    object_lookup: Function<'js>,
    method_lookup: Function<'js>,
    method_intern: Function<'js>,
    define_callback_length: Function<'js>,
}

unsafe impl<'js> rquickjs::JsLifetime<'js> for NativeCallbackIdentityBridge<'js> {
    type Changed<'to> = NativeCallbackIdentityBridge<'to>;
}

fn same_native_callback<'js>(ctx: Ctx<'js>, left: Value<'js>, right: Value<'js>) -> bool {
    let Some(bridge) = ctx.userdata::<NativeCallbackIdentityBridge<'js>>() else {
        return false;
    };
    let same = bridge.same.clone();
    drop(bridge);
    same.call((left, right)).unwrap_or(false)
}

fn install_native_callback_identity_bridge(ctx: &Ctx<'_>) -> rquickjs::Result<()> {
    // Capture invocation machinery as well as methods: Function.prototype.call
    // and WeakMap.prototype methods are user-mutable after initialization.
    let pair: Array = ctx.eval(r#"(function() {
        const map = new WeakMap();
        const apply = Reflect.apply;
        const objectConstructor = Object;
        const objectCreate = Object.create;
        const defineProperty = Object.defineProperty;
        const set = WeakMap.prototype.set;
        const get = WeakMap.prototype.get;
        const has = WeakMap.prototype.has;
        const objectOwners = new WeakMap();
        const objectCache = new Map();
        const methodCache = new Map();
        const mapGet = Map.prototype.get;
        const mapSet = Map.prototype.set;
        const mapDelete = Map.prototype.delete;
        const WeakReference = WeakRef;
        const weakDeref = WeakRef.prototype.deref;
        const Finalizer = FinalizationRegistry;
        const finalizerRegister = FinalizationRegistry.prototype.register;
        const stringIndexOf = String.prototype.indexOf;
        const stringSlice = String.prototype.slice;
        const arrayPush = Array.prototype.push;
        const number = Number;
        const isSafeInteger = Number.isSafeInteger;
        const layoutSegments = layout => {
            const fields = [];
            let cursor = 0;
            while (cursor < layout.length) {
                const start = cursor;
                for (let part = 0; part < 2; part++) {
                    const colon = apply(stringIndexOf, layout, [':', cursor]);
                    if (colon < 0) return null;
                    const bytes = number(apply(stringSlice, layout, [cursor, colon]));
                    if (!isSafeInteger(bytes) || bytes < 0) return null;
                    cursor = colon + 1 + bytes * 2;
                    if (cursor > layout.length) return null;
                }
                apply(arrayPush, fields, [apply(stringSlice, layout, [start, cursor])]);
            }
            return fields;
        };
        const layoutContains = (actual, expected) => {
            if (actual === expected) return true;
            const have = layoutSegments(actual);
            const want = layoutSegments(expected);
            if (!have || !want) return false;
            const used = objectCreate(null);
            for (let desired = 0; desired < want.length; desired++) {
                let found = false;
                for (let index = 0; index < have.length; index++) {
                    if (!used[index] && have[index] === want[desired]) {
                        used[index] = true;
                        found = true;
                        break;
                    }
                }
                if (!found) return false;
            }
            return true;
        };
        const finalized = new Finalizer(({pointer, weak}) => {
            if (apply(mapGet, objectCache, [pointer]) === weak)
                apply(mapDelete, objectCache, [pointer]);
        });
        const methodFinalized = new Finalizer(({token, weak}) => {
            if (apply(mapGet, methodCache, [token]) === weak)
                apply(mapDelete, methodCache, [token]);
        });
        return [
            (wrapper, token) => apply(set, map, [wrapper, token]),
            (left, right) => apply(has, map, [left]) &&
                apply(has, map, [right]) &&
                apply(get, map, [left]) === apply(get, map, [right]),
            (pointer, layout, proposed, guardian) => {
                const cached = apply(mapGet, objectCache, [pointer]);
                const existing = cached && apply(weakDeref, cached, []);
                if (existing) {
                    if (apply(get, objectOwners, [existing]).layout !== layout)
                        throw new TypeError('native object layout changed while its wrapper is live');
                    return existing;
                }
                const weak = new WeakReference(proposed);
                apply(set, objectOwners, [proposed, {pointer, layout, guardian}]);
                apply(mapSet, objectCache, [pointer, weak]);
                apply(finalizerRegister, finalized, [proposed, {pointer, weak}]);
                return proposed;
            },
            (wrapper, expected) => {
                const owner = apply(get, objectOwners, [wrapper]);
                return owner && layoutContains(owner.layout, expected)
                    ? owner.pointer : '';
            },
            (pointer, expected) => {
                const weak = apply(mapGet, objectCache, [pointer]);
                const existing = weak && apply(weakDeref, weak, []);
                const owner = existing && apply(get, objectOwners, [existing]);
                return owner && layoutContains(owner.layout, expected)
                    ? existing : undefined;
            },
            token => {
                const weak = apply(mapGet, methodCache, [token]);
                return weak && apply(weakDeref, weak, []);
            },
            (token, proposed) => {
                const weak = apply(mapGet, methodCache, [token]);
                const existing = weak && apply(weakDeref, weak, []);
                if (existing) return existing;
                const next = new WeakReference(proposed);
                apply(mapSet, methodCache, [token, next]);
                apply(finalizerRegister, methodFinalized, [proposed, {token, weak: next}]);
                return proposed;
            },
            (callback, length) => apply(defineProperty, objectConstructor, [callback, 'length', {value: length}]),
        ];
    })()"#)?;
    let register: Function = pair.get(0)?;
    let same: Function = pair.get(1)?;
    let intern_object: Function = pair.get(2)?;
    let object_pointer: Function = pair.get(3)?;
    let object_lookup: Function = pair.get(4)?;
    let method_lookup: Function = pair.get(5)?;
    let method_intern: Function = pair.get(6)?;
    let define_callback_length: Function = pair.get(7)?;
    ctx.store_userdata(NativeCallbackIdentityBridge {
        register, same, intern_object, object_pointer, object_lookup,
        method_lookup, method_intern, define_callback_length,
    })
        .expect("native callback identity bridge userdata is already borrowed");
    ctx.globals().prop("__thaw_same_native_callback",
        Function::new(ctx.clone(), same_native_callback)?)?;
    Ok(())
}

extern "C" {
    fn JS_GetAsyncContext(ctx: *mut rquickjs::qjs::JSContext) -> rquickjs::qjs::JSValue;
    fn JS_SwapAsyncContext(ctx: *mut rquickjs::qjs::JSContext,
                           next: rquickjs::qjs::JSValue) -> rquickjs::qjs::JSValue;
}

fn install_async_context_bootstrap<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<()> {
    let bridge = Object::new(ctx.clone())?;
    bridge.set("get", Function::new(ctx.clone(), |ctx: Ctx<'js>| -> Value<'js> {
        // The C API returns an owned duplicate from this exact runtime.
        unsafe { Value::from_raw(ctx.clone(), JS_GetAsyncContext(ctx.as_raw().as_ptr())) }
    })?)?;
    bridge.set("swap", Function::new(ctx.clone(), |ctx: Ctx<'js>, next: Value<'js>| -> Value<'js> {
        // Swap duplicates `next` and transfers ownership of the previous slot.
        unsafe { Value::from_raw(ctx.clone(), JS_SwapAsyncContext(ctx.as_raw().as_ptr(), next.as_raw())) }
    })?)?;
    // PLATFORM_GLOBALS captures these functions into its private IIFE. The
    // temporary property is removed before any user bundle is evaluated.
    ctx.globals().set("__thaw_async_context_bootstrap", bridge)
}

fn ensure_context() {
    JS.with(|cell| {
        let mut slot = cell.borrow_mut();
        slot.get_or_insert_with(|| {
            let runtime = Runtime::new().expect("failed to create a QuickJS runtime");
            runtime.set_loader(BundleModuleResolver, BundleModuleLoader);
            runtime.set_host_promise_rejection_tracker(Some(Box::new(track_js_promise_rejection)));
            let context = Context::full(&runtime).expect("failed to create a QuickJS context");
            context.with(|ctx| {
                install_shared_array_buffer_functions(&ctx);
                let intrinsic_then: Function = ctx.eval("Promise.prototype.then")
                    .expect("failed to capture intrinsic Promise.then");
                let intrinsic_object_is: Function = ctx.eval("Object.is")
                    .expect("failed to capture intrinsic Object.is");
                ctx.store_userdata(NativeBundleOwner {
                    next_sequence: Cell::new(0),
                    sources: RefCell::new(HashMap::new()),
                    markers: RefCell::new(HashMap::new()),
                    entries: RefCell::new(HashMap::new()),
                    mixed_callbacks: RefCell::new(HashMap::new()),
                    opaque_edges: RefCell::new(HashMap::new()),
                    origin_modules: RefCell::new(HashMap::new()),
                    then: intrinsic_then,
                    object_is: intrinsic_object_is,
                }).expect("native bundle owner userdata is already borrowed");
                ctx.globals().prop("__thaw_register_native_bundle",
                    Function::new(ctx.clone(), register_native_bundle)
                        .expect("failed to create native bundle registrar"))
                    .expect("failed to install native bundle registrar");
                ctx.globals()
                    .set(
                        "__thaw_os_thread_token",
                        format!("{:?}", std::thread::current().id()),
                    )
                    .expect("failed to install OS thread identity");
                ctx.globals()
                    .set(
                        "__thaw_host_env_json",
                        serde_json::to_string(
                            &std::env::vars().collect::<std::collections::BTreeMap<_, _>>(),
                        )
                        .expect("failed to serialize host environment"),
                    )
                    .expect("failed to install host environment");
                let arguments = std::env::args().collect::<Vec<_>>();
                let eval = cli_eval(&arguments);
                let script = cli_script(&arguments);
                let mut node_arguments = Vec::with_capacity(arguments.len() + 1);
                node_arguments.push(arguments.first().cloned().unwrap_or_default());
                if let Some((index, _)) = eval {
                    node_arguments.extend(arguments.iter().skip(index + 2).cloned());
                } else if let Some((index, _)) = script {
                    node_arguments.extend(arguments.iter().skip(index).cloned());
                } else {
                    node_arguments.extend(arguments.iter().cloned());
                }
                ctx.globals()
                    .set(
                        "__thaw_host_argv_json",
                        serde_json::to_string(&node_arguments)
                            .expect("failed to serialize host arguments"),
                    )
                    .expect("failed to install host arguments");
                ctx.globals()
                    .set(
                        "__thaw_host_exec_argv_json",
                        serde_json::to_string(if let Some((index, _)) = eval {
                            &arguments[1..arguments.len().min(index + 2)]
                        } else if let Some((index, _)) = script {
                            &arguments[1..index]
                        } else {
                            &[]
                        })
                        .expect("failed to serialize host exec arguments"),
                    )
                    .expect("failed to install host exec arguments");
                install_native_callback_identity_bridge(&ctx)
                    .expect("failed to install native callback identity bridge");
                install_napi_bridge(&ctx).expect("failed to install N-API bridge");
                ctx.globals().set("__thaw_url_parse", Function::new(ctx.clone(), host_url_parse)
                    .expect("failed to create URL parser")).expect("failed to install URL parser");
                ctx.globals().set("__thaw_url_set", Function::new(ctx.clone(), host_url_set)
                    .expect("failed to create URL setter")).expect("failed to install URL setter");
                let shared_env = HOST_WORKERS.with(|table| table.borrow().shared_env.clone());
                install_shared_environment_functions(&ctx, shared_env)
                    .expect("failed to install shared Worker environment accessors");
                let shared_env_init = Function::new(ctx.clone(), initialize_shared_environment)
                    .expect("failed to create shared Worker environment initializer");
                ctx.globals()
                    .set("__thaw_shared_env_init", shared_env_init)
                    .expect("failed to install shared Worker environment initializer");
                let stdout = Function::new(ctx.clone(), |text: crate::Wtf8String| {
                    print!("{}", crate::wtf8_display(&text.0));
                    let _ = io::stdout().flush();
                })
                .expect("failed to create JavaScript stdout writer");
                let stderr = Function::new(ctx.clone(), |text: crate::Wtf8String| {
                    eprint!("{}", crate::wtf8_display(&text.0));
                    let _ = io::stderr().flush();
                })
                .expect("failed to create JavaScript stderr writer");
                let raw_process_write = Function::new(ctx.clone(), |fd: i32, encoded: String| -> rquickjs::Result<()> {
                    if !matches!(fd, 1 | 2) || !encoded.len().is_multiple_of(2) || !encoded.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                        return Err(rquickjs::Error::new_from_js_message("process stream", "fd 1/2 and valid hex bytes", "invalid raw process output"));
                    }
                    let bytes = hex_decode(&encoded);
                    let result = if fd == 1 {
                        let mut output = io::stdout().lock();
                        output.write_all(&bytes).and_then(|()| output.flush())
                    } else {
                        let mut output = io::stderr().lock();
                        output.write_all(&bytes).and_then(|()| output.flush())
                    };
                    result.map_err(|error| rquickjs::Error::new_from_js_message("process stream", "writable fd 1/2", error.to_string()))
                })
                .expect("failed to create raw process stream writer");
                ctx.globals()
                    .set("__thaw_console_stdout", stdout)
                    .expect("failed to install JavaScript stdout writer");
                ctx.globals()
                    .set("__thaw_console_stderr", stderr)
                    .expect("failed to install JavaScript stderr writer");
                ctx.globals()
                    .set("__thaw_process_write_bytes", raw_process_write)
                    .expect("failed to install raw process stream writer");
                let stdin_start = Function::new(ctx.clone(), start_host_stdin)
                    .expect("failed to create process stdin starter");
                let stdin_poll = Function::new(ctx.clone(), poll_host_stdin)
                    .expect("failed to create process stdin poller");
                let stdin_active = Function::new(ctx.clone(), host_stdin_active)
                    .expect("failed to create process stdin activity probe");
                ctx.globals()
                    .set("__thaw_process_start_stdin", stdin_start)
                    .expect("failed to install process stdin starter");
                ctx.globals()
                    .set("__thaw_process_poll_stdin", stdin_poll)
                    .expect("failed to install process stdin poller");
                ctx.globals()
                    .set("__thaw_stdin_active", stdin_active)
                    .expect("failed to install process stdin activity probe");
                let stdin_raw_mode = Function::new(ctx.clone(), set_stdin_raw_mode)
                    .expect("failed to create process stdin raw-mode bridge");
                ctx.globals()
                    .set("__thaw_process_set_raw_mode", stdin_raw_mode)
                    .expect("failed to install process stdin raw-mode bridge");
                let is_tty = Function::new(ctx.clone(), |fd: i32| unsafe {
                    libc::isatty(fd) == 1
                })
                .expect("failed to create process TTY bridge");
                ctx.globals()
                    .set("__thaw_process_is_tty", is_tty)
                    .expect("failed to install process TTY bridge");
                let cwd = Function::new(ctx.clone(), || -> rquickjs::Result<String> {
                    std::env::current_dir()
                        .map(|path| path.to_string_lossy().into_owned())
                        .map_err(|error| {
                            rquickjs::Error::new_from_js_message("process", "cwd", error.to_string())
                        })
                })
                .expect("failed to create process cwd bridge");
                ctx.globals()
                    .set("__thaw_process_cwd", cwd)
                    .expect("failed to install process cwd bridge");
                let chdir =
                    Function::new(ctx.clone(), |path: crate::Wtf8String| -> rquickjs::Result<()> {
                        std::env::set_current_dir(String::from_utf8_lossy(&path.0).as_ref())
                            .map_err(|error| {
                                rquickjs::Error::new_from_js_message(
                                    "process",
                                    "chdir",
                                    error.to_string(),
                                )
                            })
                    })
                .expect("failed to create process chdir bridge");
                ctx.globals()
                    .set("__thaw_process_chdir", chdir)
                    .expect("failed to install process chdir bridge");
                let exit = Function::new(ctx.clone(), |code: i32| -> () {
                    // `std::process::exit` runs TLS destructors, which try to
                    // tear down this still-active QuickJS context.
                    restore_stdin_termios();
                    unsafe { libc::_exit(code) };
                })
                .expect("failed to create process exit bridge");
                ctx.globals()
                    .set("__thaw_process_exit", exit)
                    .expect("failed to install process exit bridge");
                let configure_signal = Function::new(ctx.clone(), configure_process_signal)
                    .expect("failed to create process signal bridge");
                ctx.globals()
                    .set("__thaw_process_configure_signal", configure_signal)
                    .expect("failed to install process signal bridge");
                let detach_array_buffer =
                    Function::new(ctx.clone(), |mut value: ArrayBuffer<'_>| {
                        value.detach();
                    })
                    .expect("failed to create ArrayBuffer detacher");
                ctx.globals()
                    .set("__thaw_detach_array_buffer", detach_array_buffer)
                    .expect("failed to install ArrayBuffer detacher");
                let quickjs_gc_function = Function::new(ctx.clone(), run_quickjs_gc)
                    .expect("failed to create QuickJS garbage collector trigger");
                ctx.globals()
                    .set("__thaw_gc", quickjs_gc_function)
                    .expect("failed to install QuickJS garbage collector trigger");
                #[cfg(feature = "wasm")]
                {
                let wasm_compile_function = Function::new(ctx.clone(), wasm_compile)
                    .expect("failed to create WebAssembly compiler");
                let wasm_instantiate_function = Function::new(ctx.clone(), wasm_instantiate)
                    .expect("failed to create WebAssembly instantiator");
                let wasm_custom_sections_function =
                    Function::new(ctx.clone(), wasm_custom_sections)
                        .expect("failed to create WebAssembly custom-section reader");
                let wasm_retain_import_function = Function::new(ctx.clone(), wasm_retain_import)
                    .expect("failed to create WebAssembly import retainer");
                let wasm_retain_funcref_function = Function::new(ctx.clone(), wasm_retain_funcref)
                    .expect("failed to create WebAssembly function-reference retainer");
                let wasm_restore_import_function = Function::new(ctx.clone(), wasm_restore_import)
                    .expect("failed to create WebAssembly import restorer");
                let wasm_retain_value_function = Function::new(ctx.clone(), wasm_retain_value)
                    .expect("failed to create WebAssembly externref retainer");
                let wasm_retain_value_at_function =
                    Function::new(ctx.clone(), wasm_retain_value_at)
                        .expect("failed to create WebAssembly externref reactivator");
                let wasm_restore_value_function = Function::new(ctx.clone(), wasm_restore_value)
                    .expect("failed to create WebAssembly externref restorer");
                let wasm_reference_stats_function =
                    Function::new(ctx.clone(), wasm_reference_stats)
                        .expect("failed to create WebAssembly reference statistics reader");
                let wasm_release_function = Function::new(ctx.clone(), wasm_release)
                    .expect("failed to create WebAssembly resource releaser");
                let wasm_release_pending_function =
                    Function::new(ctx.clone(), wasm_release_pending)
                        .expect("failed to create WebAssembly pending-resource releaser");
                let wasm_call_function = Function::new(ctx.clone(), wasm_call)
                    .expect("failed to create WebAssembly function caller");
                let wasm_call_funcref_function = Function::new(ctx.clone(), wasm_call_funcref)
                    .expect("failed to create WebAssembly function-reference caller");
                let wasm_export_funcref_function = Function::new(ctx.clone(), wasm_export_funcref)
                    .expect("failed to create WebAssembly exported function-reference reader");
                let wasm_global_function = Function::new(ctx.clone(), wasm_global)
                    .expect("failed to create WebAssembly global accessor");
                let wasm_global_create_function = Function::new(ctx.clone(), wasm_global_create)
                    .expect("failed to create WebAssembly global allocator");
                let wasm_global_retain_function = Function::new(ctx.clone(), wasm_global_retain)
                    .expect("failed to create WebAssembly global retainer");
                let wasm_memory_create_function = Function::new(ctx.clone(), wasm_memory_create)
                    .expect("failed to create WebAssembly memory allocator");
                let wasm_memory_function = Function::new(ctx.clone(), wasm_memory)
                    .expect("failed to create WebAssembly memory accessor");
                let wasm_table_create_function = Function::new(ctx.clone(), wasm_table_create)
                    .expect("failed to create WebAssembly table allocator");
                let wasm_table_retain_function = Function::new(ctx.clone(), wasm_table_retain)
                    .expect("failed to create WebAssembly table retainer");
                let wasm_table_function = Function::new(ctx.clone(), wasm_table)
                    .expect("failed to create WebAssembly table accessor");
                ctx.globals()
                    .set("__thaw_wasm_compile", wasm_compile_function)
                    .expect("failed to install WebAssembly compiler");
                ctx.globals()
                    .set("__thaw_wasm_instantiate", wasm_instantiate_function)
                    .expect("failed to install WebAssembly instantiator");
                ctx.globals()
                    .set("__thaw_wasm_custom_sections", wasm_custom_sections_function)
                    .expect("failed to install WebAssembly custom-section reader");
                ctx.globals()
                    .set("__thaw_wasm_retain_import", wasm_retain_import_function)
                    .expect("failed to install WebAssembly import retainer");
                ctx.globals()
                    .set("__thaw_wasm_retain_funcref", wasm_retain_funcref_function)
                    .expect("failed to install WebAssembly function-reference retainer");
                ctx.globals()
                    .set("__thaw_wasm_restore_import", wasm_restore_import_function)
                    .expect("failed to install WebAssembly import restorer");
                ctx.globals()
                    .set("__thaw_wasm_retain_value", wasm_retain_value_function)
                    .expect("failed to install WebAssembly externref retainer");
                ctx.globals()
                    .set("__thaw_wasm_retain_value_at", wasm_retain_value_at_function)
                    .expect("failed to install WebAssembly externref reactivator");
                ctx.globals()
                    .set("__thaw_wasm_restore_value", wasm_restore_value_function)
                    .expect("failed to install WebAssembly externref restorer");
                ctx.globals()
                    .set("__thaw_wasm_reference_stats", wasm_reference_stats_function)
                    .expect("failed to install WebAssembly reference statistics reader");
                ctx.globals()
                    .set("__thaw_wasm_release", wasm_release_function)
                    .expect("failed to install WebAssembly resource releaser");
                ctx.globals()
                    .set("__thaw_wasm_release_pending", wasm_release_pending_function)
                    .expect("failed to install WebAssembly pending-resource releaser");
                ctx.globals()
                    .set("__thaw_wasm_call", wasm_call_function)
                    .expect("failed to install WebAssembly function caller");
                ctx.globals()
                    .set("__thaw_wasm_call_funcref", wasm_call_funcref_function)
                    .expect("failed to install WebAssembly function-reference caller");
                ctx.globals()
                    .set("__thaw_wasm_export_funcref", wasm_export_funcref_function)
                    .expect("failed to install WebAssembly exported function-reference reader");
                ctx.globals()
                    .set("__thaw_wasm_global", wasm_global_function)
                    .expect("failed to install WebAssembly global accessor");
                ctx.globals()
                    .set("__thaw_wasm_global_create", wasm_global_create_function)
                    .expect("failed to install WebAssembly global allocator");
                ctx.globals()
                    .set("__thaw_wasm_global_retain", wasm_global_retain_function)
                    .expect("failed to install WebAssembly global retainer");
                ctx.globals()
                    .set("__thaw_wasm_memory_create", wasm_memory_create_function)
                    .expect("failed to install WebAssembly memory allocator");
                ctx.globals()
                    .set("__thaw_wasm_memory", wasm_memory_function)
                    .expect("failed to install WebAssembly memory accessor");
                ctx.globals()
                    .set("__thaw_wasm_table_create", wasm_table_create_function)
                    .expect("failed to install WebAssembly table allocator");
                ctx.globals()
                    .set("__thaw_wasm_table_retain", wasm_table_retain_function)
                    .expect("failed to install WebAssembly table retainer");
                ctx.globals()
                    .set("__thaw_wasm_table", wasm_table_function)
                    .expect("failed to install WebAssembly table accessor");
                }
                let worker_spawn = Function::new(
                    ctx.clone(),
                    |bundle_source: String,
                     source: String,
                     worker_data_json: String,
                     config_json: String,
                     thread_id: u32| {
                        spawn_host_worker(
                            bundle_source,
                            source,
                            worker_data_json,
                            config_json,
                            thread_id,
                        )
                    },
                )
                .expect("failed to create Worker spawner");
                let worker_send = Function::new(ctx.clone(), |handle: u32, payload: String| {
                    send_host_worker(handle, payload)
                })
                .expect("failed to create Worker sender");
                let worker_terminate =
                    Function::new(ctx.clone(), |handle: u32| terminate_host_worker(handle))
                        .expect("failed to create Worker terminator");
                let worker_ref = Function::new(
                    ctx.clone(),
                    |handle: u32, refed: bool| set_host_worker_refed(handle, refed),
                )
                .expect("failed to create Worker liveness updater");
                let worker_stdin =
                    Function::new(ctx.clone(), |handle: u32, payload: String, ended: bool| {
                        send_host_worker_stdin(handle, payload, ended)
                    })
                    .expect("failed to create Worker stdin sender");
                let worker_route_direct = Function::new(
                    ctx.clone(),
                    |origin: u32, target: u32, payload: String, source: u32, request: u64| {
                        route_host_worker_direct(origin, target, payload, source, request)
                    },
                )
                .expect("failed to create Worker direct router");
                let worker_parent_direct =
                    Function::new(ctx.clone(), |target: u32, payload: String, request: u64| {
                        route_parent_direct(target, payload, request)
                    })
                    .expect("failed to create parent direct router");
                let worker_direct_result =
                    Function::new(ctx.clone(), |handle: u32, request: u64, error: String| {
                        resolve_host_worker_direct(handle, request, error)
                    })
                    .expect("failed to create Worker direct resolver");
                let worker_port =
                    Function::new(ctx.clone(), |handle: u32, port: String, payload: String| {
                        send_host_worker_port(handle, port, payload)
                    })
                    .expect("failed to create Worker port sender");
                let worker_read_source =
                    Function::new(ctx.clone(), |path: String| -> rquickjs::Result<String> {
                        std::fs::read_to_string(&path).map(|mut source| {
                            // All worker JSON and package manifest readers share this boundary.
                            if source.starts_with('\u{feff}') {
                                source.drain(..'\u{feff}'.len_utf8());
                            }
                            source
                        }).map_err(|error| {
                            rquickjs::Error::new_from_js_message(
                                "Worker path",
                                "JavaScript source",
                                format!("failed to read `{path}`: {error}"),
                            )
                        })
                    })
                    .expect("failed to create Worker source reader");
                let worker_poll = Function::new(ctx.clone(), poll_host_workers)
                    .expect("failed to create Worker event poller");
                let worker_active = Function::new(ctx.clone(), host_workers_active)
                    .expect("failed to create Worker activity probe");
                let child_process = Function::new(
                    ctx.clone(),
                    |command: String, arguments: String, options: String| {
                        run_child_process(command, arguments, options)
                    },
                )
                .expect("failed to create child process runner");
                let child_spawn = Function::new(
                    ctx.clone(),
                    |command: String, arguments: String, options: String| {
                        spawn_host_child(command, arguments, options)
                    },
                )
                .expect("failed to create child process spawner");
                let child_stdin =
                    Function::new(ctx.clone(), |handle: u32, value: String, end: bool| {
                        send_host_child_stdin(handle, value, end)
                    })
                    .expect("failed to create child process stdin sender");
                let child_kill = Function::new(ctx.clone(), |handle: u32, signal: i32| {
                    kill_host_child(handle, signal)
                })
                .expect("failed to create child process killer");
                let child_poll = Function::new(ctx.clone(), poll_host_children)
                    .expect("failed to create child process poller");
                let child_active = Function::new(ctx.clone(), host_children_active)
                    .expect("failed to create child process activity probe");
                ctx.globals()
                    .set("__thaw_worker_spawn", worker_spawn)
                    .expect("failed to install Worker spawner");
                ctx.globals()
                    .set("__thaw_worker_send", worker_send)
                    .expect("failed to install Worker sender");
                ctx.globals()
                    .set("__thaw_worker_terminate", worker_terminate)
                    .expect("failed to install Worker terminator");
                ctx.globals()
                    .set("__thaw_worker_ref", worker_ref)
                    .expect("failed to install Worker liveness updater");
                ctx.globals()
                    .set("__thaw_worker_stdin", worker_stdin)
                    .expect("failed to install Worker stdin sender");
                ctx.globals()
                    .set("__thaw_worker_route_direct", worker_route_direct)
                    .expect("failed to install Worker direct router");
                ctx.globals()
                    .set("__thaw_worker_parent_direct", worker_parent_direct)
                    .expect("failed to install parent direct router");
                ctx.globals()
                    .set("__thaw_worker_direct_result", worker_direct_result)
                    .expect("failed to install Worker direct resolver");
                ctx.globals()
                    .set("__thaw_worker_port", worker_port)
                    .expect("failed to install Worker port sender");
                ctx.globals()
                    .set("__thaw_worker_read_source", worker_read_source)
                    .expect("failed to install Worker source reader");
                ctx.globals()
                    .set("__thaw_worker_poll", worker_poll)
                    .expect("failed to install Worker event poller");
                ctx.globals()
                    .set("__thaw_worker_active", worker_active)
                    .expect("failed to install Worker activity probe");
                ctx.globals()
                    .set("__thaw_child_process_sync", child_process)
                    .expect("failed to install child process runner");
                ctx.globals()
                    .set("__thaw_child_process_spawn", child_spawn)
                    .expect("failed to install child process spawner");
                ctx.globals()
                    .set("__thaw_child_process_stdin", child_stdin)
                    .expect("failed to install child process stdin sender");
                ctx.globals()
                    .set("__thaw_child_process_kill", child_kill)
                    .expect("failed to install child process killer");
                ctx.globals()
                    .set("__thaw_child_process_poll", child_poll)
                    .expect("failed to install child process poller");
                ctx.globals()
                    .set("__thaw_child_process_active", child_active)
                    .expect("failed to install child process activity probe");
                let random_hex = Function::new(ctx.clone(), |size: u32| {
                    let mut bytes = vec![0u8; size as usize];
                    getrandom::getrandom(&mut bytes).expect("OS random source failed");
                    hex_encode(&bytes)
                })
                .expect("failed to create JavaScript random source");
                let hash_hex = Function::new(ctx.clone(), |algorithm: String, value: String| {
                    hex_encode(&digest_bytes(&algorithm, &hex_decode(&value)))
                })
                .expect("failed to create JavaScript hash function");
                let hmac_hex = Function::new(
                    ctx.clone(),
                    |algorithm: String, key: String, value: String| {
                        hex_encode(&hmac_bytes(
                            &algorithm,
                            &hex_decode(&key),
                            &hex_decode(&value),
                        ))
                    },
                )
                .expect("failed to create JavaScript HMAC function");
                let pbkdf2_hex = Function::new(
                    ctx.clone(),
                    |algorithm: String,
                     password: String,
                     salt: String,
                     iterations: u32,
                     length: u32| {
                        hex_encode(&pbkdf2_bytes(
                            &algorithm,
                            &hex_decode(&password),
                            &hex_decode(&salt),
                            iterations,
                            length as usize,
                        ))
                    },
                )
                .expect("failed to create JavaScript PBKDF2 function");
                let scrypt_hex = Function::new(
                    ctx.clone(),
                    |password: String,
                     salt: String,
                     length: u32,
                     cost: u32,
                     block_size: u32,
                     parallelization: u32| {
                        if cost < 2 || !cost.is_power_of_two() {
                            return Err(rquickjs::Error::new_from_js_message(
                                "scrypt cost",
                                "power of two greater than one",
                                cost.to_string(),
                            ));
                        }
                        let params = scrypt::Params::new(
                            cost.ilog2() as u8,
                            block_size,
                            parallelization,
                            (length as usize).clamp(10, 64),
                        )
                        .map_err(|error| {
                            rquickjs::Error::new_from_js_message(
                                "scrypt options",
                                "valid scrypt options",
                                format!(
                                    "{error}: N={cost}, r={block_size}, p={parallelization}, length={length}"
                                ),
                            )
                        })?;
                        let mut output = vec![0; length as usize];
                        scrypt::scrypt(
                            &hex_decode(&password),
                            &hex_decode(&salt),
                            &params,
                            &mut output,
                        )
                        .map_err(|error| {
                            rquickjs::Error::new_from_js_message(
                                "scrypt input",
                                "derived key",
                                error.to_string(),
                            )
                        })?;
                        Ok(hex_encode(&output))
                    },
                )
                .expect("failed to create JavaScript scrypt function");
                let cipher_hex = Function::new(
                    ctx.clone(),
                    |algorithm: String,
                     key: String,
                     iv: String,
                     value: String,
                     decrypt: bool,
                     finalize: bool|
                     -> rquickjs::Result<String> {
                        if !algorithm.eq_ignore_ascii_case("aes-256-cbc") {
                            return Err(rquickjs::Error::new_from_js_message(
                                "cipher",
                                "aes-256-cbc",
                                algorithm,
                            ));
                        }
                        let key = hex_decode(&key);
                        let iv = hex_decode(&iv);
                        let value = hex_decode(&value);
                        if decrypt {
                            let cipher = cbc::Decryptor::<aes::Aes256>::new_from_slices(&key, &iv)
                                .map_err(|_| rquickjs::Error::new_from_js("key/iv", "AES-256-CBC"))?;
                            let output = if finalize {
                                cipher.decrypt_padded_vec_mut::<Pkcs7>(&value)
                            } else {
                                cipher.decrypt_padded_vec_mut::<NoPadding>(&value)
                            }
                                .map_err(|_| rquickjs::Error::new_from_js("ciphertext", "padded AES-256-CBC data"))?;
                            Ok(hex_encode(&output))
                        } else {
                            let cipher = cbc::Encryptor::<aes::Aes256>::new_from_slices(&key, &iv)
                                .map_err(|_| rquickjs::Error::new_from_js("key/iv", "AES-256-CBC"))?;
                            let output = if finalize {
                                cipher.encrypt_padded_vec_mut::<Pkcs7>(&value)
                            } else {
                                cipher.encrypt_padded_vec_mut::<NoPadding>(&value)
                            };
                            Ok(hex_encode(&output))
                        }
                    },
                )
                .expect("failed to create JavaScript cipher function");
                // Backs `createPrivateKey`/`createPublicKey`/`KeyObject.
                // asymmetricKeyType` and `createSign`/`createVerify`/
                // `crypto.sign`/`crypto.verify` (`asymmetric_crypto.rs`,
                // `platform_globals/buffer_crypto.js`) -- resolves PEM/
                // DER/passphrase-protected key material into a plain,
                // normalized PEM string every other crypto function here
                // consumes.
                let crypto_import_key = Function::new(
                    ctx.clone(),
                    |bytes_hex: String, is_der: bool, passphrase: String| {
                        crypto_import_key_json(&bytes_hex, is_der, &passphrase)
                    },
                )
                .expect("failed to create JavaScript key-import function");
                let crypto_asymmetric_sign = Function::new(
                    ctx.clone(),
                    |digest_algorithm: String, pem: String, data: String| {
                        crypto_asymmetric_sign_hex(&digest_algorithm, &pem, &hex_decode(&data))
                            .map(|signature| hex_encode(&signature))
                            .map_err(|error| {
                                rquickjs::Error::new_from_js_message("sign input", "valid key/data", error)
                            })
                    },
                )
                .expect("failed to create JavaScript asymmetric sign function");
                let crypto_asymmetric_sign_pss = Function::new(
                    ctx.clone(),
                    |digest_algorithm: String, pem: String, data: String| {
                        crypto_asymmetric_sign_pss_hex(&digest_algorithm, &pem, &hex_decode(&data))
                            .map(|signature| hex_encode(&signature))
                            .map_err(|error| {
                                rquickjs::Error::new_from_js_message("sign input", "valid RSA key/data", error)
                            })
                    },
                )
                .expect("failed to create JavaScript asymmetric PSS sign function");
                let crypto_asymmetric_verify = Function::new(
                    ctx.clone(),
                    |digest_algorithm: String, pem: String, data: String, signature: String| {
                        crypto_asymmetric_verify(
                            &digest_algorithm,
                            &pem,
                            &hex_decode(&data),
                            &hex_decode(&signature),
                        )
                        .map_err(|error| {
                            rquickjs::Error::new_from_js_message("verify input", "valid key/data", error)
                        })
                    },
                )
                .expect("failed to create JavaScript asymmetric verify function");
                let crypto_asymmetric_verify_pss = Function::new(
                    ctx.clone(),
                    |digest_algorithm: String, pem: String, data: String, signature: String| {
                        crypto_asymmetric_verify_pss(
                            &digest_algorithm,
                            &pem,
                            &hex_decode(&data),
                            &hex_decode(&signature),
                        )
                        .map_err(|error| {
                            rquickjs::Error::new_from_js_message("verify input", "valid RSA key/data", error)
                        })
                    },
                )
                .expect("failed to create JavaScript asymmetric PSS verify function");
                let crypto_asymmetric_encrypt = Function::new(
                    ctx.clone(),
                    |pem: String, oaep_digest: String, data: String| {
                        crypto_asymmetric_encrypt_hex(&pem, &oaep_digest, &hex_decode(&data))
                            .map(|value| hex_encode(&value))
                            .map_err(|error| {
                                rquickjs::Error::new_from_js_message(
                                    "encrypt input",
                                    "valid RSA key/data",
                                    error,
                                )
                            })
                    },
                )
                .expect("failed to create JavaScript asymmetric encrypt function");
                let crypto_asymmetric_decrypt = Function::new(
                    ctx.clone(),
                    |pem: String, oaep_digest: String, data: String| {
                        crypto_asymmetric_decrypt_hex(&pem, &oaep_digest, &hex_decode(&data))
                            .map(|value| hex_encode(&value))
                            .map_err(|error| {
                                rquickjs::Error::new_from_js_message(
                                    "decrypt input",
                                    "valid RSA key/data",
                                    error,
                                )
                            })
                    },
                )
                .expect("failed to create JavaScript asymmetric decrypt function");
                let crypto_asymmetric_private_encrypt = Function::new(
                    ctx.clone(),
                    |pem: String, data: String| {
                        crypto_asymmetric_private_encrypt_hex(&pem, &hex_decode(&data))
                            .map(|value| hex_encode(&value))
                            .map_err(|error| {
                                rquickjs::Error::new_from_js_message(
                                    "privateEncrypt input",
                                    "valid RSA private key/data",
                                    error,
                                )
                            })
                    },
                )
                .expect("failed to create JavaScript asymmetric privateEncrypt function");
                let crypto_asymmetric_public_decrypt = Function::new(
                    ctx.clone(),
                    |pem: String, data: String| {
                        crypto_asymmetric_public_decrypt_hex(&pem, &hex_decode(&data))
                            .map(|value| hex_encode(&value))
                            .map_err(|error| {
                                rquickjs::Error::new_from_js_message(
                                    "publicDecrypt input",
                                    "valid RSA key/data",
                                    error,
                                )
                            })
                    },
                )
                .expect("failed to create JavaScript asymmetric publicDecrypt function");
                let crypto_generate_key_pair = Function::new(
                    ctx.clone(),
                    |key_type: String,
                     modulus_bits_or_curve: String,
                     public_exponent: String,
                     private_key_type: String,
                     public_key_type: String| {
                        crypto_generate_key_pair_json(
                            &key_type,
                            &modulus_bits_or_curve,
                            &public_exponent,
                            &private_key_type,
                            &public_key_type,
                        )
                        .map_err(|error| {
                            rquickjs::Error::new_from_js_message(
                                "generateKeyPair input",
                                "valid key type/options",
                                error,
                            )
                        })
                    },
                )
                .expect("failed to create JavaScript key-pair generation function");
                let crypto_diffie_hellman = Function::new(
                    ctx.clone(),
                    |private_pem: String, public_pem: String| {
                        crypto_diffie_hellman_hex(&private_pem, &public_pem)
                            .map(|secret| hex_encode(&secret))
                            .map_err(|error| {
                                rquickjs::Error::new_from_js_message(
                                    "diffieHellman input",
                                    "a matching pair of EC or X25519 keys",
                                    error,
                                )
                            })
                    },
                )
                .expect("failed to create JavaScript Diffie-Hellman function");
                let crypto_ecdh_generate_keys = Function::new(ctx.clone(), |curve: String| {
                    crypto_ecdh_generate_keys_hex(&curve).map_err(|error| {
                        rquickjs::Error::new_from_js_message(
                            "ECDH curve",
                            "a supported EC curve name",
                            error,
                        )
                    })
                })
                .expect("failed to create JavaScript ECDH key-generation function");
                let crypto_ecdh_public_from_private = Function::new(
                    ctx.clone(),
                    |curve: String, private_key_hex: String| {
                        crypto_ecdh_public_from_private_hex(&curve, &private_key_hex).map_err(
                            |error| {
                                rquickjs::Error::new_from_js_message(
                                    "ECDH private key",
                                    "a valid raw EC private key",
                                    error,
                                )
                            },
                        )
                    },
                )
                .expect("failed to create JavaScript ECDH public-key-derivation function");
                let crypto_ecdh_compute_secret = Function::new(
                    ctx.clone(),
                    |curve: String, private_key_hex: String, public_key_hex: String| {
                        crypto_ecdh_compute_secret_hex(&curve, &private_key_hex, &public_key_hex)
                            .map_err(|error| {
                                rquickjs::Error::new_from_js_message(
                                    "ECDH computeSecret input",
                                    "a valid raw EC private key and peer public key",
                                    error,
                                )
                            })
                    },
                )
                .expect("failed to create JavaScript ECDH computeSecret function");
                let hpack_huffman_encode = Function::new(ctx.clone(), |value: String| {
                    let mut output = Vec::new();
                    httlib_huffman::encode(&hex_decode(&value), &mut output)
                        .map(|()| hex_encode(&output))
                        .map_err(|error| {
                            rquickjs::Error::new_from_js_message(
                                "HPACK bytes",
                                "Huffman bytes",
                                error.to_string(),
                            )
                        })
                })
                .expect("failed to create HPACK Huffman encoder");
                let hpack_huffman_decode = Function::new(ctx.clone(), |value: String| {
                    let mut output = Vec::new();
                    httlib_huffman::decode(
                        &hex_decode(&value),
                        &mut output,
                        httlib_huffman::DecoderSpeed::FourBits,
                    )
                    .map(|()| hex_encode(&output))
                    .map_err(|error| {
                        rquickjs::Error::new_from_js_message(
                            "HPACK Huffman bytes",
                            "decoded bytes",
                            error.to_string(),
                        )
                    })
                })
                .expect("failed to create HPACK Huffman decoder");
                let zlib_hex = Function::new(
                    ctx.clone(),
                    |operation: String,
                     format: String,
                     value: String|
                     -> rquickjs::Result<String> {
                        let input = hex_decode(&value);
                        let result = if operation == "compress" {
                            compress_bytes(&format, &input)
                        } else {
                            decompress_bytes(&format, &input)
                        };
                        result.map(|bytes| hex_encode(&bytes)).map_err(|error| {
                            rquickjs::Error::new_from_js_message(
                                "zlib",
                                "Buffer",
                                error.to_string(),
                            )
                        })
                    },
                )
                .expect("failed to create JavaScript compression function");
                let web_zlib_streams = Rc::new(RefCell::new(HashMap::<u32, WebZlibStream>::new()));
                let next_web_zlib_stream = Rc::new(Cell::new(1u32));
                let create_web_zlib_stream = {
                    let streams = Rc::clone(&web_zlib_streams);
                    let next = Rc::clone(&next_web_zlib_stream);
                    Function::new(
                        ctx.clone(),
                        move |operation: String, format: String| -> rquickjs::Result<u32> {
                            let stream =
                                WebZlibStream::new(&operation, &format).map_err(|error| {
                                    rquickjs::Error::new_from_js_message(
                                        "zlib stream",
                                        "handle",
                                        error.to_string(),
                                    )
                                })?;
                            let handle = next.get();
                            next.set(handle.wrapping_add(1).max(1));
                            streams.borrow_mut().insert(handle, stream);
                            Ok(handle)
                        },
                    )
                    .expect("failed to create streaming zlib allocator")
                };
                let write_web_zlib_stream = {
                    let streams = Rc::clone(&web_zlib_streams);
                    Function::new(
                        ctx.clone(),
                        move |handle: u32,
                              value: String,
                              finish: bool|
                              -> rquickjs::Result<String> {
                            let input = hex_decode(&value);
                            let result = if finish {
                                let stream =
                                    streams.borrow_mut().remove(&handle).ok_or_else(|| {
                                        rquickjs::Error::new_from_js_message(
                                            "zlib stream",
                                            "handle",
                                            "unknown stream handle",
                                        )
                                    })?;
                                stream.finish(&input)
                            } else {
                                streams
                                    .borrow_mut()
                                    .get_mut(&handle)
                                    .ok_or_else(|| {
                                        rquickjs::Error::new_from_js_message(
                                            "zlib stream",
                                            "handle",
                                            "unknown stream handle",
                                        )
                                    })?
                                    .write_and_flush(&input)
                            };
                            result.map(|bytes| hex_encode(&bytes)).map_err(|error| {
                                rquickjs::Error::new_from_js_message(
                                    "zlib stream",
                                    "Buffer",
                                    error.to_string(),
                                )
                            })
                        },
                    )
                    .expect("failed to create streaming zlib writer")
                };
                let drop_web_zlib_stream = {
                    let streams = Rc::clone(&web_zlib_streams);
                    Function::new(ctx.clone(), move |handle: u32| {
                        streams.borrow_mut().remove(&handle).is_some()
                    })
                    .expect("failed to create streaming zlib closer")
                };
                let fs_handles = Rc::new(RefCell::new(FsHandleTable::new()));
                let fs_function = {
                    let handles = Rc::clone(&fs_handles);
                    Function::new(
                        ctx.clone(),
                        move |operation: String, path: String, value: String, recursive: bool| {
                            host_fs(operation, path, value, recursive, &mut handles.borrow_mut())
                        },
                    )
                    .expect("failed to create JavaScript filesystem function")
                };
                let tcp_connect = Function::new(ctx.clone(), |host: String, port: u32| {
                    net_connect(&host, port as u16)
                })
                .expect("failed to create JavaScript TCP connector");
                let tcp_write = Function::new(ctx.clone(), |handle: u32, value: String| {
                    net_write(handle, &hex_decode(&value))
                })
                .expect("failed to create JavaScript TCP writer");
                let tcp_finish = Function::new(ctx.clone(), |handle: u32| net_finish(handle))
                    .expect("failed to create JavaScript TCP finisher");
                let tcp_shutdown =
                    Function::new(ctx.clone(), |handle: u32| net_shutdown_write(handle))
                        .expect("failed to create JavaScript TCP shutdown function");
                let tcp_poll_read = Function::new(ctx.clone(), |handle: u32, retain_on_eof: rquickjs::function::Opt<bool>| {
                    if retain_on_eof.0.unwrap_or(false) { net_poll_read_impl(handle, true) } else { net_poll_read(handle) }
                })
                    .expect("failed to create JavaScript TCP polling reader");
                let tcp_destroy = Function::new(ctx.clone(), |handle: u32| net_destroy(handle))
                    .expect("failed to create JavaScript TCP closer");
                let tcp_listen = Function::new(ctx.clone(), |host: String, port: u32| {
                    net_listen(&host, port as u16)
                })
                .expect("failed to create JavaScript TCP listener");
                let tcp_accept = Function::new(ctx.clone(), |handle: u32| net_accept(handle))
                    .expect("failed to create JavaScript TCP acceptor");
                let tcp_poll_accept =
                    Function::new(ctx.clone(), |handle: u32| net_poll_accept(handle))
                        .expect("failed to create JavaScript TCP polling acceptor");
                let tcp_read = Function::new(ctx.clone(), |handle: u32| net_read_all(handle))
                    .expect("failed to create JavaScript TCP reader");
                let tcp_close_listener =
                    Function::new(ctx.clone(), |handle: u32| net_close_listener(handle))
                        .expect("failed to create JavaScript TCP listener closer");
                let udp_bind_function = Function::new(ctx.clone(), |host: String, port: u32| {
                    match u16::try_from(port) { Ok(port) => udp_bind(&host, port), Err(_) => "err|EINVAL|invalid UDP port".to_string() }
                })
                .expect("failed to create JavaScript UDP binder");
                let udp_send_function = Function::new(
                    ctx.clone(),
                    |handle: u32, value: String, host: String, port: u32| {
                        match u16::try_from(port) { Ok(port) if port != 0 => udp_send(handle, &hex_decode(&value), &host, port), _ => "err|EINVAL|invalid UDP port".to_string() }
                    },
                )
                .expect("failed to create JavaScript UDP sender");
                let udp_receive_function =
                    Function::new(ctx.clone(), |handle: u32| udp_receive(handle))
                        .expect("failed to create JavaScript UDP receiver");
                let udp_connect_function = Function::new(ctx.clone(), |handle: u32, host: String, port: u32| {
                    match u16::try_from(port) { Ok(port) if port != 0 => udp_connect(handle, &host, port), _ => "err|EINVAL|invalid UDP port".to_string() }
                }).expect("failed to create JavaScript UDP connector");
                let udp_disconnect_function = Function::new(ctx.clone(), |handle: u32| udp_disconnect(handle))
                    .expect("failed to create JavaScript UDP disconnector");
                let udp_close_function =
                    Function::new(ctx.clone(), |handle: u32| udp_close(handle))
                        .expect("failed to create JavaScript UDP closer");
                let tls_connect_function = Function::new(
                    ctx.clone(),
                    |host: String, port: u32, server_name: String, ca: String| {
                        tls_connect(TlsClientOptions {
                            host: &host,
                            port: port as u16,
                            server_name: &server_name,
                            ca_spec: &ca,
                            cert_spec: "",
                            key_spec: "",
                            alpn_spec: "",
                            report_alpn: false,
                            reject_unauthorized: true,
                        })
                    },
                )
                .expect("failed to create JavaScript TLS connector");
                let tls_connect_with_identity_function = Function::new(
                    ctx.clone(),
                    |host: String,
                     port: u32,
                     server_name: String,
                     ca: String,
                     cert: String,
                     key: String| {
                        tls_connect(TlsClientOptions {
                            host: &host,
                            port: port as u16,
                            server_name: &server_name,
                            ca_spec: &ca,
                            cert_spec: &cert,
                            key_spec: &key,
                            alpn_spec: "",
                            report_alpn: false,
                            reject_unauthorized: true,
                        })
                    },
                )
                .expect("failed to create JavaScript mutual TLS connector");
                let tls_connect_with_options_function = Function::new(
                    ctx.clone(),
                    |host: String,
                     port: u32,
                     server_name: String,
                     ca: String,
                     cert: String,
                     key: String,
                     options: String| {
                        let (verification, alpn) =
                            options.split_once('|').unwrap_or(("1", options.as_str()));
                        tls_connect(TlsClientOptions {
                            host: &host,
                            port: port as u16,
                            server_name: &server_name,
                            ca_spec: &ca,
                            cert_spec: &cert,
                            key_spec: &key,
                            alpn_spec: alpn,
                            report_alpn: true,
                            reject_unauthorized: verification != "0",
                        })
                    },
                )
                .expect("failed to create JavaScript TLS options connector");
                let tls_write_function =
                    Function::new(ctx.clone(), |handle: u32, value: String| {
                        tls_write(handle, &hex_decode(&value))
                    })
                    .expect("failed to create JavaScript TLS writer");
                let tls_finish_function =
                    Function::new(ctx.clone(), |handle: u32| tls_finish(handle))
                        .expect("failed to create JavaScript TLS finisher");
                let tls_shutdown_function =
                    Function::new(ctx.clone(), |handle: u32| tls_shutdown_write(handle))
                        .expect("failed to create JavaScript TLS shutdown function");
                let tls_poll_read_function =
                    Function::new(ctx.clone(), |handle: u32| tls_poll_read(handle))
                        .expect("failed to create JavaScript TLS polling reader");
                let tls_destroy_function =
                    Function::new(ctx.clone(), |handle: u32| tls_destroy(handle))
                        .expect("failed to create JavaScript TLS closer");
                let tls_alpn_function = Function::new(ctx.clone(), |handle: u32| tls_alpn(handle))
                    .expect("failed to create JavaScript TLS ALPN reader");
                let tls_peer_certificate_function =
                    Function::new(ctx.clone(), |handle: u32| tls_certificate(handle, true))
                        .expect("failed to create JavaScript TLS peer certificate reader");
                let tls_local_certificate_function =
                    Function::new(ctx.clone(), |handle: u32| tls_certificate(handle, false))
                        .expect("failed to create JavaScript TLS local certificate reader");
                let tls_peer_certificate_metadata_function =
                    Function::new(ctx.clone(), |handle: u32| {
                        tls_certificate_metadata(handle, true)
                    })
                    .expect("failed to create JavaScript TLS peer certificate metadata reader");
                let tls_local_certificate_metadata_function =
                    Function::new(ctx.clone(), |handle: u32| {
                        tls_certificate_metadata(handle, false)
                    })
                    .expect("failed to create JavaScript TLS local certificate metadata reader");
                let tls_server_listen_function = Function::new(
                    ctx.clone(),
                    |host: String, port: u32, cert: String, key: String| {
                        tls_server_listen(TlsServerOptions {
                            host: &host,
                            port: port as u16,
                            cert_spec: &cert,
                            key_spec: &key,
                            ca_spec: "",
                            request_cert: false,
                            reject_unauthorized: true,
                            alpn_spec: "",
                        })
                    },
                )
                .expect("failed to create JavaScript TLS listener");
                let tls_server_listen_with_ca_function = Function::new(
                    ctx.clone(),
                    |host: String,
                     port: u32,
                     cert: String,
                     key: String,
                     ca: String,
                     reject_unauthorized: bool| {
                        tls_server_listen(TlsServerOptions {
                            host: &host,
                            port: port as u16,
                            cert_spec: &cert,
                            key_spec: &key,
                            ca_spec: &ca,
                            request_cert: true,
                            reject_unauthorized,
                            alpn_spec: "",
                        })
                    },
                )
                .expect("failed to create JavaScript mutual TLS listener");
                let tls_server_listen_with_options_function = Function::new(
                    ctx.clone(),
                    |host: String,
                     port: u32,
                     cert: String,
                     key: String,
                     ca: String,
                     flags: u32,
                     alpn: String| {
                        tls_server_listen(TlsServerOptions {
                            host: &host,
                            port: port as u16,
                            cert_spec: &cert,
                            key_spec: &key,
                            ca_spec: &ca,
                            request_cert: flags & 1 != 0,
                            reject_unauthorized: flags & 2 != 0,
                            alpn_spec: &alpn,
                        })
                    },
                )
                .expect("failed to create JavaScript TLS options listener");
                let tls_server_accept_function =
                    Function::new(ctx.clone(), |handle: u32| tls_server_accept(handle))
                        .expect("failed to create JavaScript TLS acceptor");
                let tls_server_poll_accept_function =
                    Function::new(ctx.clone(), |handle: u32| tls_server_poll_accept(handle))
                        .expect("failed to create JavaScript TLS polling acceptor");
                let tls_server_read_function =
                    Function::new(ctx.clone(), |handle: u32| tls_server_read(handle))
                        .expect("failed to create JavaScript TLS reader");
                let tls_server_close_function =
                    Function::new(ctx.clone(), |handle: u32| tls_server_close_listener(handle))
                        .expect("failed to create JavaScript TLS listener closer");
                let os_info_function = Function::new(ctx.clone(), os_info_json)
                    .expect("failed to create JavaScript OS information source");
                let os_identity_function = Function::new(ctx.clone(), os_identity_json)
                    .expect("failed to create JavaScript OS identity source");
                let os_get_priority_function = Function::new(ctx.clone(), os_get_priority_json)
                    .expect("failed to create JavaScript OS priority getter");
                let os_set_priority_function = Function::new(ctx.clone(), os_set_priority_json)
                    .expect("failed to create JavaScript OS priority setter");
                let os_network_interfaces_function =
                    Function::new(ctx.clone(), network_interfaces_json)
                        .expect("failed to create JavaScript OS network interface source");
                let dns_lookup_function = Function::new(ctx.clone(), dns_lookup_json)
                    .expect("failed to create JavaScript DNS lookup source");
                // Backs `Intl.DateTimeFormat` (`platform_globals/intl.js`) --
                // the one native primitive needed to compute a real,
                // DST-aware IANA timezone offset/breakdown for a given
                // instant. See `quickjs/intl.rs`'s own doc comment.
                let intl_zoned_parts_function = Function::new(
                    ctx.clone(),
                    |tz_name: String, timestamp_ms: f64| {
                        intl_zoned_parts_json(&tz_name, timestamp_ms)
                    },
                )
                .expect("failed to create JavaScript Intl timezone source");
                #[cfg(feature = "intl")]
                let intl_locale_parse_function =
                    Function::new(ctx.clone(), |tag: String| intl_locale_parse_json(&tag))
                        .expect("failed to create JavaScript Intl locale parser");
                #[cfg(feature = "intl")]
                let intl_locale_resolve_function =
                    Function::new(ctx.clone(), |tag: String| intl_locale_resolve_json(&tag))
                        .expect("failed to create JavaScript Intl locale resolver");
                #[cfg(feature = "intl")]
                let intl_datetime_resolved_options_function = Function::new(
                    ctx.clone(),
                    |tag: String| intl_datetime_resolved_options_json(&tag),
                )
                .expect("failed to create JavaScript Intl DateTimeFormat option resolver");
                #[cfg(feature = "intl")]
                let intl_locale_maximize_function =
                    Function::new(ctx.clone(), |tag: String| intl_locale_maximize_json(&tag))
                        .expect("failed to create JavaScript Intl locale maximizer");
                #[cfg(feature = "intl")]
                let intl_locale_minimize_function =
                    Function::new(ctx.clone(), |tag: String| intl_locale_minimize_json(&tag))
                        .expect("failed to create JavaScript Intl locale minimizer");
                #[cfg(feature = "intl")]
                let intl_number_format_function = Function::new(
                    ctx.clone(),
                    |locale: String, digits: String, use_grouping: bool| {
                        intl_number_format(&locale, &digits, use_grouping)
                    },
                )
                .expect("failed to create JavaScript Intl number formatter");
                #[cfg(feature = "intl")]
                let intl_compact_number_function = Function::new(
                    ctx.clone(),
                    |locale: String, digits: String, long: bool| {
                        intl_compact_number(&locale, &digits, long)
                    },
                )
                .expect("failed to create JavaScript Intl compact number formatter");
                #[cfg(feature = "intl")]
                let intl_currency_fraction_digits_function =
                    Function::new(ctx.clone(), |currency: String| {
                        intl_currency_fraction_digits(&currency).map(i32::from)
                    })
                    .expect("failed to create JavaScript Intl currency fraction-digit resolver");
                #[cfg(feature = "intl")]
                let intl_percent_format_function = Function::new(
                    ctx.clone(),
                    |locale: String, digits: String, grouping: bool| {
                        intl_percent_format(&locale, &digits, grouping).unwrap_or_default()
                    },
                )
                .expect("failed to create JavaScript Intl percent formatter");
                #[cfg(feature = "intl")]
                let intl_percent_format_large_function = Function::new(
                    ctx.clone(),
                    |locale: String, representative: String, core: String, grouping: bool| {
                        intl_percent_format_large(&locale, &representative, &core, grouping).unwrap_or_default()
                    },
                )
                .expect("failed to create JavaScript Intl large percent formatter");
                #[cfg(feature = "intl")]
                let intl_currency_format_function = Function::new(
                    ctx.clone(),
                    |locale: String, digits: String, currency: String, display: String, grouping: bool| {
                        intl_currency_format(&locale, &digits, &currency, &display, grouping)
                            .unwrap_or_default()
                    },
                )
                .expect("failed to create JavaScript Intl currency formatter");
                #[cfg(feature = "intl")]
                let intl_currency_format_large_function = Function::new(
                    ctx.clone(),
                    |locale: String, representative: String, core: String, currency: String, display: String, grouping: bool| {
                        intl_currency_format_large(&locale, &representative, &core, &currency, &display, grouping)
                            .unwrap_or_default()
                    },
                )
                .expect("failed to create JavaScript Intl large currency formatter");
                #[cfg(feature = "intl")]
                let intl_unit_format_function = Function::new(
                    ctx.clone(),
                    |locale: String, digits: String, unit: String, width: String, grouping: bool| {
                        intl_unit_format(&locale, &digits, &unit, &width, grouping).unwrap_or_default()
                    },
                )
                .expect("failed to create JavaScript Intl unit formatter");
                #[cfg(feature = "intl")]
                let intl_unit_format_large_function = Function::new(
                    ctx.clone(),
                    |locale: String, representative: String, core: String, unit: String, width: String, grouping: bool| {
                        intl_unit_format_large(&locale, &representative, &core, &unit, &width, grouping)
                            .unwrap_or_default()
                    },
                )
                .expect("failed to create JavaScript Intl large unit formatter");
                #[cfg(feature = "intl")]
                let intl_time_zone_name_function = Function::new(
                    ctx.clone(),
                    |locale: String,
                     time_zone: String,
                     timestamp_ms: f64,
                     offset_minutes: i32,
                     generic: bool| {
                        intl_time_zone_name(
                            &locale,
                            &time_zone,
                            timestamp_ms,
                            offset_minutes,
                            generic,
                        )
                        .unwrap_or_default()
                    },
                )
                .expect("failed to create JavaScript Intl timezone-name formatter");
                #[cfg(feature = "intl")]
                let intl_relative_time_format_function = Function::new(
                    ctx.clone(),
                    |locale: String, unit: String, style: String, numeric: String, value: f64| {
                        intl_relative_time_format(&locale, &unit, &style, &numeric, value)
                    },
                )
                .expect("failed to create JavaScript Intl relative-time formatter");
                #[cfg(feature = "intl")]
                let intl_segment_function = Function::new(
                    ctx.clone(),
                    |locale: String, granularity: String, text: crate::Wtf8String| {
                        intl_segment(&locale, &granularity, &text.0)
                    },
                )
                .expect("failed to create JavaScript Intl segmenter");
                #[cfg(feature = "intl")]
                let intl_collator_compare_function = Function::new(
                    ctx.clone(),
                    |locale: String,
                     sensitivity: String,
                     ignore_punctuation: bool,
                     numeric: bool,
                     case_first: String,
                     a: crate::Wtf8String,
                     b: crate::Wtf8String| {
                        intl_collator_compare(&locale, &sensitivity, ignore_punctuation, numeric, &case_first, &a.0, &b.0)
                    },
                )
                .expect("failed to create JavaScript Intl collator");
                #[cfg(feature = "icu4c")]
                let intl_collator_compare_search_function = Function::new(
                    ctx.clone(),
                    |locale: String,
                     sensitivity: String,
                     ignore_punctuation: bool,
                     numeric: bool,
                     case_first: String,
                     a: crate::Wtf8String,
                     b: crate::Wtf8String| {
                        intl_collator_compare_search(
                            &locale,
                            &sensitivity,
                            ignore_punctuation,
                            numeric,
                            &case_first,
                            &a.0,
                            &b.0,
                        )
                    },
                )
                .expect("failed to create JavaScript Intl search collator");
                #[cfg(feature = "icu4c")]
                let intl_plural_range_icu4c_function = Function::new(
                    ctx.clone(),
                    |locale: String, kind: String, start: String, end: String| {
                        intl_plural_range_icu4c(&locale, &kind, &start, &end)
                    },
                )
                .expect("failed to create JavaScript Intl ICU4C plural-range resolver");
                #[cfg(feature = "icu4c")]
                let intl_datetime_narrow_icu4c_function = Function::new(
                    ctx.clone(),
                    |locale: String, options_json: String, zoned_json: String| {
                        intl_datetime_narrow_icu4c(&locale, &options_json, &zoned_json)
                    },
                )
                .expect("failed to create JavaScript Intl ICU4C narrow date formatter");
                #[cfg(feature = "icu4c")]
                let intl_datetime_range_icu4c_function = Function::new(
                    ctx.clone(),
                    |locale: String,
                     options_json: String,
                     start_json: String,
                     end_json: String| {
                        intl_datetime_range_icu4c(&locale, &options_json, &start_json, &end_json)
                    },
                )
                .expect("failed to create JavaScript Intl ICU4C date-range formatter");
                #[cfg(feature = "icu4c")]
                let intl_number_range_icu4c_function = Function::new(
                    ctx.clone(),
                    |locale: String, skeleton: String, start: String, end: String| {
                        intl_number_range_icu4c(&locale, &skeleton, &start, &end)
                    },
                )
                .expect("failed to create JavaScript Intl ICU4C number-range formatter");
                #[cfg(feature = "icu4c")]
                let intl_datetime_range_parts_icu4c_function = Function::new(
                    ctx.clone(),
                    |locale: String,
                     options_json: String,
                     start_json: String,
                     end_json: String| {
                        intl_datetime_range_parts_icu4c(
                            &locale,
                            &options_json,
                            &start_json,
                            &end_json,
                        )
                    },
                )
                .expect("failed to create JavaScript Intl ICU4C date-range-parts formatter");
                #[cfg(feature = "icu4c")]
                let intl_number_range_parts_icu4c_function = Function::new(
                    ctx.clone(),
                    |locale: String, skeleton: String, start: String, end: String| {
                        intl_number_range_parts_icu4c(&locale, &skeleton, &start, &end)
                    },
                )
                .expect("failed to create JavaScript Intl ICU4C number-range-parts formatter");
                #[cfg(feature = "intl")]
                let intl_plural_category_function = Function::new(
                    ctx.clone(),
                    |locale: String, kind: String, digits: String| {
                        intl_plural_category(&locale, &kind, &digits)
                    },
                )
                .expect("failed to create JavaScript Intl plural-category resolver");
                #[cfg(feature = "intl")]
                let intl_plural_categories_function = Function::new(
                    ctx.clone(),
                    |locale: String, kind: String| intl_plural_categories(&locale, &kind),
                )
                .expect("failed to create JavaScript Intl plural-categories resolver");
                #[cfg(feature = "intl")]
                let intl_list_format_function = Function::new(
                    ctx.clone(),
                    |locale: String, kind: String, style: String, items_json: String| {
                        intl_list_format(&locale, &kind, &style, &items_json)
                    },
                )
                .expect("failed to create JavaScript Intl list formatter");
                #[cfg(feature = "intl")]
                let intl_datetime_format_parts_function = Function::new(
                    ctx.clone(),
                    |locale: String, options_json: String, zoned_parts_json: String| {
                        intl_datetime_format_parts_json(&locale, &options_json, &zoned_parts_json)
                    },
                )
                .expect("failed to create JavaScript Intl datetime formatter");
                #[cfg(feature = "intl")]
                let intl_datetime_skeleton_parts_function = Function::new(
                    ctx.clone(),
                    |locale: String, options_json: String, zoned_parts_json: String| {
                        intl_datetime_skeleton_parts_native(&locale, &options_json, &zoned_parts_json)
                    },
                )
                .expect("failed to create JavaScript Intl datetime skeleton formatter");
                ctx.globals()
                    .set("__thaw_crypto_random_hex", random_hex)
                    .expect("failed to install JavaScript random source");
                ctx.globals()
                    .set("__thaw_crypto_hash_hex", hash_hex)
                    .expect("failed to install JavaScript hash function");
                ctx.globals()
                    .set("__thaw_crypto_hmac_hex", hmac_hex)
                    .expect("failed to install JavaScript HMAC function");
                ctx.globals()
                    .set("__thaw_crypto_pbkdf2_hex", pbkdf2_hex)
                    .expect("failed to install JavaScript PBKDF2 function");
                ctx.globals()
                    .set("__thaw_crypto_scrypt_hex", scrypt_hex)
                    .expect("failed to install JavaScript scrypt function");
                ctx.globals()
                    .set("__thaw_crypto_cipher_hex", cipher_hex)
                    .expect("failed to install JavaScript cipher function");
                ctx.globals()
                    .set("__thaw_crypto_import_key_json", crypto_import_key)
                    .expect("failed to install JavaScript key-import function");
                ctx.globals()
                    .set("__thaw_crypto_asymmetric_sign_hex", crypto_asymmetric_sign)
                    .expect("failed to install JavaScript asymmetric sign function");
                ctx.globals()
                    .set(
                        "__thaw_crypto_asymmetric_sign_pss_hex",
                        crypto_asymmetric_sign_pss,
                    )
                    .expect("failed to install JavaScript asymmetric PSS sign function");
                ctx.globals()
                    .set(
                        "__thaw_crypto_asymmetric_verify",
                        crypto_asymmetric_verify,
                    )
                    .expect("failed to install JavaScript asymmetric verify function");
                ctx.globals()
                    .set(
                        "__thaw_crypto_asymmetric_verify_pss",
                        crypto_asymmetric_verify_pss,
                    )
                    .expect("failed to install JavaScript asymmetric PSS verify function");
                ctx.globals()
                    .set(
                        "__thaw_crypto_asymmetric_encrypt_hex",
                        crypto_asymmetric_encrypt,
                    )
                    .expect("failed to install JavaScript asymmetric encrypt function");
                ctx.globals()
                    .set(
                        "__thaw_crypto_asymmetric_decrypt_hex",
                        crypto_asymmetric_decrypt,
                    )
                    .expect("failed to install JavaScript asymmetric decrypt function");
                ctx.globals()
                    .set(
                        "__thaw_crypto_asymmetric_private_encrypt_hex",
                        crypto_asymmetric_private_encrypt,
                    )
                    .expect("failed to install JavaScript asymmetric privateEncrypt function");
                ctx.globals()
                    .set(
                        "__thaw_crypto_asymmetric_public_decrypt_hex",
                        crypto_asymmetric_public_decrypt,
                    )
                    .expect("failed to install JavaScript asymmetric publicDecrypt function");
                ctx.globals()
                    .set(
                        "__thaw_crypto_generate_key_pair_json",
                        crypto_generate_key_pair,
                    )
                    .expect("failed to install JavaScript key-pair generation function");
                ctx.globals()
                    .set(
                        "__thaw_crypto_diffie_hellman_hex",
                        crypto_diffie_hellman,
                    )
                    .expect("failed to install JavaScript Diffie-Hellman function");
                ctx.globals()
                    .set(
                        "__thaw_crypto_ecdh_generate_keys_hex",
                        crypto_ecdh_generate_keys,
                    )
                    .expect("failed to install JavaScript ECDH key-generation function");
                ctx.globals()
                    .set(
                        "__thaw_crypto_ecdh_public_from_private_hex",
                        crypto_ecdh_public_from_private,
                    )
                    .expect("failed to install JavaScript ECDH public-key-derivation function");
                ctx.globals()
                    .set(
                        "__thaw_crypto_ecdh_compute_secret_hex",
                        crypto_ecdh_compute_secret,
                    )
                    .expect("failed to install JavaScript ECDH computeSecret function");
                ctx.globals()
                    .set("__thaw_intl_zoned_parts", intl_zoned_parts_function)
                    .expect("failed to install JavaScript Intl timezone source");
                #[cfg(feature = "intl")]
                ctx.globals()
                    .set("__thaw_intl_locale_parse", intl_locale_parse_function)
                    .expect("failed to install JavaScript Intl locale parser");
                #[cfg(feature = "intl")]
                ctx.globals()
                    .set("__thaw_intl_locale_resolve", intl_locale_resolve_function)
                    .expect("failed to install JavaScript Intl locale resolver");
                #[cfg(feature = "intl")]
                ctx.globals()
                    .set("__thaw_intl_datetime_resolved_options", intl_datetime_resolved_options_function)
                    .expect("failed to install JavaScript Intl DateTimeFormat option resolver");
                #[cfg(feature = "intl")]
                ctx.globals()
                    .set("__thaw_intl_locale_maximize", intl_locale_maximize_function)
                    .expect("failed to install JavaScript Intl locale maximizer");
                #[cfg(feature = "intl")]
                ctx.globals()
                    .set("__thaw_intl_locale_minimize", intl_locale_minimize_function)
                    .expect("failed to install JavaScript Intl locale minimizer");
                #[cfg(feature = "intl")]
                ctx.globals()
                    .set("__thaw_intl_number_format", intl_number_format_function)
                    .expect("failed to install JavaScript Intl number formatter");
                #[cfg(feature = "intl")]
                ctx.globals()
                    .set(
                        "__thaw_intl_compact_number",
                        intl_compact_number_function,
                    )
                    .expect("failed to install JavaScript Intl compact number formatter");
                #[cfg(feature = "intl")]
                ctx.globals()
                    .set(
                        "__thaw_intl_currency_fraction_digits",
                        intl_currency_fraction_digits_function,
                    )
                    .expect("failed to install JavaScript Intl currency fraction-digit resolver");
                #[cfg(feature = "intl")]
                ctx.globals()
                    .set("__thaw_intl_percent_format", intl_percent_format_function)
                    .expect("failed to install JavaScript Intl percent formatter");
                #[cfg(feature = "intl")]
                ctx.globals()
                    .set("__thaw_intl_percent_format_large", intl_percent_format_large_function)
                    .expect("failed to install JavaScript Intl large percent formatter");
                #[cfg(feature = "intl")]
                ctx.globals()
                    .set("__thaw_intl_currency_format", intl_currency_format_function)
                    .expect("failed to install JavaScript Intl currency formatter");
                #[cfg(feature = "intl")]
                ctx.globals()
                    .set("__thaw_intl_currency_format_large", intl_currency_format_large_function)
                    .expect("failed to install JavaScript Intl large currency formatter");
                #[cfg(feature = "intl")]
                ctx.globals()
                    .set("__thaw_intl_unit_format", intl_unit_format_function)
                    .expect("failed to install JavaScript Intl unit formatter");
                #[cfg(feature = "intl")]
                ctx.globals()
                    .set("__thaw_intl_unit_format_large", intl_unit_format_large_function)
                    .expect("failed to install JavaScript Intl large unit formatter");
                #[cfg(feature = "intl")]
                ctx.globals()
                    .set("__thaw_intl_time_zone_name", intl_time_zone_name_function)
                    .expect("failed to install JavaScript Intl timezone-name formatter");
                #[cfg(feature = "intl")]
                ctx.globals()
                    .set(
                        "__thaw_intl_relative_time_format",
                        intl_relative_time_format_function,
                    )
                    .expect("failed to install JavaScript Intl relative-time formatter");
                #[cfg(feature = "intl")]
                ctx.globals()
                    .set("__thaw_intl_segment", intl_segment_function)
                    .expect("failed to install JavaScript Intl segmenter");
                #[cfg(feature = "intl")]
                ctx.globals()
                    .set("__thaw_intl_collator_compare", intl_collator_compare_function)
                    .expect("failed to install JavaScript Intl collator");
                #[cfg(feature = "icu4c")]
                ctx.globals()
                    .set(
                        "__thaw_intl_collator_compare_search",
                        intl_collator_compare_search_function,
                    )
                    .expect("failed to install JavaScript Intl search collator");
                #[cfg(feature = "icu4c")]
                ctx.globals()
                    .set(
                        "__thaw_intl_plural_range_icu4c",
                        intl_plural_range_icu4c_function,
                    )
                    .expect("failed to install JavaScript Intl ICU4C plural-range resolver");
                #[cfg(feature = "icu4c")]
                ctx.globals()
                    .set(
                        "__thaw_intl_datetime_narrow_icu4c",
                        intl_datetime_narrow_icu4c_function,
                    )
                    .expect("failed to install JavaScript Intl ICU4C narrow date formatter");
                #[cfg(feature = "icu4c")]
                ctx.globals()
                    .set(
                        "__thaw_intl_datetime_range_icu4c",
                        intl_datetime_range_icu4c_function,
                    )
                    .expect("failed to install JavaScript Intl ICU4C date-range formatter");
                #[cfg(feature = "icu4c")]
                ctx.globals()
                    .set(
                        "__thaw_intl_number_range_icu4c",
                        intl_number_range_icu4c_function,
                    )
                    .expect("failed to install JavaScript Intl ICU4C number-range formatter");
                #[cfg(feature = "icu4c")]
                ctx.globals()
                    .set(
                        "__thaw_intl_datetime_range_parts_icu4c",
                        intl_datetime_range_parts_icu4c_function,
                    )
                    .expect("failed to install JavaScript Intl ICU4C date-range-parts formatter");
                #[cfg(feature = "icu4c")]
                ctx.globals()
                    .set(
                        "__thaw_intl_number_range_parts_icu4c",
                        intl_number_range_parts_icu4c_function,
                    )
                    .expect("failed to install JavaScript Intl ICU4C number-range-parts formatter");
                #[cfg(feature = "intl")]
                ctx.globals()
                    .set("__thaw_intl_plural_category", intl_plural_category_function)
                    .expect("failed to install JavaScript Intl plural-category resolver");
                #[cfg(feature = "intl")]
                ctx.globals()
                    .set(
                        "__thaw_intl_plural_categories",
                        intl_plural_categories_function,
                    )
                    .expect("failed to install JavaScript Intl plural-categories resolver");
                #[cfg(feature = "intl")]
                ctx.globals()
                    .set("__thaw_intl_list_format", intl_list_format_function)
                    .expect("failed to install JavaScript Intl list formatter");
                #[cfg(feature = "intl")]
                ctx.globals()
                    .set(
                        "__thaw_intl_datetime_format_parts",
                        intl_datetime_format_parts_function,
                    )
                    .expect("failed to install JavaScript Intl datetime formatter");
                #[cfg(feature = "intl")]
                ctx.globals()
                    .set(
                        "__thaw_intl_datetime_skeleton_parts",
                        intl_datetime_skeleton_parts_function,
                    )
                    .expect("failed to install JavaScript Intl datetime skeleton formatter");
                ctx.globals()
                    .set("__thaw_hpack_huffman_encode", hpack_huffman_encode)
                    .expect("failed to install HPACK Huffman encoder");
                ctx.globals()
                    .set("__thaw_hpack_huffman_decode", hpack_huffman_decode)
                    .expect("failed to install HPACK Huffman decoder");
                ctx.globals()
                    .set("__thaw_zlib_hex", zlib_hex)
                    .expect("failed to install JavaScript compression function");
                ctx.globals()
                    .set("__thaw_zlib_stream_create", create_web_zlib_stream)
                    .expect("failed to install streaming zlib allocator");
                ctx.globals()
                    .set("__thaw_zlib_stream_write", write_web_zlib_stream)
                    .expect("failed to install streaming zlib writer");
                ctx.globals()
                    .set("__thaw_zlib_stream_drop", drop_web_zlib_stream)
                    .expect("failed to install streaming zlib closer");
                ctx.globals()
                    .set("__thaw_fs", fs_function)
                    .expect("failed to install JavaScript filesystem function");
                ctx.globals()
                    .set("__thaw_net_connect", tcp_connect)
                    .expect("failed to install JavaScript TCP connector");
                ctx.globals()
                    .set("__thaw_net_write", tcp_write)
                    .expect("failed to install JavaScript TCP writer");
                ctx.globals()
                    .set("__thaw_net_finish", tcp_finish)
                    .expect("failed to install JavaScript TCP finisher");
                ctx.globals()
                    .set("__thaw_net_shutdown", tcp_shutdown)
                    .expect("failed to install JavaScript TCP shutdown function");
                ctx.globals()
                    .set("__thaw_net_poll_read", tcp_poll_read)
                    .expect("failed to install JavaScript TCP polling reader");
                ctx.globals()
                    .set("__thaw_net_destroy", tcp_destroy)
                    .expect("failed to install JavaScript TCP closer");
                ctx.globals()
                    .set("__thaw_net_listen", tcp_listen)
                    .expect("failed to install JavaScript TCP listener");
                ctx.globals()
                    .set("__thaw_net_accept", tcp_accept)
                    .expect("failed to install JavaScript TCP acceptor");
                ctx.globals()
                    .set("__thaw_net_poll_accept", tcp_poll_accept)
                    .expect("failed to install JavaScript TCP polling acceptor");
                ctx.globals()
                    .set("__thaw_net_read", tcp_read)
                    .expect("failed to install JavaScript TCP reader");
                ctx.globals()
                    .set("__thaw_net_close_listener", tcp_close_listener)
                    .expect("failed to install JavaScript TCP listener closer");
                ctx.globals()
                    .set("__thaw_udp_bind", udp_bind_function)
                    .expect("failed to install JavaScript UDP binder");
                ctx.globals()
                    .set("__thaw_udp_send", udp_send_function)
                    .expect("failed to install JavaScript UDP sender");
                ctx.globals()
                    .set("__thaw_udp_receive", udp_receive_function)
                    .expect("failed to install JavaScript UDP receiver");
                ctx.globals()
                    .set("__thaw_udp_connect", udp_connect_function)
                    .expect("failed to install JavaScript UDP connector");
                ctx.globals()
                    .set("__thaw_udp_disconnect", udp_disconnect_function)
                    .expect("failed to install JavaScript UDP disconnector");
                ctx.globals()
                    .set("__thaw_udp_close", udp_close_function)
                    .expect("failed to install JavaScript UDP closer");
                ctx.globals()
                    .set("__thaw_tls_connect", tls_connect_function)
                    .expect("failed to install JavaScript TLS connector");
                ctx.globals()
                    .set(
                        "__thaw_tls_connect_with_identity",
                        tls_connect_with_identity_function,
                    )
                    .expect("failed to install JavaScript mutual TLS connector");
                ctx.globals()
                    .set(
                        "__thaw_tls_connect_with_options",
                        tls_connect_with_options_function,
                    )
                    .expect("failed to install JavaScript TLS options connector");
                ctx.globals()
                    .set("__thaw_tls_write", tls_write_function)
                    .expect("failed to install JavaScript TLS writer");
                ctx.globals()
                    .set("__thaw_tls_finish", tls_finish_function)
                    .expect("failed to install JavaScript TLS finisher");
                ctx.globals()
                    .set("__thaw_tls_shutdown", tls_shutdown_function)
                    .expect("failed to install JavaScript TLS shutdown function");
                ctx.globals()
                    .set("__thaw_tls_poll_read", tls_poll_read_function)
                    .expect("failed to install JavaScript TLS polling reader");
                ctx.globals()
                    .set("__thaw_tls_destroy", tls_destroy_function)
                    .expect("failed to install JavaScript TLS closer");
                ctx.globals()
                    .set("__thaw_tls_alpn", tls_alpn_function)
                    .expect("failed to install JavaScript TLS ALPN reader");
                ctx.globals()
                    .set("__thaw_tls_peer_certificate", tls_peer_certificate_function)
                    .expect("failed to install JavaScript TLS peer certificate reader");
                ctx.globals()
                    .set(
                        "__thaw_tls_local_certificate",
                        tls_local_certificate_function,
                    )
                    .expect("failed to install JavaScript TLS local certificate reader");
                ctx.globals()
                    .set(
                        "__thaw_tls_peer_certificate_metadata",
                        tls_peer_certificate_metadata_function,
                    )
                    .expect("failed to install JavaScript TLS peer certificate metadata reader");
                ctx.globals()
                    .set(
                        "__thaw_tls_local_certificate_metadata",
                        tls_local_certificate_metadata_function,
                    )
                    .expect("failed to install JavaScript TLS local certificate metadata reader");
                ctx.globals()
                    .set("__thaw_tls_server_listen", tls_server_listen_function)
                    .expect("failed to install JavaScript TLS listener");
                ctx.globals()
                    .set(
                        "__thaw_tls_server_listen_with_ca",
                        tls_server_listen_with_ca_function,
                    )
                    .expect("failed to install JavaScript mutual TLS listener");
                ctx.globals()
                    .set(
                        "__thaw_tls_server_listen_with_options",
                        tls_server_listen_with_options_function,
                    )
                    .expect("failed to install JavaScript TLS options listener");
                ctx.globals()
                    .set("__thaw_tls_server_accept", tls_server_accept_function)
                    .expect("failed to install JavaScript TLS acceptor");
                ctx.globals()
                    .set(
                        "__thaw_tls_server_poll_accept",
                        tls_server_poll_accept_function,
                    )
                    .expect("failed to install JavaScript TLS polling acceptor");
                ctx.globals()
                    .set("__thaw_tls_server_read", tls_server_read_function)
                    .expect("failed to install JavaScript TLS reader");
                ctx.globals()
                    .set("__thaw_tls_server_close", tls_server_close_function)
                    .expect("failed to install JavaScript TLS listener closer");
                ctx.globals()
                    .set("__thaw_os_info", os_info_function)
                    .expect("failed to install JavaScript OS information source");
                ctx.globals()
                    .set("__thaw_os_identity", os_identity_function)
                    .expect("failed to install JavaScript OS identity source");
                ctx.globals()
                    .set("__thaw_os_get_priority", os_get_priority_function)
                    .expect("failed to install JavaScript OS priority getter");
                ctx.globals()
                    .set("__thaw_os_set_priority", os_set_priority_function)
                    .expect("failed to install JavaScript OS priority setter");
                ctx.globals()
                    .set(
                        "__thaw_os_network_interfaces",
                        os_network_interfaces_function,
                    )
                    .expect("failed to install JavaScript OS network interface source");
                ctx.globals()
                    .set("__thaw_dns_lookup", dns_lookup_function)
                    .expect("failed to install JavaScript DNS lookup source");
                install_graph_handle_functions(&ctx)
                    .expect("failed to install graph handle functions");
                install_async_context_bootstrap(&ctx)
                    .expect("failed to install async context bootstrap");
                let platform_result = ctx.eval::<(), _>(PLATFORM_GLOBALS);
                ctx.globals().remove("__thaw_async_context_bootstrap")
                    .expect("failed to remove async context bootstrap");
                platform_result.expect("failed to install JavaScript platform globals");
            });
            (runtime, context)
        });
    });
}

fn with_context<R>(f: impl FnOnce(Ctx<'_>) -> R) -> R {
    ensure_context();
    JS.with(|cell| {
        cell.borrow()
            .as_ref()
            .expect("QuickJS context was initialized")
            .1
            .with(f)
    })
}

/// Like [`with_context`], but reuses the currently-active `Ctx` (see
/// `ActiveNapiContext`) instead of calling `with_context` again when one
/// is already active -- needed for a native callback invoked *by* a
/// dynamic call (real example: zod's `.superRefine((val, ctx) => { ctx.
/// addIssue(...); })`) that itself makes a further dynamic call from
/// inside its own body: the *outer* dynamic call is still on the stack
/// at that point (its own `with_context`'s `RefCell` borrow, and
/// `Context::with`'s own internal runtime lock, both still held), so a
/// second top-level `with_context` call would panic ("RefCell already
/// borrowed") rather than deadlock or corrupt anything -- confirmed via
/// a real repro. Bypassing `with_context`/`Context::with` entirely on
/// the reentrant path (reusing the already-active `Ctx` directly, not
/// re-locking anything) avoids both.
fn with_active_or_context<R>(f: impl for<'js> FnOnce(Ctx<'js>) -> R) -> R {
    ACTIVE_NAPI_CONTEXT.with(|active| {
        let active = active.get();
        if active.is_null() {
            with_context(f)
        } else {
            // SAFETY: `active` was set by `ActiveNapiContext::enter`,
            // called with a `Ctx` still alive on the stack of the outer
            // call currently reentering into us -- it hasn't been
            // dropped, only reborrowed here for the duration of `f`.
            let ctx = unsafe { (*(active as *const Ctx<'static>)).clone() };
            f(ctx)
        }
    })
}
