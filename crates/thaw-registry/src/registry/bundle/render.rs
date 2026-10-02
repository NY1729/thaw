const STAR_ORIGIN_RUNTIME: &str = r#"function __thaw_bundle_origin_for(ownerKey) {
  var own = Object.prototype.hasOwnProperty;
  var ambiguous = {};
  function factoryOf(key) {
    if (own.call(__thaw_bundle_export_graphs, key)) return key;
    var query = key.indexOf('?'), fragment = key.indexOf('#', 1);
    var at = query < 0 ? fragment : (fragment < 0 ? query : Math.min(query, fragment));
    return at < 0 ? key : key.slice(0, at);
  }
  function graphOf(key) { return __thaw_bundle_export_graphs[factoryOf(key)]; }
  function targetOf(key, spec) {
    return __thaw_bundle_target(__thaw_bundle_import_maps[factoryOf(key)] || {}, spec);
  }
  var enumerable = Object.prototype.propertyIsEnumerable;
  function valueOf(target, sourceKey, spec) {
    var edges = __thaw_bundle_edges[sourceKey];
    if (edges && own.call(edges, spec)) return edges[spec];
    // A cycle may reach this edge before its owner executes the loader.
    // Export discovery must never evaluate a dependency ahead of that owner.
    return undefined;
  }
  function namesOf(key, visited, sourceKey, spec) {
    if (visited.indexOf(key) >= 0) return [];
    var graph = graphOf(key);
    if (!graph) {
      var value = valueOf({ key: key, factory: factoryOf(key) }, sourceKey, spec);
      return value == null ? [] : Object.keys(value).filter(function(name) { return name !== '__esModule'; });
    }
    var next = visited.concat(key);
    var names = Object.keys(graph.local).concat(Object.keys(graph.indirect));
    for (var index = 0; index < graph.stars.length; index++) {
      var target = targetOf(key, graph.stars[index]);
      if (!target) continue;
      var more = namesOf(target.key, next, key, graph.stars[index]);
      for (var j = 0; j < more.length; j++) {
        if (more[j] !== 'default' && names.indexOf(more[j]) < 0) names.push(more[j]);
      }
    }
    return names;
  }
  function resolve(key, name, visited, allowMissing, sourceKey, spec) {
    var visit = key + '\u0000' + name;
    if (visited.indexOf(visit) >= 0) return null;
    var graph = graphOf(key);
    if (!graph) {
      // Star exports skip the CommonJS interop marker; explicit named
      // reexports still resolve it through allowMissing.
      if (name === '__esModule' && !allowMissing) return null;
      var value = valueOf({ key: key, factory: factoryOf(key) }, sourceKey, spec);
      return (allowMissing || (value != null && enumerable.call(value, name))) ? { kind: 'cjs', key: key, sourceKey: sourceKey, spec: spec, name: name } : null;
    }
    var next = visited.concat(visit);
    if (own.call(graph.local, name)) {
      return { kind: 'esm', key: key, binding: graph.local[name], name: name };
    }
    if (own.call(graph.indirect, name)) {
      var indirect = graph.indirect[name];
      var source = targetOf(key, indirect[0]);
      if (!source) return null;
      if (indirect[1] === null) {
        return { kind: 'namespace', key: source.key, binding: '*namespace*' };
      }
      if (indirect[2] && !graphOf(source.key)) {
        return { kind: 'cjs-default', key: source.key, sourceKey: key, spec: indirect[0] };
      }
      return resolve(source.key, indirect[1], next, true, key, indirect[0]);
    }
    if (name === 'default') return null;
    var found = null;
    for (var index = 0; index < graph.stars.length; index++) {
      var star = targetOf(key, graph.stars[index]);
      if (!star) continue;
      var candidate = resolve(star.key, name, next, false, key, graph.stars[index]);
      if (candidate === ambiguous) return ambiguous;
      if (!candidate) continue;
      if (found && !sameBinding(found, candidate)) return ambiguous;
      found = candidate;
    }
    return found;
  }
  function sameBinding(left, right) {
    if (left.kind !== right.kind) return false;
    if (left.kind === 'cjs') return left.key === right.key && left.name === right.name;
    if (left.kind === 'cjs-default') return left.key === right.key;
    return left.key === right.key && left.binding === right.binding;
  }
  function read(resolution) {
    if (resolution.kind === 'cjs' || resolution.kind === 'cjs-default') {
      var edges = __thaw_bundle_edges[resolution.sourceKey];
      if (!edges || !own.call(edges, resolution.spec)) throw new ReferenceError("Cannot access '" + (resolution.name || 'default') + "' before initialization");
      var value = valueOf(null, resolution.sourceKey, resolution.spec);
      return resolution.kind === 'cjs' ? value[resolution.name] : value && value.__esModule ? value.default : value;
    }
    var source = __thaw_bundle_require(resolution.key, factoryOf(resolution.key));
    return resolution.kind === 'namespace' ? source : source[resolution.name];
  }
  function linksReady(key, visited) {
    if (visited.indexOf(key) >= 0) return true;
    var graph = graphOf(key);
    if (!graph) return true;
    var next = visited.concat(key), edges = __thaw_bundle_edges[key];
    // Explicit named reexports cannot introduce a competing star binding.
    // Their names are static even if their opaque target has not run yet.
    var specs = graph.stars;
    for (var index = 0; index < specs.length; index++) {
      var spec = specs[index], target = targetOf(key, spec);
      if (!target) continue;
      // ESM names and terminal bindings are known from the source graph
      // before evaluation. Only opaque CJS/native targets need a loaded edge
      // to discover their exported names.
      if ((!graphOf(target.key) && (!edges || !own.call(edges, spec))) || !linksReady(target.key, next)) return false;
    }
    var indirectNames = Object.keys(graph.indirect);
    for (var n = 0; n < indirectNames.length; n++) {
      var indirect = graph.indirect[indirectNames[n]], target = targetOf(key, indirect[0]);
      // A direct opaque named reexport has a statically fixed name/binding.
      // An ESM target can itself resolve through unknown opaque stars.
      if (target && graphOf(target.key) && !linksReady(target.key, next)) return false;
    }
    return true;
  }
  return {
    edge: function(spec, value) {
      var edges = __thaw_bundle_edges[ownerKey];
      if (!edges) edges = __thaw_bundle_edges[ownerKey] = Object.create(null);
      if (own.call(edges, spec) && edges[spec] === value) return;
      edges[spec] = value;
      __thaw_bundle_edge_version++;
      // A cycle can reach an ESM module before its later CJS/native loader.
      // Only link when its full transitive export graph is available.
      for (var key in __thaw_bundle_star_linkers) __thaw_bundle_star_linkers[key]();
    },
    track: function(exports) {
      var linked = false, linking = false;
      var link = function() {
        if (linked || linking || !linksReady(ownerKey, [])) return;
        linking = true;
        try {
          // A CJS Proxy ownKeys trap can record another edge while names are
          // discovered. Recompute before publication if the graph changed.
          var bindings, before;
          do {
            before = __thaw_bundle_edge_version;
            bindings = [];
            var graph = graphOf(ownerKey), names = [];
            for (var index = 0; index < graph.stars.length; index++) {
              var spec = graph.stars[index], target = targetOf(ownerKey, spec);
              if (!target) continue;
              var more = namesOf(target.key, [], ownerKey, spec);
              for (var j = 0; j < more.length; j++) {
                var name = more[j];
                if (name !== 'default' && names.indexOf(name) < 0) names.push(name);
              }
            }
            for (var n = 0; n < names.length; n++) {
              var name = names[n];
              if (own.call(graph.local, name) || own.call(graph.indirect, name) || (name !== '__esModule' && own.call(exports, name))) continue;
              var resolution = resolve(ownerKey, name, [], false, null, null);
              if (resolution && resolution !== ambiguous) bindings.push([name, resolution]);
            }
          } while (before !== __thaw_bundle_edge_version && linksReady(ownerKey, []));
          if (!linksReady(ownerKey, [])) return;
          for (var b = 0; b < bindings.length; b++) {
            var binding = bindings[b];
            if (binding[0] !== '__esModule' && own.call(exports, binding[0])) continue;
            Object.defineProperty(exports, binding[0], { enumerable: true, get: function(r) { return function() { return read(r); }; }(binding[1]) });
          }
          linked = true;
          delete __thaw_bundle_star_linkers[ownerKey];
        } finally { linking = false; }
      };
      __thaw_bundle_star_linkers[ownerKey] = link;
      link();
    },
    defaultImport: function(spec, value) {
      var target = targetOf(ownerKey, spec);
      return target && graphOf(target.key) ? value.default : value && value.__esModule ? value.default : value;
    },
    names: function(spec, value) {
      var target = targetOf(ownerKey, spec);
      return target && graphOf(target.key) ? namesOf(target.key, [], ownerKey, spec) : (value == null ? [] : Object.keys(value).filter(function(name) { return name !== '__esModule'; }));
    },
    resolve: function(name) { var found = resolve(ownerKey, name, [], false, null, null); return found === ambiguous ? null : found; },
    read: read
  };
}
"#;

/// Renders `modules` into one JS string: a small embedded CommonJS
/// module-system emulation (a factory + a per-module require-spec-to-key
/// map, both precomputed statically -- no runtime path resolution needed
/// in the emitted JS at all), then a final `module.exports = ...` that
/// invokes `main_key`. The result is exactly the kind of single JS
/// string thaw-bridge's `wrap_as_commonjs_module` already knows how to
/// run: it doesn't need to know or care that this is a bundle rather
/// than one file.
///
/// Everything except the final `module.exports = ` assignment is inside
/// an IIFE, deliberately never touching global scope: multiple
/// `--use`'d packages all get `loadScript`'d into the *same* shared
/// QuickJS-NG global context (thaw-bridge's `wrap_as_commonjs_module`,
/// called once per package), so if these helpers were plain globals, a
/// second package's bundle would stomp the first's `__thaw_bundle_cache`/
/// `__thaw_bundle_require`/etc. the moment it loaded. That's invisible
/// for a module that only calls `require` eagerly at load time (already
/// finished and cached by then), but a *lazy* internal require --
/// deferred inside a function body, called only after a later package
/// has overwritten the globals -- would silently resolve against the
/// wrong package's module map. The IIFE's closures keep each package's
/// module system private to itself regardless of what loads after it.
fn render_bundle(main_key: &str, modules: &[BundledModule]) -> String {
    let mut out = String::from("module.exports = (function(require) {\n");

    out.push_str("var __thaw_bundle_exports = globalThis.__thaw_bundle_exports || (globalThis.__thaw_bundle_exports = {});\n");
    out.push_str("var __thaw_bundle_cache = {};\n");
    out.push_str("var __thaw_bundle_edges = Object.create(null);\n");
    out.push_str("var __thaw_bundle_star_linkers = Object.create(null);\n");
    out.push_str("var __thaw_bundle_edge_version = 0;\n");
    out.push_str("var __thaw_bundle_factories = {\n");
    for module in modules {
        let asynchronous = if module.async_module { "async " } else { "" };
        if let Some(origin) = &module.origin_parameter {
            out.push_str(&format!(
                "{}: function({origin}) {{ return {asynchronous}function(module, exports, require, requireAsync, __filename, __dirname, __thaw_require) {{\n{}\n}}; }},\n",
                js_string_literal(&module.key),
                module.source,
            ));
        } else {
            out.push_str(&format!(
                "{}: {asynchronous}function(module, exports, require, requireAsync, __filename, __dirname, __thaw_require) {{\n{}\n}},\n",
                js_string_literal(&module.key),
                module.source,
            ));
        }
    }
    out.push_str("};\n");

    out.push_str("var __thaw_bundle_import_maps = {\n");
    for module in modules {
        out.push_str(&format!("{}: {{", js_string_literal(&module.key)));
        for (spec, target) in &module.imports {
            out.push_str(&format!(
                "{}: {}, ",
                js_string_literal(spec),
                js_string_literal(target)
            ));
        }
        out.push_str("},\n");
    }
    out.push_str("};\n");

    out.push_str("var __thaw_bundle_known_package_maps = {\n");
    for module in modules {
        out.push_str(&format!("{}: {},\n", js_string_literal(&module.key),
            serde_json::to_string(&module.known_packages).expect("package names are serializable")));
    }
    out.push_str("};\n");

    out.push_str("var __thaw_bundle_require_maps = {\n");
    for module in modules {
        out.push_str(&format!("{}: {{", js_string_literal(&module.key)));
        for (spec, target) in &module.requires {
            out.push_str(&format!(
                "{}: {}, ",
                js_string_literal(spec),
                js_string_literal(target)
            ));
        }
        out.push_str("},\n");
    }
    out.push_str("};\n");

    let export_graphs = modules.iter().filter_map(|module| {
        module.export_graph.as_ref().map(|graph| (module.key.clone(), graph.clone()))
    }).collect::<serde_json::Map<String, serde_json::Value>>();
    let export_graph_json = serde_json::Value::Object(export_graphs).to_string();
    out.push_str(&format!("var __thaw_bundle_export_graphs = JSON.parse({});\n", js_string_literal(&export_graph_json)));
    out.push_str(STAR_ORIGIN_RUNTIME);

    out.push_str(
        "function __thaw_bundle_target(map, spec) {\n\
         \x20\x20if (Object.prototype.hasOwnProperty.call(map, spec)) return { key: map[spec], factory: map[spec] };\n\
         \x20\x20var query = spec.indexOf('?');\n\
         \x20\x20var fragment = spec.indexOf('#', 1);\n\
         \x20\x20var suffixAt = query < 0 ? fragment : (fragment < 0 ? query : Math.min(query, fragment));\n\
         \x20\x20if (suffixAt < 0) return null;\n\
         \x20\x20var base = spec.slice(0, suffixAt);\n\
         \x20\x20if (!Object.prototype.hasOwnProperty.call(map, base)) return null;\n\
         \x20\x20var factory = map[base];\n\
         \x20\x20return { key: factory + spec.slice(suffixAt), factory: factory };\n\
         }\n\
         function __thaw_bundle_import_missing(known, spec) {\n\
         \x20\x20spec = String(spec);\n\
         \x20\x20if (spec.charAt(0) === '#') return true;\n\
         \x20\x20var query = spec.indexOf('?'), fragment = spec.indexOf('#', 1);\n\
         \x20\x20var suffixAt = query < 0 ? fragment : (fragment < 0 ? query : Math.min(query, fragment));\n\
         \x20\x20if (suffixAt >= 0) spec = spec.slice(0, suffixAt);\n\
         \x20\x20var slash = spec.indexOf('/');\n\
         \x20\x20var next = spec.charAt(0) === '@' && slash >= 0 ? spec.indexOf('/', slash + 1) : slash;\n\
         \x20\x20var name = next < 0 ? spec : spec.slice(0, next);\n\
         \x20\x20return known.indexOf(name) >= 0;\n\
         }\n\
         function __thaw_bundle_require(key, factoryKey) {\n\
         \x20\x20if (String(factoryKey || key).endsWith('.node') && require.addon) return require.addon();\n\
         \x20\x20if (!(key in __thaw_bundle_cache)) {\n\
         \x20\x20\x20\x20factoryKey = factoryKey || key;\n\
         \x20\x20\x20\x20var mod = { exports: {}, filename: '/thaw_modules/' + (factoryKey || key) };\n\
         \x20\x20\x20\x20__thaw_bundle_cache[key] = mod;\n\
         \x20\x20\x20\x20var map = __thaw_bundle_require_maps[factoryKey] || {};\n\
         \x20\x20\x20\x20var importMap = __thaw_bundle_import_maps[factoryKey] || {};\n\
         \x20\x20\x20\x20var knownPackages = __thaw_bundle_known_package_maps[factoryKey] || [];\n\
         \x20\x20\x20\x20var localRequire = function(spec) {\n\
         \x20\x20\x20\x20\x20\x20if ((spec === 'bindings' || spec === 'node-gyp-build') && typeof require.addon === 'function') return require.addon;\n\
         \x20\x20\x20\x20\x20\x20var target = __thaw_bundle_target(map, spec);\n\
         \x20\x20\x20\x20\x20\x20if (target) return __thaw_bundle_require(target.key, target.factory);\n\
         \x20\x20\x20\x20\x20\x20return require(spec);\n\
         \x20\x20\x20\x20};\n\
         \x20\x20\x20\x20localRequire.addon = require.addon;\n\
         \x20\x20\x20\x20localRequire.resolve = function(spec) { var target = __thaw_bundle_target(map, String(spec)); return target ? target.key : String(spec); };\n\
         \x20\x20\x20\x20var localImport = function(spec) { var target = __thaw_bundle_target(importMap, spec); if (target) return __thaw_bundle_require(target.key, target.factory); if (__thaw_bundle_import_missing(knownPackages, spec)) throw new Error('Cannot resolve import ' + spec); return require(spec); };\n\
         \x20\x20\x20\x20var localRequireAsync = function(spec) {\n\
         \x20\x20\x20\x20\x20\x20var target = __thaw_bundle_target(importMap, spec);\n\
         \x20\x20\x20\x20\x20\x20if (!target) return Promise.resolve().then(function() { if (__thaw_bundle_import_missing(knownPackages, spec)) throw new Error('Cannot resolve import ' + spec); return require(spec); });\n\
         \x20\x20\x20\x20\x20\x20return Promise.resolve().then(function() { var value = __thaw_bundle_require(target.key, target.factory); return __thaw_bundle_cache[target.key].ready.then(function() { return value; }); });\n\
         \x20\x20\x20\x20};\n\
         \x20\x20\x20\x20var filename = '/thaw_modules/' + factoryKey, slash = filename.lastIndexOf('/'), dirname = slash < 0 ? '.' : filename.slice(0, slash);\n\
         \x20\x20\x20\x20var initialized;\n\
         \x20\x20\x20\x20try { var factory = __thaw_bundle_factories[factoryKey]; if (Object.prototype.hasOwnProperty.call(__thaw_bundle_export_graphs, factoryKey)) factory = factory(__thaw_bundle_origin_for(key)); initialized = factory(mod, mod.exports, localRequire, localRequireAsync, filename, dirname, localImport); } catch (error) { delete __thaw_bundle_cache[key]; delete __thaw_bundle_edges[key]; delete __thaw_bundle_star_linkers[key]; throw error; }\n\
         \x20\x20\x20\x20mod.ready = Promise.resolve(initialized).then(function() { return mod.exports; }, function(error) { delete __thaw_bundle_cache[key]; delete __thaw_bundle_edges[key]; delete __thaw_bundle_star_linkers[key]; throw error; });\n\
         \x20\x20}\n\
         \x20\x20return __thaw_bundle_cache[key].exports;\n\
         }\n\
         function __thaw_bundle_create_require(base) {\n\
         \x20\x20var text = String(base || '');\n\
         \x20\x20var keys = Object.keys(__thaw_bundle_require_maps);\n\
         \x20\x20var factoryKey = keys.indexOf(text) >= 0 ? text : keys.reduce(function(best, key) { return text.endsWith('/' + key) && (!best || key.length > best.length) ? key : best; }, null);\n\
         \x20\x20var map = __thaw_bundle_require_maps[factoryKey] || {};\n\
         \x20\x20var created = function(spec) { if ((String(spec) === 'bindings' || String(spec) === 'node-gyp-build') && typeof require.addon === 'function') return require.addon; spec = String(spec); if (Object.prototype.hasOwnProperty.call(__thaw_bundle_exports, spec)) return __thaw_bundle_exports[spec]; var target = __thaw_bundle_target(map, spec); if (!target && !factoryKey) { for (var index = 0; index < keys.length && !target; index++) target = __thaw_bundle_target(__thaw_bundle_require_maps[keys[index]] || {}, spec); } if (target) return __thaw_bundle_require(target.key, target.factory); return require(spec); };\n\
         \x20\x20created.addon = require.addon;\n\
         \x20\x20created.resolve = function(spec) { var target = __thaw_bundle_target(map, String(spec)); return target ? target.key : String(spec); };\n\
         \x20\x20created.cache = __thaw_bundle_cache; return created;\n\
         }\n\
         function __thaw_bundle_create_import(base) {\n\
         \x20\x20var map = __thaw_bundle_import_maps[String(base)] || {};\n\
         \x20\x20var known = __thaw_bundle_known_package_maps[String(base)] || [];\n\
         \x20\x20return function(spec) { spec = String(spec); var target = __thaw_bundle_target(map, spec); if (target) return __thaw_bundle_require(target.key, target.factory); if (__thaw_bundle_import_missing(known, spec)) throw new Error('Cannot resolve import ' + spec); return require(spec); };\n\
         }\n\
         function __thaw_bundle_create_import_async(base) {\n\
         \x20\x20var importModule = __thaw_bundle_create_import(base);\n\
         \x20\x20return function(spec) { return Promise.resolve().then(function() { var target = __thaw_bundle_target(__thaw_bundle_import_maps[String(base)] || {}, String(spec)); var value = importModule(spec); return target ? __thaw_bundle_cache[target.key].ready.then(function() { return value; }) : value; }); };\n\
         }\n\
         function __thaw_bundle_register_worker_main(key, mod) {\n\
         \x20\x20if (Object.prototype.hasOwnProperty.call(__thaw_bundle_cache, key)) throw new Error('Worker entry already initialized');\n\
         \x20\x20mod.filename = '/thaw_modules/' + key;\n\
         \x20\x20var resolveReady, rejectReady;\n\
         \x20\x20mod.ready = new Promise(function(resolve, reject) { resolveReady = resolve; rejectReady = reject; });\n\
         \x20\x20__thaw_bundle_cache[key] = mod;\n\
         \x20\x20return function(succeeded, error) { if (succeeded) resolveReady(mod.exports); else rejectReady(error); };\n\
         }\n\
         globalThis.__thaw_bundle_create_require = __thaw_bundle_create_require;\n\
         globalThis.__thaw_bundle_create_import = __thaw_bundle_create_import;\n\
         globalThis.__thaw_bundle_create_import_async = __thaw_bundle_create_import_async;\n\
         var __thaw_worker_bundle_source =\n\
         \x20\x20'(function() {\\nvar __thaw_bundle_exports = globalThis.__thaw_bundle_exports || (globalThis.__thaw_bundle_exports = {});\\nvar __thaw_bundle_cache = {};\\nvar __thaw_bundle_edges = Object.create(null);\\nvar __thaw_bundle_star_linkers = Object.create(null);\\nvar __thaw_bundle_edge_version = 0;\\nvar __thaw_bundle_factories = {' +\n\
         \x20\x20Object.keys(__thaw_bundle_factories).map(function(key) { return JSON.stringify(key) + ': ' + __thaw_bundle_factories[key].toString(); }).join(',\\n') +\n\
         \x20\x20'};\\nvar __thaw_bundle_require_maps = ' + JSON.stringify(__thaw_bundle_require_maps) + ';\\n' +\n\
         \x20\x20'var __thaw_bundle_import_maps = ' + JSON.stringify(__thaw_bundle_import_maps) + ';\\n' +\n\
         \x20\x20'var __thaw_bundle_known_package_maps = ' + JSON.stringify(__thaw_bundle_known_package_maps) + ';\\n' +\n\
         \x20\x20'var __thaw_bundle_export_graphs = JSON.parse(' + JSON.stringify(JSON.stringify(__thaw_bundle_export_graphs)) + ');\\n' +\n\
         \x20\x20__thaw_bundle_origin_for.toString() + '\\n' + __thaw_bundle_target.toString() + '\\n' + __thaw_bundle_import_missing.toString() + '\\n' + __thaw_bundle_require.toString() + '\\n' + __thaw_bundle_create_require.toString() + '\\n' + __thaw_bundle_create_import.toString() + '\\n' + __thaw_bundle_create_import_async.toString() + '\\n' + __thaw_bundle_register_worker_main.toString() + '\\n' +\n\
         \x20\x20'globalThis.__thaw_bundle_create_require = __thaw_bundle_create_require;\\nglobalThis.__thaw_bundle_create_import = __thaw_bundle_create_import;\\nglobalThis.__thaw_bundle_create_import_async = __thaw_bundle_create_import_async;\\nglobalThis.__thaw_bundle_worker_origin = __thaw_bundle_origin_for;\\nglobalThis.__thaw_bundle_register_worker_main = __thaw_bundle_register_worker_main;\\n})();\\n';\n\
         globalThis.__thaw_worker_bundle_source = __thaw_worker_bundle_source;\n",
    );

    if modules.iter().any(|module| module.key == "node:http") {
        // This fetch polyfill decompresses a `Content-Encoding: br`
        // response through `DecompressionStream('br')` unconditionally
        // (real servers send Brotli-encoded bodies often -- e.g. any
        // Cloudflare-fronted site -- once we ask for it via
        // `Accept-Encoding`, which this shim always does below) even
        // though the literal word "brotli"/"Brotli" never otherwise
        // appears anywhere in this shim or in a typical package's own
        // bundled source (only the short code `'br'` does). thaw-cli's
        // `source_uses_brotli` (build.rs) is a blind substring scan for
        // that literal word to decide whether to compile thaw-quickjs's
        // optional Brotli decoder in at all -- so without this comment,
        // a real `br`-compressed response silently gets a stubbed-out
        // decoder no matter how large or small the package bundle is.
        // Found via ky's real HTTPS traffic to a Cloudflare-fronted
        // site (nothing to do with ky itself -- any fetch of a Brotli-
        // compressing server hits this).
        out.push_str("// uses brotli decompression ('br' content-encoding)\n");
        out.push_str(
            r#"if (typeof globalThis.fetch !== 'function') {
  globalThis.fetch = function(input, init) {
    var request;
    try { request = new Request(input, init); } catch (error) { return Promise.reject(error); }
    return Promise.resolve().then(async function() {
      var body = request.body ? await request.bytes() : new Uint8Array(), redirects = 0;
      function aborted() {
        var reason = request.signal && request.signal.reason;
        if (reason !== undefined) return reason;
        var error = new Error('This operation was aborted'); error.name = 'AbortError'; return error;
      }
      if (request.signal && request.signal.aborted) throw aborted();
      return new Promise(function(resolve, reject) {
        var active = null, settled = false;
        function finishReject(error) { if (settled) return; settled = true; cleanup(); reject(error); }
        function cleanup() { if (request.signal) request.signal.removeEventListener('abort', onAbort); }
        function onAbort() { if (active && typeof active.destroy === 'function') active.destroy(); finishReject(aborted()); }
        if (request.signal) request.signal.addEventListener('abort', onAbort, { once: true });
        var initialHeaders = {}; request.headers.forEach(function(value, name) { initialHeaders[name] = value; }); if (initialHeaders['accept-encoding'] === undefined) initialHeaders['accept-encoding'] = 'gzip, deflate, br';
        function dispatch(url, method, bytes, redirected, headers) {
          var parsed;
          try { parsed = new URL(url); } catch (error) { finishReject(error); return; }
          if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') { finishReject(new TypeError('fetch only supports http: and https: URLs')); return; }
          if (parsed.username || parsed.password) { finishReject(new TypeError('Request URL cannot contain credentials')); return; }
          parsed.hash = '';
          var transport = __thaw_bundle_require(parsed.protocol === 'https:' ? 'node:https' : 'node:http');
          if ((method === 'GET' || method === 'HEAD') && bytes.length) { bytes = new Uint8Array(); delete headers['content-length']; }
          var options = { method: method, headers: headers, signal: request.signal };
          try {
            active = transport.request(parsed.href, options, function(incoming) {
              var status = Number(incoming.statusCode), location = incoming.headers && incoming.headers.location;
              if (location && [301, 302, 303, 307, 308].indexOf(status) >= 0) {
                if (request.redirect === 'error') { finishReject(new TypeError('fetch redirect mode is set to error')); return; }
                if (request.redirect === 'follow') {
                  if (++redirects > 20) { finishReject(new TypeError('fetch redirect count exceeded')); return; }
                  var nextUrl = new URL(String(location), parsed), nextMethod = method, nextBody = bytes, nextHeaders = Object.assign({}, headers);
                  if (status === 303 && method !== 'HEAD' || (status === 301 || status === 302) && method === 'POST') { nextMethod = 'GET'; nextBody = new Uint8Array(); Object.keys(nextHeaders).forEach(function(name) { if (name.indexOf('content-') === 0) delete nextHeaders[name]; }); }
                  if (nextUrl.origin !== parsed.origin) ['authorization', 'proxy-authorization', 'cookie', 'cookie2'].forEach(function(name) { delete nextHeaders[name]; });
                  dispatch(nextUrl.href, nextMethod, nextBody, true, nextHeaders); return;
                }
              }
              var responseHeaders = new Headers();
              if (Array.isArray(incoming.rawHeaders)) for (var index = 0; index + 1 < incoming.rawHeaders.length; index += 2) responseHeaders.append(incoming.rawHeaders[index], incoming.rawHeaders[index + 1]);
              else if (incoming.headers) Object.keys(incoming.headers).forEach(function(name) { var value = incoming.headers[name]; if (Array.isArray(value)) value.forEach(function(item) { responseHeaders.append(name, item); }); else if (value !== undefined) responseHeaders.append(name, value); });
              var noBody = method === 'HEAD' || status === 101 || status === 204 || status === 205 || status === 304, controller, bodyAbort;
              function cleanupBody() { if (request.signal && bodyAbort) request.signal.removeEventListener('abort', bodyAbort); }
              var stream = noBody ? null : new ReadableStream({ start: function(value) { controller = value; }, cancel: function(reason) { cleanupBody(); if (incoming.destroy) incoming.destroy(reason); } });
              if (stream) {
                incoming.on('data', function(chunk) { if (controller) controller.enqueue(chunk instanceof Uint8Array ? chunk : new Uint8Array(chunk)); });
                incoming.on('end', function() { cleanupBody(); if (controller) controller.close(); });
                incoming.on('error', function(error) { cleanupBody(); if (controller) controller.error(error); });
                incoming.on('aborted', function() { cleanupBody(); if (controller) controller.error(new TypeError('terminated')); });
                bodyAbort = function() { cleanupBody(); if (controller) controller.error(aborted()); if (incoming.destroy) incoming.destroy(); };
                if (request.signal) request.signal.addEventListener('abort', bodyAbort, { once: true });
                var contentEncoding = String(responseHeaders.get('content-encoding') || '').trim().toLowerCase();
                if (contentEncoding === 'gzip' || contentEncoding === 'deflate' || contentEncoding === 'br') stream = stream.pipeThrough(new DecompressionStream(contentEncoding));
              }
              var response;
              try { response = new Response(stream, { status: status, statusText: incoming.statusMessage || '', headers: responseHeaders }); }
              catch (error) { finishReject(error); return; }
              if (globalThis.__thaw_set_response_metadata) globalThis.__thaw_set_response_metadata(response, parsed.href, redirected);
              if (!settled) { settled = true; cleanup(); resolve(response); }
            });
            active.once('error', function(error) { var failure = new TypeError('fetch failed'); failure.cause = error; finishReject(failure); });
            if (bytes.length) active.write(bytes);
            active.end();
          } catch (error) { finishReject(error); }
        }
        dispatch(request.url, request.method, body, false, initialHeaders);
      });
    });
  };
}
"#,
        );
    }

    out.push_str(&format!(
        "var __thaw_bundle_entry_key = {};\n\
         var __thaw_bundle_entry = __thaw_bundle_require(__thaw_bundle_entry_key);\n\
         __thaw_bundle_exports[__thaw_bundle_entry_key] = __thaw_bundle_entry;\n\
         globalThis.__thaw_module_ready = __thaw_bundle_cache[__thaw_bundle_entry_key].ready;\n\
         return __thaw_bundle_entry;\n",
        js_string_literal(main_key)
    ));
    out.push_str("})(globalThis.require);\n");

    out
}

/// A double-quoted JS string literal for `s` -- used for module-map keys
/// and require specs, which in practice are always simple path-like
/// strings, but escaped properly regardless.
fn js_string_literal(s: &str) -> String {
    let mut out = String::from("\"");
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}
