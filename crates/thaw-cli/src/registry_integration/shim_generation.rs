/// Resolves each `--use`d package against the local registry
/// (thaw-registry; `registry_dir` defaults to `thaw_modules/`),
/// generating its callable surface exactly like `generate_bridge_shims`
/// does for a standalone `.d.ts` -- but additionally auto-linking the
/// package's `native.a` if it ships one (replacing a manual `--link`),
/// and collecting its `bundle.js` (if any) into a single generated
/// `__thaw_module_init` (thaw-bridge's `generate_module_init`) so it's
/// auto-loaded before user code runs (replacing a manual `loadScript`
/// call). Returns the generated shim text, the native lib paths to
/// link, and any cross-package name-collision rewrites the caller must
/// also apply to the user's own source (`rewrite_qualified_calls`).
fn generate_registry_shims(
    registry_dir: &Path,
    use_packages: &[String],
    user_source: &str,
    embed_native_addons: bool,
    output: &Path,
    external_native_staging: Option<&Path>,
) -> Result<RegistryShims, String> {
    let observed_arities = observed_member_call_arities(user_source)?;
    let observed_identifier_arities = observed_identifier_call_arities(user_source)?;
    let observed_bare_member_objects = observed_bare_member_object_identifiers(user_source)?;
    let mut observed_function_arities = observed_identifier_arities.clone();
    for (name, arities) in &observed_arities {
        observed_function_arities
            .entry(name.clone())
            .or_default()
            .extend(arities);
    }
    let mut resolved = Vec::new();
    for name in use_packages {
        let package = if name.starts_with("node:") {
            thaw_registry::resolve_builtin(name)?
        } else {
            thaw_registry::resolve(registry_dir, name)?
        };
        let mut functions = thaw_bridge::parse_dts(&package.dts_source)
            .map_err(|e| format!("failed to parse `{name}`'s package.d.ts: {e}"))?;
        let classes = thaw_bridge::parse_dts_classes(&package.dts_source)
            .map_err(|e| format!("failed to parse `{name}`'s package.d.ts classes: {e}"))?;
        let values = thaw_bridge::parse_dts_values(&package.dts_source)
            .map_err(|e| format!("failed to parse `{name}`'s package.d.ts values: {e}"))?;
        let commonjs_export_name = commonjs_export_name(&package.dts_source)
            .map_err(|e| format!("failed to parse `{name}`'s CommonJS export: {e}"))?;
        let called_commonjs_namespace_properties = called_commonjs_namespace_properties(
            &package.dts_source,
            commonjs_export_name.as_deref(),
            &observed_arities,
        )
        .map_err(|e| format!("failed to parse `{name}`'s CommonJS properties: {e}"))?;
        for property in &called_commonjs_namespace_properties {
            if functions.iter().any(|function| &function.name == property) {
                continue;
            }
            let arities = &observed_arities[property];
            let minimum = *arities.first().expect("called properties have an arity");
            let maximum = *arities.last().expect("called properties have an arity");
            functions.push(thaw_bridge::DtsFunction {
                name: property.clone(),
                generic: None,
                params: (0..maximum)
                    .map(|index| {
                        (
                            format!("arg{index}"),
                            thaw_bridge::DtsType::Native(thaw_hir::HirType::Json),
                        )
                    })
                    .collect(),
                required_params: minimum,
                rest_param: None,
                ret: thaw_bridge::DtsType::Native(thaw_hir::HirType::JsValue),
            });
        }
        // Whether there's actually a `native.a` to link a FastPath
        // signature against -- without one, a fully-primitive real npm
        // function (e.g. date-fns's `daysToWeeks(days: number): number`)
        // would still classify FastPath on type shape alone and produce
        // an unresolvable `declare function`, even though a working JS
        // implementation is sitting right there in `bundle.js`. See
        // `thaw_bridge::effective_classifications`'s doc comment.
        let native_lib_available = package.native_lib.is_some() || is_native_builtin(&package.name);
        let classifications =
            thaw_bridge::effective_classifications(&functions, native_lib_available);
        let factory_class_returns = thaw_bridge::function_return_named_types(&package.dts_source)
            .into_iter()
            .filter(|(_, class_name)| classes.iter().any(|class| &class.name == class_name))
            .collect();
        let namespace_self_aliases =
            thaw_bridge::self_referential_namespace_aliases(&package.dts_source);
        let mut type_only_exports = thaw_bridge::exported_type_names(&package.dts_source);
        type_only_exports.extend(classes.iter().map(|class| class.name.clone()));
        let nested_namespaces = thaw_bridge::nested_namespace_members(&package.dts_source);
        resolved.push(ResolvedPackage {
            name: package.name.clone(),
            commonjs_export_name,
            called_commonjs_namespace_properties: called_commonjs_namespace_properties
                .into_iter()
                .collect(),
            functions,
            values,
            classes,
            classifications,
            native_lib: package.native_lib,
            native_addon: package.native_addon,
            native_dependencies: package.native_dependencies,
            bundle_js: package.bundle_js,
            factory_class_returns,
            namespace_self_aliases,
            type_only_exports,
            nested_namespaces,
        });
    }

    // Every generated top-level name -- FastPath ambient declaration or
    // Fallback wrapper alike -- would otherwise land in the *same* flat
    // global scope (QuickJS-NG globals for Fallback, the LLVM module's
    // own symbol table for FastPath). Two different packages exporting
    // the same name (e.g. `qs` and `@hapi/hoek` both export `stringify`)
    // used to silently collide: whichever package's shim/binding ran
    // last won, with no error -- exactly the kind of order-dependent
    // surprise this project has treated as a bug to catch loudly every
    // other time it showed up (see thaw-bridge's `classify_all`, for the
    // same problem one level down, *within* one `.d.ts`'s own
    // overloads). A collision where every involved package classifies
    // the name as Fallback is auto-resolved below by dropping the bare
    // name for it (`QualifiedFallback::suppress_bare`), forcing
    // qualified syntax (`qs.stringify(x)`, rewritten to a package-
    // qualified alias -- see `rewrite_qualified_calls`); a
    // FastPath-involved collision is a real native-symbol clash this
    // can't paper over, so it stays a hard error.
    let mut declared_by: std::collections::HashMap<String, Vec<(String, bool)>> =
        std::collections::HashMap::new();
    for pkg in &resolved {
        for (name, classification) in &pkg.classifications {
            let is_fast_path = matches!(classification, thaw_bridge::Classification::FastPath(_));
            declared_by
                .entry(name.clone())
                .or_default()
                .push((pkg.name.clone(), is_fast_path));
        }
    }
    for (name, packages) in &declared_by {
        if packages.len() < 2 {
            continue;
        }
        if let Some((fast_path_pkg, _)) = packages.iter().find(|(_, is_fast_path)| *is_fast_path) {
            let other_pkg = packages
                .iter()
                .map(|(p, _)| p.as_str())
                .find(|p| *p != fast_path_pkg)
                .unwrap_or(fast_path_pkg);
            return Err(format!(
                "`{name}` is declared by both `{fast_path_pkg}` and `{other_pkg}` -- automatic \
                 resolution only covers Fallback functions, not a Fast path native symbol clash"
            ));
        }
    }
    let colliding: std::collections::HashSet<&String> = declared_by
        .iter()
        .filter(|(_, pkgs)| pkgs.len() > 1)
        .map(|(name, _)| name)
        .collect();
    let qualifier_by_package =
        package_qualifier_identifiers(resolved.iter().map(|package| package.name.as_str()));

    // Every Fallback name of every `--use`d package also gets a package-
    // qualified alias -- not just names that actually collide -- so
    // `pkg.name(...)` syntax works consistently for any `--use`d
    // package's function, whether or not `name` happens to collide with
    // some other package (see `thaw_bridge::QualifiedFallback`'s doc
    // comment: `suppress_bare` is the only thing collision status
    // changes). `rewrites` is `(package, name, alias)`, for rewriting
    // `pkg.name(...)` call syntax in the user's own source (see
    // `rewrite_qualified_calls`).
    let mut qualified_by_package: std::collections::HashMap<
        String,
        Vec<thaw_bridge::QualifiedFallback>,
    > = std::collections::HashMap::new();
    let mut rewrites: Vec<QualifiedCallRewrite> = Vec::new();
    for pkg in &resolved {
        for (name, classification) in &pkg.classifications {
            if !matches!(classification, thaw_bridge::Classification::Fallback { .. }) {
                continue;
            }
            let alias = format!("{}_{name}", sanitize_identifier(&pkg.name));
            let qualified_key = format!("{}::{name}", pkg.name);
            qualified_by_package
                .entry(pkg.name.clone())
                .or_default()
                .push(thaw_bridge::QualifiedFallback {
                    name: name.clone(),
                    alias: alias.clone(),
                    qualified_key,
                    suppress_bare: colliding.contains(name),
                });
            rewrites.push((qualifier_by_package[&pkg.name].clone(), name.clone(), alias));
        }
    }

    /// `(package_name, js_source, fallback_function_names, qualified_aliases,
    /// nested_namespace_aliases)` -- kept as owned data so the borrowed
    /// `ModuleBundle`s built from it below can outlive the loop that
    /// collects it.
    type PendingBundle = (
        String,
        String,
        Vec<String>,
        Vec<String>,
        Vec<(String, String)>,
        Vec<(String, String)>,
        Vec<(String, String, String, String)>,
    );
    type PendingNativeAddon = (String, Vec<u8>, Vec<Vec<u8>>, Option<String>);
    type PendingNativeAddonPath = (String, String, Vec<String>, Option<String>);

    let mut shim = String::new();
    let mut typed_targets: std::collections::HashMap<(String, String), String> =
        std::collections::HashMap::new();
    let mut value_targets: std::collections::HashMap<(String, String), String> =
        std::collections::HashMap::new();
    let mut jit_targets = std::collections::HashSet::new();
    let mut jit_fallback_reasons = JitFallbackReasons::new();
    // Names `union_overload_dispatch_declaration` already claims with a
    // runtime `typeof`-based dispatcher. The argument-shape-scoring loop
    // below (which drives each overload through `typed_dynamic_
    // declaration` a second time, with the call site's *observed*
    // arities) must still skip them -- doing that over again would
    // re-declare the very same symbols `union_overload_dispatch_
    // declaration` already emitted, this time under a possibly
    // different symbol name (arities can change the suffix), which is
    // how an overload's own wrapper (and the omitted-parameter-mask
    // signature thaw-hir's own pipeline derives from it) went missing
    // at link time the one time this guard was removed instead of
    // fixed properly. `union_overload_dispatch_declaration` return its
    // *own* per-overload rewrite candidates instead (`union_dispatch_
    // overload_rewrites` below), reusing the exact symbols it already
    // declared: a real call site whose argument shape statically
    // matches one overload gets rewritten straight to that overload's
    // own precisely-typed symbol, narrowing away from the dispatcher's
    // necessarily Union-typed signature (real case: `ms`'s `(value:
    // number, options?): string` vs `(value: string): number` --
    // without this, `const n: number = ms("2 days")` fails to type-
    // check against the dispatcher's `string | number` return type even
    // though the runtime dispatch itself has always worked correctly).
    let mut union_dispatched_names: std::collections::HashSet<(String, String)> =
        std::collections::HashSet::new();
    let mut union_dispatch_overload_rewrites: Vec<FallbackFunctionOverloadRewrite> = Vec::new();
    let mut fallback_function_overload_rewrites: Vec<FallbackFunctionOverloadRewrite> = Vec::new();
    let mut class_targets: std::collections::HashMap<(String, String), String> =
        std::collections::HashMap::new();
    let mut class_rewrites = Vec::new();
    let factory_class_rewrites: Vec<FactoryClassRewrite> = resolved
        .iter()
        .flat_map(|pkg| {
            pkg.factory_class_returns
                .iter()
                .map(|(function, class)| (function.clone(), class.clone()))
        })
        .collect();
    let mut class_method_rewrites = Vec::new();
    let mut callback_instance_rewrites = Vec::new();
    let mut static_class_method_rewrites = Vec::new();
    let mut class_getter_rewrites = Vec::new();
    let mut class_setter_rewrites = Vec::new();
    let mut static_class_getter_rewrites = Vec::new();
    let mut static_class_setter_rewrites = Vec::new();
    let mut native_libs = Vec::new();
    let mut bundles: Vec<PendingBundle> = Vec::new();
    let mut native_addons: Vec<PendingNativeAddon> = Vec::new();
    let mut native_addon_paths: Vec<PendingNativeAddonPath> = Vec::new();
    let no_qualified: Vec<thaw_bridge::QualifiedFallback> = Vec::new();

    for pkg in &resolved {
        let native_lib_available = pkg.native_lib.is_some() || is_native_builtin(&pkg.name);
        let qualified = qualified_by_package.get(&pkg.name).unwrap_or(&no_qualified);
        let overload_rewrite_start = fallback_function_overload_rewrites.len();
        if pkg.native_addon.is_some() && pkg.bundle_js.is_none() {
            for class in &pkg.classes {
                let helpers = generate_napi_class_constructors(class, true, &mut shim);
                if helpers.is_empty() {
                    continue;
                }
                class_targets.insert(
                    (pkg.name.clone(), class.name.clone()),
                    helpers[0].1.clone(),
                );
                class_rewrites.push((
                    qualifier_by_package[&pkg.name].clone(),
                    class.name.clone(),
                    helpers,
                ));

                for (method, symbol, argument_count, has_callback, parameter_types, return_class) in
                    generate_napi_class_method_overloads_with_callback_instances(
                        class,
                        false,
                        &observed_arities,
                        &mut shim,
                        true,
                        &mut callback_instance_rewrites,
                    )
                {
                    class_method_rewrites.push((
                        class.name.clone(),
                        method,
                        symbol,
                        argument_count,
                        has_callback,
                        parameter_types,
                        return_class,
                    ));
                }
                for getter in class.methods.iter().filter(|method| {
                    !method.is_static
                        && method.kind == thaw_bridge::DtsMethodKind::Getter
                        && method.params.is_empty()
                        && supported_class_method_return(&method.ret)
                }) {
                    let thaw_bridge::DtsType::Native(return_type) = &getter.ret else {
                        continue;
                    };
                    let Some(return_type) = render_dynamic_type(return_type) else {
                        continue;
                    };
                    let runtime_key =
                        format!("$getter${}{}", class.name, format_args!("${}", getter.name));
                    let encoded = runtime_key
                        .as_bytes()
                        .iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect::<String>();
                    let symbol = format!("__thaw_typed_napi_{encoded}");
                    shim.push_str(&format!(
                        "declare function {symbol}(receiver: JsValue): {return_type};\n"
                    ));
                    class_getter_rewrites.push((class.name.clone(), getter.name.clone(), symbol));
                }
                for setter in class.methods.iter().filter(|method| {
                    !method.is_static
                        && method.kind == thaw_bridge::DtsMethodKind::Setter
                        && method.params.len() == 1
                        && supported_class_method_param(&method.params[0].1, 0, 1)
                }) {
                    let thaw_bridge::DtsType::Native(value_type) = &setter.params[0].1 else {
                        continue;
                    };
                    let Some(rendered_type) = render_dynamic_type(value_type) else {
                        continue;
                    };
                    let runtime_key =
                        format!("$setter${}{}", class.name, format_args!("${}", setter.name));
                    let encoded = runtime_key
                        .as_bytes()
                        .iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect::<String>();
                    let symbol = format!("__thaw_typed_napi_{encoded}");
                    shim.push_str(&format!(
                        "declare function {symbol}(receiver: JsValue, value: {rendered_type}): {rendered_type};\n"
                    ));
                    class_setter_rewrites.push((
                        class.name.clone(),
                        setter.name.clone(),
                        symbol,
                        value_type.clone(),
                    ));
                }
                for getter in class.methods.iter().filter(|method| {
                    method.is_static
                        && method.kind == thaw_bridge::DtsMethodKind::Getter
                        && method.params.is_empty()
                        && supported_class_method_return(&method.ret)
                }) {
                    let thaw_bridge::DtsType::Native(return_type) = &getter.ret else {
                        continue;
                    };
                    let Some(return_type) = render_dynamic_type(return_type) else {
                        continue;
                    };
                    let runtime_key = format!(
                        "$staticgetter${}{}",
                        class.name,
                        format_args!("${}", getter.name)
                    );
                    let encoded = runtime_key
                        .as_bytes()
                        .iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect::<String>();
                    let symbol = format!("__thaw_typed_napi_{encoded}");
                    shim.push_str(&format!("declare function {symbol}(): {return_type};\n"));
                    static_class_getter_rewrites.push((
                        qualifier_by_package[&pkg.name].clone(),
                        class.name.clone(),
                        getter.name.clone(),
                        symbol,
                    ));
                }
                for setter in class.methods.iter().filter(|method| {
                    method.is_static
                        && method.kind == thaw_bridge::DtsMethodKind::Setter
                        && method.params.len() == 1
                        && supported_class_method_param(&method.params[0].1, 0, 1)
                }) {
                    let thaw_bridge::DtsType::Native(value_type) = &setter.params[0].1 else {
                        continue;
                    };
                    let Some(rendered_type) = render_dynamic_type(value_type) else {
                        continue;
                    };
                    let runtime_key = format!(
                        "$staticsetter${}{}",
                        class.name,
                        format_args!("${}", setter.name)
                    );
                    let encoded = runtime_key
                        .as_bytes()
                        .iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect::<String>();
                    let symbol = format!("__thaw_typed_napi_{encoded}");
                    shim.push_str(&format!(
                        "declare function {symbol}(value: {rendered_type}): {rendered_type};\n"
                    ));
                    static_class_setter_rewrites.push((
                        qualifier_by_package[&pkg.name].clone(),
                        class.name.clone(),
                        setter.name.clone(),
                        symbol,
                        value_type.clone(),
                    ));
                }
                for property in &class.properties {
                    let has_getter = class.methods.iter().any(|method| {
                        method.name == property.name
                            && method.is_static == property.is_static
                            && method.kind == thaw_bridge::DtsMethodKind::Getter
                    });
                    if !has_getter {
                        if let Some(symbol) = generate_napi_class_property_getter(
                            &class.name,
                            &property.name,
                            &property.ty,
                            property.is_static,
                            &mut shim,
                        ) {
                            if property.is_static {
                                static_class_getter_rewrites.push((
                                    qualifier_by_package[&pkg.name].clone(),
                                    class.name.clone(),
                                    property.name.clone(),
                                    symbol,
                                ));
                            } else {
                                class_getter_rewrites.push((
                                    class.name.clone(),
                                    property.name.clone(),
                                    symbol,
                                ));
                            }
                        }
                    }

                    let has_setter = class.methods.iter().any(|method| {
                        method.name == property.name
                            && method.is_static == property.is_static
                            && method.kind == thaw_bridge::DtsMethodKind::Setter
                    });
                    if !property.readonly && !has_setter {
                        if let Some((symbol, value_type)) = generate_napi_class_property_setter(
                            &class.name,
                            &property.name,
                            &property.ty,
                            property.is_static,
                            true,
                            &mut shim,
                        ) {
                            if property.is_static {
                                static_class_setter_rewrites.push((
                                    qualifier_by_package[&pkg.name].clone(),
                                    class.name.clone(),
                                    property.name.clone(),
                                    symbol,
                                    value_type,
                                ));
                            } else {
                                class_setter_rewrites.push((
                                    class.name.clone(),
                                    property.name.clone(),
                                    symbol,
                                    value_type,
                                ));
                            }
                        }
                    }
                }
                for (method, symbol, argument_count, has_callback, parameter_types, _) in
                    generate_napi_class_method_overloads_with_callback_instances(
                        class,
                        true,
                        &observed_arities,
                        &mut shim,
                        true,
                        &mut callback_instance_rewrites,
                    )
                {
                    static_class_method_rewrites.push((
                        qualifier_by_package[&pkg.name].clone(),
                        class.name.clone(),
                        method,
                        symbol,
                        argument_count,
                        has_callback,
                        parameter_types,
                    ));
                }
            }
        } else {
            // A Fallback (QuickJS-NG) package's classes never went
            // through any of this before -- `pkg.classes` was parsed but
            // simply never consulted here, so a class instance (real
            // example: dayjs's factory function returning its own
            // `Dayjs`) stayed a permanently opaque `JsValue` with no way
            // to call its methods. Reuses the exact same declaration
            // generator and `class_method_rewrites` consumer as the
            // native-addon branch above (`napi: false` picks the
            // QuickJS-NG symbol/backend instead) -- only instance
            // methods and (see `generate_napi_class_constructors`'s own
            // `napi` parameter) constructors and static methods. Property
            // Property reads still remain on the dynamic-value path;
            // writable instance properties use the typed setter below.
            // Real targets: dayjs/mime never needed constructor support
            // (dayjs's `Dayjs` instances come from calling its factory
            // function, mime's `Mime` instance is a ready-made package
            // export), but hono's `Hono` -- constructed directly via
            // `new Hono()` -- does.
            for class in &pkg.classes {
                let helpers = generate_napi_class_constructors(class, false, &mut shim);
                if !helpers.is_empty() {
                    class_targets.insert(
                        (pkg.name.clone(), class.name.clone()),
                        helpers[0].1.clone(),
                    );
                    class_rewrites.push((
                        qualifier_by_package[&pkg.name].clone(),
                        class.name.clone(),
                        helpers,
                    ));
                }
                for (method, symbol, argument_count, has_callback, parameter_types, return_class) in
                    generate_napi_class_method_overloads_with_callback_instances(
                        class,
                        false,
                        &observed_arities,
                        &mut shim,
                        false,
                        &mut callback_instance_rewrites,
                    )
                {
                    class_method_rewrites.push((
                        class.name.clone(),
                        method,
                        symbol,
                        argument_count,
                        has_callback,
                        parameter_types,
                        return_class,
                    ));
                }
                for (method, symbol, argument_count, has_callback, parameter_types, _) in
                    generate_napi_class_method_overloads_with_callback_instances(
                        class,
                        true,
                        &observed_arities,
                        &mut shim,
                        false,
                        &mut callback_instance_rewrites,
                    )
                {
                    static_class_method_rewrites.push((
                        qualifier_by_package[&pkg.name].clone(),
                        class.name.clone(),
                        method,
                        symbol,
                        argument_count,
                        has_callback,
                        parameter_types,
                    ));
                }
                for property in &class.properties {
                    if property.is_static || property.readonly {
                        continue;
                    }
                    if let Some((symbol, value_type)) = generate_napi_class_property_setter(
                        &class.name,
                        &property.name,
                        &property.ty,
                        false,
                        false,
                        &mut shim,
                    ) {
                        class_setter_rewrites.push((
                            class.name.clone(),
                            property.name.clone(),
                            symbol,
                            value_type,
                        ));
                    }
                }
            }
        }
        // Pre-claims a genuinely type-disjoint overload set's name (real
        // case: `ms`) with a proper dispatcher before the ordinary
        // first-successful-overload-wins loop below ever reaches it --
        // see that loop's own comment on `already_bound`. Runs only for
        // a name with more than one `.d.ts` overload at all, so an
        // ordinary non-overloaded function (the common case) doesn't pay
        // for this check.
        let mut overloaded_names: Vec<&str> = Vec::new();
        for function in &pkg.functions {
            if !overloaded_names.contains(&function.name.as_str())
                && pkg.functions.iter().filter(|f| f.name == function.name).count() > 1
            {
                overloaded_names.push(&function.name);
            }
        }
        let overloaded_names_for_argument_shape_dispatch = overloaded_names.clone();
        for name in overloaded_names {
            let is_fallback = pkg.classifications.iter().any(|(candidate, classification)| {
                candidate == name
                    && matches!(classification, thaw_bridge::Classification::Fallback { .. })
            });
            if !is_fallback {
                continue;
            }
            let overloads = pkg
                .functions
                .iter()
                .filter(|f| f.name == name)
                .collect::<Vec<_>>();
            if let Some((symbol, declaration, rewrites)) =
                union_overload_dispatch_declaration(&pkg.name, name, &overloads)
            {
                shim.push_str(&declaration);
                typed_targets.insert((pkg.name.clone(), name.to_string()), symbol);
                union_dispatched_names.insert((pkg.name.clone(), name.to_string()));
                union_dispatch_overload_rewrites.extend(rewrites);
            }
        }
        // Names for which `typed_dynamic_bare_alias` below actually
        // emitted a typed forwarding wrapper under the bare name itself
        // -- `generate_shim` skips its own, always-untyped fallback for
        // these (see its own `already_typed` parameter).
        let mut bare_aliased: std::collections::HashSet<String> = std::collections::HashSet::new();
        for function in &pkg.functions {
            let is_fallback = pkg.classifications.iter().any(|(name, classification)| {
                name == &function.name
                    && matches!(classification, thaw_bridge::Classification::Fallback { .. })
            });
            if is_fallback {
                let napi = pkg.native_addon.is_some() && pkg.bundle_js.is_none();
                let jit_operation = pkg
                    .bundle_js
                    .as_deref()
                    .filter(|_| pkg.native_addon.is_none())
                    .and_then(|source| {
                        jit_export(
                            source,
                            &function.name,
                            pkg.commonjs_export_name.as_deref() == Some(&function.name)
                                || pkg.functions.len() == 1,
                            function,
                        )
                    });
                if jit_operation.is_none() {
                    jit_fallback_reasons
                        .entry((pkg.name.clone(), function.name.clone()))
                        .or_insert_with(|| {
                            if pkg.native_addon.is_some() {
                                "native addon call requires its N-API/JavaScript wrapper boundary"
                                    .into()
                            } else {
                                jit_rejection_reason(
                                    pkg.bundle_js.as_deref().unwrap_or_default(),
                                    function,
                                )
                            }
                        });
                }
                let declaration = jit_operation
                    .as_ref()
                    .map(|operation| jit_numeric_declaration(&pkg.name, function, operation))
                    .or_else(|| {
                        let empty_arities = std::collections::BTreeSet::new();
                        let call_arities = observed_function_arities
                            .get(&function.name)
                            .unwrap_or(&empty_arities);
                        typed_dynamic_declaration(&pkg.name, function, napi, call_arities, None)
                    });
                // First successful overload wins, matching TS's own
                // overload-resolution convention of preferring the first
                // declared match: an overloaded `.d.ts` name (see
                // `Classification::classify_all`'s doc comment) can have
                // *several* overloads each independently produce a typed
                // declaration here. Every overload's declaration text
                // uses the *same* symbol name (derived only from
                // `package::name`, not the overload), so a later one
                // isn't just an unused, harmlessly-discarded alternative
                // if it's emitted too -- it's a duplicate declaration of
                // that same name with a different signature, which
                // thaw-hir has no defined behavior for. Real case:
                // lodash's `random` (5 overloads, all just arity/
                // optional-parameter variations of each other -- any one
                // of them is a reasonable enough default).
                //
                // A `typed_targets` entry may already exist here even on
                // this function's very *first* overload:
                // `union_overload_dispatch_declaration` below pre-claims
                // the name for a genuinely type-disjoint overload set
                // (real case: `ms`'s `(value: number, options?): string`
                // vs. `(value: string): number`, which the "first wins"
                // rule alone would otherwise silently narrow to just one
                // calling convention) before this loop ever reaches it.
                let already_bound = typed_targets.contains_key(&(pkg.name.clone(), function.name.clone()));
                if let Some((symbol, declaration)) = declaration.filter(|_| !already_bound) {
                    shim.push_str(&declaration);
                    // The package-qualified alias (`pkg.name(...)`,
                    // rewritten to `q.alias` -- see `rewrite_qualified_calls`)
                    // is unconditionally generated for every Fallback name
                    // regardless of collision status (see
                    // `QualifiedFallback`'s own doc comment), so it's
                    // always worth typing when possible -- qualification
                    // is exactly the mechanism a cross-package collision
                    // is disambiguated *through*, unlike the plain bare
                    // name below.
                    if let Some(q) = qualified.iter().find(|q| q.name == function.name) {
                        if let Some(alias) = typed_dynamic_bare_alias(
                            &q.alias,
                            &symbol,
                            function,
                            jit_operation.is_some(),
                            napi,
                        ) {
                            shim.push_str(&alias);
                            bare_aliased.insert(function.name.clone());
                        }
                    }
                    // Cross-package collisions (`colliding`) already force
                    // qualified syntax for the *untyped* bare name
                    // (`suppress_bare`) -- the same reasoning applies here:
                    // a typed bare-name wrapper would be just as liable to
                    // silently resolve to whichever colliding package's own
                    // `--use` happened to declare it, so it's skipped for
                    // exactly the same names. A name thaw-hir gives its own
                    // global meaning to as a bare identifier (`undefined`,
                    // `NaN`, `Infinity`) is skipped for a different reason
                    // -- see `shadows_a_thaw_literal_identifier`'s own doc
                    // comment -- but the qualified alias above is unaffected
                    // either way, so the name stays reachable through it.
                    // A real JS/TS reserved word (`in`, real example:
                    // joi's `Root.in(ref, options?): Reference`) is
                    // skipped for a third reason -- see `is_reserved_js_
                    // identifier`'s own doc comment -- again leaving the
                    // qualified alias as the only way to reach it.
                    if !colliding.contains(&function.name)
                        && !thaw_bridge::shadows_a_thaw_literal_identifier(&function.name)
                        && !thaw_bridge::is_reserved_js_identifier(&function.name)
                    {
                        if let Some(alias) = typed_dynamic_bare_alias(
                            &function.name,
                            &symbol,
                            function,
                            jit_operation.is_some(),
                            napi,
                        ) {
                            shim.push_str(&alias);
                            bare_aliased.insert(function.name.clone());
                        }
                    }
                    typed_targets.insert(
                        (pkg.name.clone(), function.name.clone()),
                        symbol.clone(),
                    );
                    if pkg
                        .called_commonjs_namespace_properties
                        .contains(&function.name)
                    {
                        if let Some(alias) = qualified
                            .iter()
                            .find(|qualified| qualified.name == function.name)
                            .map(|qualified| qualified.alias.clone())
                        {
                            fallback_function_overload_rewrites.push((
                                alias,
                                symbol.clone(),
                                function.required_params,
                                function.params.len(),
                                dts_function_param_hir_types(function),
                                None,
                            ));
                        }
                    }
                    if observed_identifier_arities.contains_key(&function.name)
                        && !overloaded_names_for_argument_shape_dispatch
                            .contains(&function.name.as_str())
                        && function.generic.as_ref().is_some_and(|generic| {
                            generic.contextual_param_types.iter().any(|ty| ty.starts_with('('))
                        })
                    {
                        fallback_function_overload_rewrites.push((
                            function.name.clone(),
                            symbol.clone(),
                            function.required_params,
                            function.params.len(),
                            dts_function_param_hir_types(function),
                            function.generic.clone(),
                        ));
                    }
                    if let (Some(generic), Some(arities)) = (
                        function.generic.as_ref(),
                        observed_identifier_arities.get(&function.name),
                    ) {
                        if function.rest_param.is_some()
                            && !overloaded_names_for_argument_shape_dispatch
                                .contains(&function.name.as_str())
                            && generic.contextual_rest_param_type.is_some()
                        {
                            let direct = symbol.replace(
                                "__thaw_typed_wrapper_",
                                "__thaw_typed_",
                            );
                            let specialized = generic_rest_array_result(generic).is_some();
                            for &arity in arities {
                                let mut params = dts_function_param_hir_types(function);
                                params.resize(arity, thaw_hir::HirType::Json);
                                fallback_function_overload_rewrites.push((
                                    function.name.clone(),
                                    format!(
                                        "{direct}{}__arity_{arity}",
                                        if specialized { "__generic_rest" } else { "" }
                                    ),
                                    arity,
                                    arity,
                                    params,
                                    Some(generic.clone()),
                                ));
                            }
                        }
                    }
                    if jit_operation.is_some() {
                        jit_targets.insert((pkg.name.clone(), function.name.clone()));
                    }
                }
            }
        }
        // Argument-shape-aware overload dispatch: an overloaded name not
        // already claimed by a `typeof`-based runtime dispatcher above
        // only ever exposes its *first* overload through the bare/
        // qualified alias just emitted -- real example: uuid's
        // `v4<TBuf extends Uint8Array = Uint8Array>(options?, buf?,
        // offset?): TBuf`, whose generic buffer-output overload is
        // otherwise unreachable no matter what a real call site passes.
        // Every overload (including whichever one became the default
        // above) gets its own freshly-suffixed declaration here,
        // independent of the bare-alias symbol -- a little redundant
        // declaration text, but far simpler than trying to splice the
        // first-wins loop's already-claimed symbol back in, and exactly
        // the "duplicate declaration under a different name for the same
        // runtime symbol" shape `typed_dynamic_declaration`'s own
        // `overload_suffix` already documents supporting. Each becomes a
        // `FallbackFunctionOverloadRewrite` candidate that `class_methods
        // .rs`'s rewrite pass picks between by arity and, on a tie,
        // `overload_type_score` against each real call site's actual
        // argument types -- the bare-alias default keeps working
        // unchanged for any call that pass doesn't recognize a better
        // match for.
        for name in &overloaded_names_for_argument_shape_dispatch {
            let name = *name;
            if union_dispatched_names.contains(&(pkg.name.clone(), name.to_string()))
                || !typed_targets.contains_key(&(pkg.name.clone(), name.to_string()))
            {
                continue;
            }
            let is_fallback = pkg.classifications.iter().any(|(candidate, classification)| {
                candidate == name
                    && matches!(classification, thaw_bridge::Classification::Fallback { .. })
            });
            if !is_fallback {
                continue;
            }
            let napi = pkg.native_addon.is_some() && pkg.bundle_js.is_none();
            let empty_arities = std::collections::BTreeSet::new();
            let call_arities = observed_function_arities
                .get(name)
                .unwrap_or(&empty_arities);
            for (index, function) in pkg.functions.iter().enumerate() {
                if function.name != name {
                    continue;
                }
                let Some((symbol, declaration)) =
                    typed_dynamic_declaration(&pkg.name, function, napi, call_arities, Some(index))
                else {
                    continue;
                };
                shim.push_str(&declaration);
                if let Some(generic) = function
                    .generic
                    .as_ref()
                    .filter(|_| function.rest_param.is_some())
                {
                    let direct = symbol.replace("__thaw_typed_wrapper_", "__thaw_typed_");
                    let specialized = generic_rest_array_result(generic).is_some();
                    for &arity in call_arities {
                        let mut params = dts_function_param_hir_types(function);
                        if let Some(contextual) = generic.contextual_param_types.first() {
                            if generic.type_params.iter().any(|(parameter, _)| {
                                contextual == &format!("{parameter}[]")
                                    || contextual.contains(&format!("<{parameter}>"))
                            }) {
                                params[0] = thaw_hir::HirType::Array(Box::new(
                                    thaw_hir::HirType::Json,
                                ));
                            }
                        }
                        params.resize(arity, thaw_hir::HirType::Json);
                        fallback_function_overload_rewrites.push((
                            name.to_string(),
                            format!(
                                "{direct}{}__arity_{arity}",
                                if specialized { "__generic_rest" } else { "" }
                            ),
                            arity,
                            arity,
                            params,
                            Some(generic.clone()),
                        ));
                    }
                    continue;
                }
                fallback_function_overload_rewrites.push((
                    name.to_string(),
                    symbol,
                    function.required_params,
                    function.params.len(),
                    dts_function_param_hir_types(function),
                    function.generic.clone(),
                ));
            }
        }
        let package_overloads = fallback_function_overload_rewrites
            .drain(overload_rewrite_start..)
            .collect::<Vec<_>>();
        for candidate in package_overloads {
            if !union_dispatched_names.contains(&(pkg.name.clone(), candidate.0.clone())) {
                fallback_function_overload_rewrites.push(candidate.clone());
            }
            if let Some(qualified) = qualified
                .iter()
                .find(|qualified| qualified.name == candidate.0)
            {
                let mut candidate = candidate;
                candidate.0 = qualified.alias.clone();
                fallback_function_overload_rewrites.push(candidate);
            }
        }
        // Added separately from `package_overloads` above (which
        // explicitly excludes a union-dispatched name from ever
        // reaching the bare-name rewrite list): these already reuse
        // `union_overload_dispatch_declaration`'s own symbols, so
        // there's no duplicate-declaration risk here to guard against.
        for candidate in union_dispatch_overload_rewrites.drain(..) {
            fallback_function_overload_rewrites.push(candidate.clone());
            if let Some(qualified) = qualified
                .iter()
                .find(|qualified| qualified.name == candidate.0)
            {
                let mut aliased = candidate;
                aliased.0 = qualified.alias.clone();
                fallback_function_overload_rewrites.push(aliased);
            }
        }
        if pkg.native_addon.is_some() && pkg.bundle_js.is_none() {
            shim.push_str(&thaw_bridge::generate_native_addon_shim(
                &pkg.functions,
                qualified,
                &bare_aliased,
            ));
        } else {
            shim.push_str(&thaw_bridge::generate_shim(
                &pkg.functions,
                native_lib_available,
                qualified,
                &bare_aliased,
            ));
        }

        if let Some(native_lib) = &pkg.native_lib {
            native_libs.push(native_lib.clone());
        }
        if let Some(native_addon) = &pkg.native_addon {
            let root_export = if pkg.bundle_js.is_some() {
                None
            } else {
                pkg.commonjs_export_name
                    .clone()
                    .or_else(|| (pkg.functions.len() == 1).then(|| pkg.functions[0].name.clone()))
            };
            if embed_native_addons {
                let bytes = std::fs::read(native_addon).map_err(|error| {
                    format!(
                        "failed to embed native addon `{}`: {error}",
                        native_addon.display()
                    )
                })?;
                native_addons.push((
                    pkg.name.clone(),
                    bytes,
                    pkg.native_dependencies
                        .iter()
                        .map(std::fs::read)
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|error| format!("failed to embed native dependency: {error}"))?,
                    root_export,
                ));
            } else {
                let executable_name = output
                    .file_name()
                    .and_then(|name| name.to_str())
                    .ok_or("output path has no valid file name")?;
                let package_dir = format!(
                    "{}.native/{}",
                    executable_name,
                    sanitize_identifier(&pkg.name)
                );
                let destination = external_native_staging
                    .ok_or("external native staging directory is missing")?
                    .join(sanitize_identifier(&pkg.name));
                std::fs::create_dir_all(&destination).map_err(|error| {
                    format!("failed to create `{}`: {error}", destination.display())
                })?;
                std::fs::copy(native_addon, destination.join("native.node")).map_err(|error| {
                    format!("failed to copy native addon `{}`: {error}", native_addon.display())
                })?;
                let mut dependencies = Vec::new();
                for dependency in &pkg.native_dependencies {
                    let name = dependency
                        .file_name()
                        .ok_or("native dependency path has no file name")?;
                    std::fs::copy(dependency, destination.join(name)).map_err(|error| {
                        format!(
                            "failed to copy native dependency `{}`: {error}",
                            dependency.display()
                        )
                    })?;
                    dependencies.push(format!(
                        "@executable/{package_dir}/{}",
                        name.to_string_lossy()
                    ));
                }
                native_addon_paths.push((
                    pkg.name.clone(),
                    format!("@executable/{package_dir}/native.node"),
                    dependencies,
                    root_export,
                ));
            }
        }
        if let Some(bundle_js) = &pkg.bundle_js {
            // `class_targets` (populated by `generate_napi_class_
            // constructors`, above) exists to drive `class_rewrites` --
            // rewriting a `new ClassName(...)` call site's callee
            // directly -- and nothing else; a bare, non-`new` use of
            // the class name (a static method call's own callee,
            // reading/writing a static property, passing the class
            // value itself around) needs a real *value* binding
            // instead, which `class_targets`'s own symbol can't serve
            // as (it names a constructor-invoking `declare function`,
            // not a value). Two real bugs found getting exactly this
            // shape (luxon's `Settings.defaultLocale = "en-US"`, a
            // plain static property *assignment*) to actually build
            // and run correctly:
            // - A class with no public constructor (`private
            //   constructor(...)`, real example: luxon's `DateTime`/
            //   `Duration`/`Interval`) never gets a `class_targets`
            //   entry at all (`generate_napi_class_constructors` bails
            //   out on `!class.constructible`, correctly -- `new
            //   DateTime()` really shouldn't compile), so its bare name
            //   fell back to the type-only `JsValue` alias every
            //   exported class also gets -- fine as a *type*, not a
            //   *value*: "unknown variable `__thaw_type_luxon_
            //   DateTime`".
            // - A class *with* a public constructor but never actually
            //   `new`-called anywhere (luxon's own `Settings`, a purely
            //   static utility class with no explicit constructor at
            //   all -- `constructible` defaults to `true`, so `class_
            //   targets` gets a real, if pointless, zero-arg-
            //   constructor-invoking entry) had its bare name resolve
            //   to *that* symbol instead -- a `declare function`
            //   reference used as a plain property-assignment target
            //   isn't a value at all in thaw's type system: "function
            //   value `...$new$Settings$arity0` needs a monomorphic
            //   native implementation".
            // Both fixed the same way: bound via the same `$value$`-
            // keyed runtime getter mechanism a `Str`/`F64`/`JsValue`
            // constant export already uses, reading the *real* class
            // value straight off the bundle's own live JS exports --
            // and (below) preferred over `class_targets`'s own symbol
            // for `package_exports`'s bare-identifier mapping
            // specifically, leaving `class_rewrites`'s own `new
            // ClassName(...)` handling completely untouched (it doesn't
            // consult `package_exports` at all).
            //
            // Scoped to a class the user's own source actually
            // references as a bare member-expression object anywhere
            // (`observed_bare_member_object_identifiers` -- a plain AST
            // walk over every `Expr::Member`, so a method call's own
            // callee, a property read, and an assignment target are all
            // covered uniformly) and deduplicated by name. Both matter:
            // unscoped, this bound a purely type-level helper class
            // with no real runtime counterpart at all to a value that
            // reads back `undefined` (real example: socket.io's own
            // `StrictEventEmitter`, never referenced directly by name
            // in real user code); undeduplicated, a class name appearing
            // more than once in `pkg.classes` (declaration merging, or
            // the same class re-exported under its own name from more
            // than one `.d.ts` file thaw-registry's flattening
            // concatenates -- real examples: yaml's `NodeBase`,
            // socket.io's `StrictEventEmitter` again) emitted the exact
            // same `declare function`/`let` pair twice, which the
            // shim's own duplicate-binding check (rightly) rejects as a
            // hard error.
            let mut bare_value_class_names: std::collections::HashSet<String> =
                pkg.values.iter().map(|value| value.name.clone()).collect();
            let bare_value_classes = pkg
                .classes
                .iter()
                .filter(|class| {
                    observed_bare_member_objects.contains(&class.name)
                        && bare_value_class_names.insert(class.name.clone())
                })
                .map(|class| thaw_bridge::DtsValue {
                    name: class.name.clone(),
                    ty: thaw_bridge::DtsType::Native(thaw_hir::HirType::JsValue),
                })
                .collect::<Vec<_>>();
            let value_exports = pkg
                .values
                .iter()
                .chain(bare_value_classes.iter())
                .filter_map(|value| {
                    let dynamic;
                    let ty = match &value.ty {
                        thaw_bridge::DtsType::Native(ty) => ty,
                        thaw_bridge::DtsType::Unsupported(_) => {
                            dynamic = thaw_hir::HirType::JsValue;
                            &dynamic
                        }
                    };
                    if !matches!(ty, thaw_hir::HirType::Str | thaw_hir::HirType::F64 | thaw_hir::HirType::Bool | thaw_hir::HirType::Json | thaw_hir::HirType::JsValue) {
                        return None;
                    }
                    let rendered = render_dynamic_type(ty)?;
                    let local = format!(
                        "__thaw_value_{}_{}",
                        sanitize_identifier(&pkg.name),
                        value.name
                    );
                    let runtime_getter = format!("{}::$value${}", pkg.name, value.name);
                    let encoded = runtime_getter
                        .as_bytes()
                        .iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect::<String>();
                    let typed_getter = format!("__thaw_typed_js_{encoded}");
                    shim.push_str(&format!(
                        "declare function {typed_getter}(): {rendered};\nlet {local}: {rendered} = {typed_getter}();\n"
                    ));
                    value_targets.insert((pkg.name.clone(), value.name.clone()), local.clone());
                    Some((
                        value.name.clone(),
                        runtime_getter,
                        local,
                        typed_getter,
                    ))
                })
                .collect::<Vec<_>>();
            // Only Fallback functions need binding inside the loaded
            // script (see `ModuleBundle::fallback_names`'s doc comment);
            // FastPath functions are real FFI calls and never touch
            // QuickJS-NG at all.
            let fallback_names: Vec<String> = pkg
                .classifications
                .iter()
                .filter_map(|(name, classification)| match classification {
                    thaw_bridge::Classification::Fallback { .. }
                        if !jit_targets.contains(&(pkg.name.clone(), name.clone())) =>
                    {
                        Some(name.clone())
                    }
                    thaw_bridge::Classification::FastPath(_) => None,
                    thaw_bridge::Classification::Fallback { .. } => None,
                })
                .collect();
            let qualified_aliases: Vec<(String, String)> = qualified
                .iter()
                .map(|q| (q.name.clone(), q.qualified_key.clone()))
                .collect();
            // A nested-namespace member (`z.coerce.number`, flattened to
            // its own synthetic top-level name `__thaw_ns_coerce_number`
            // by thaw-registry -- see `thaw_bridge::
            // nested_namespace_members`'s own doc comment) is never
            // actually a real property of `module.exports` under that
            // synthetic name; it only exists at its real, qualified
            // runtime path (`module.exports.coerce.number`). Kept as its
            // own separate list, *not* folded into `qualified_aliases`
            // above -- see `thaw_bridge::ModuleBundle::
            // nested_namespace_aliases`'s own doc comment for why a
            // dotted path needs a different capture mechanism (inside
            // the bundle's own wrapped script) than a plain collision
            // alias (a separate, later `loadScript`) does.
            let nested_namespace_aliases: Vec<(String, String)> = pkg
                .nested_namespaces
                .iter()
                .flat_map(|(namespace, members)| {
                    members.iter().map(move |(member, target)| {
                        (format!("{namespace}.{member}"), target.clone())
                    })
                })
                .collect();
            // A package whose *only* export is a class (real example:
            // hono, whose main export is the `Hono` class with no
            // top-level functions at all) previously never got its
            // bundle loaded at all: `fallback_names` only tracks
            // top-level Fallback functions, never `pkg.classes`, so a
            // class-only package's `!fallback_names.is_empty()` check
            // was always false and its `loadScript` never ran --
            // silently leaving every export (the class included)
            // missing from `globalThis`, since that's set up by the
            // generic `for (var k in module.exports)` copy loop this
            // same `loadScript` call also carries (`generation.rs`'s
            // `wrap_as_commonjs_module`), which never got a chance to
            // run either.
            // Every class this package declares (constructible or not --
            // a static-only class's methods are called as
            // `ClassName.method(...)`, which resolves `ClassName` the
            // same `thaw_js_get_global` way a constructor does). See
            // `ModuleBundle::class_names`'s own doc comment for why a
            // *default*-exported class specifically needs this (a named
            // one already gets bound by the generic `module.exports` ->
            // `globalThis` copy loop below).
            let class_names: Vec<String> =
                pkg.classes.iter().map(|class| class.name.clone()).collect();
            if !fallback_names.is_empty()
                || pkg.native_addon.is_some()
                || !pkg.classes.is_empty()
                || !value_exports.is_empty()
            {
                bundles.push((
                    pkg.name.clone(),
                    bundle_js.clone(),
                    fallback_names,
                    class_names,
                    qualified_aliases,
                    nested_namespace_aliases,
                    value_exports,
                ));
            }
        }
    }

    let module_bundles: Vec<thaw_bridge::ModuleBundle> = bundles
        .iter()
        .map(
            |(name, js, fallback_names, class_names, qualified_aliases, nested_namespace_aliases, value_exports)| {
                thaw_bridge::ModuleBundle {
                    package_name: name.as_str(),
                    js_source: js.as_str(),
                    fallback_names,
                    class_names,
                    qualified_aliases,
                    nested_namespace_aliases,
                    value_exports,
                }
            },
        )
        .collect();
    shim.push_str(&thaw_bridge::generate_module_init(&module_bundles));
    let native_addons: Vec<thaw_bridge::NativeAddon<'_>> = native_addons
        .iter()
        .map(|(name, bytes, dependencies, root_export)| thaw_bridge::NativeAddon {
            package_name: name,
            bytes,
            dependencies: dependencies.iter().map(Vec::as_slice).collect(),
            root_export: root_export.as_deref(),
        })
        .collect();
    shim.push_str(&thaw_bridge::generate_native_addon_init(&native_addons));
    let native_addon_paths: Vec<thaw_bridge::NativeAddonPath<'_>> = native_addon_paths
        .iter()
        .map(|(name, path, dependencies, root_export)| thaw_bridge::NativeAddonPath {
            package_name: name,
            path,
            dependencies: dependencies.iter().map(String::as_str).collect(),
            root_export: root_export.as_deref(),
        })
        .collect();
    shim.push_str(&thaw_bridge::generate_native_addon_path_init(
        &native_addon_paths,
    ));

    let mut external_exports = ExternalExports::new();
    let mut external_namespace_aliases: ExternalNamespaceAliases = std::collections::HashMap::new();
    let mut external_nested_namespaces: ExternalNestedNamespaces = std::collections::HashMap::new();
    for pkg in &resolved {
        if !pkg.namespace_self_aliases.is_empty() {
            external_namespace_aliases
                .insert(pkg.name.clone(), pkg.namespace_self_aliases.clone());
        }
        if !pkg.nested_namespaces.is_empty() {
            external_nested_namespaces.insert(pkg.name.clone(), pkg.nested_namespaces.clone());
        }
        let mut package_exports = std::collections::HashMap::new();
        for name in &pkg.type_only_exports {
            // ponytail: external type-only imports are erased to JsValue;
            // preserve their structure only when native layout is required.
            let target = format!("__thaw_type_{}_{}", sanitize_identifier(&pkg.name), name);
            shim.push_str(&format!("type {target} = JsValue;\n"));
            package_exports.insert(name.clone(), target);
        }
        for (name, classification) in &pkg.classifications {
            let target = if matches!(classification, thaw_bridge::Classification::Fallback { .. }) {
                typed_targets
                    .get(&(pkg.name.clone(), name.clone()))
                    .cloned()
                    .unwrap_or_else(|| format!("{}_{name}", sanitize_identifier(&pkg.name)))
            } else {
                name.clone()
            };
            package_exports.insert(name.clone(), target.clone());
        }
        for class in &pkg.classes {
            // A value binding (see `bare_value_classes` above) always
            // wins over `class_targets`'s constructor-invocation symbol
            // for this bare-identifier mapping specifically: the two
            // are never both meant for the same use (a `new ClassName
            // (...)` call site never consults `package_exports` at all
            // -- that's `class_rewrites`'s own, separate job, untouched
            // here), and a class observed as a bare member-object
            // (luxon's `Settings.defaultLocale = ...`) needs the real
            // live value, never the constructor symbol, even when the
            // class also happens to be constructible.
            if let Some(target) = value_targets.get(&(pkg.name.clone(), class.name.clone())) {
                package_exports.insert(class.name.clone(), target.clone());
            } else if let Some(target) =
                class_targets.get(&(pkg.name.clone(), class.name.clone()))
            {
                package_exports.insert(class.name.clone(), target.clone());
            }
        }
        for value in &pkg.values {
            if let Some(target) = value_targets.get(&(pkg.name.clone(), value.name.clone())) {
                package_exports.insert(value.name.clone(), target.clone());
            }
        }
        if let Some(target) = pkg
            .commonjs_export_name
            .as_ref()
            .and_then(|name| package_exports.get(name))
            .cloned()
        {
            package_exports.insert("default".to_string(), target);
        } else if package_exports.len() == 1 {
            let target = package_exports.values().next().unwrap().clone();
            package_exports.insert("default".to_string(), target);
        } else if pkg
            .commonjs_export_name
            .as_deref()
            .is_some_and(|name| !package_exports.contains_key(name))
            && !package_exports.is_empty()
        {
            // `export = X;` where `X` is bound to a whole object/
            // namespace value (real example: lodash's `declare const _:
            // LoDashStatic;`, ~300 methods), not a single function/class
            // whose own flattened name happens to equal `X` -- unlike
            // `qs`-shaped packages where the exported identifier *is*
            // itself one of its own flattened function names.
            // `package_exports` has no key literally named `X` at all
            // (the object identifier is never itself one of its own
            // flattened method names), so neither branch above can
            // produce a "default" *export* entry, and `import Name from
            // "pkg"` (module_graph.rs's `bundle`) fails outright with "no
            // export named `default`" even though every individual
            // method (`import { chunk } from "pkg"`) already works.
            //
            // Real TypeScript's own `esModuleInterop`/synthetic-default-
            // export convention treats `export = X;` as satisfying a
            // default import with X's own full shape -- i.e. the *whole*
            // package export table, exactly like a bare `import * as
            // Name from "pkg"` would. `external_namespace_aliases`
            // (consumed by module_graph.rs's `bundle`) already binds a
            // default import that resolves via this set as a full
            // namespace over `dependency_exports` -- see
            // `self_referential_namespace_aliases`'s identical use for
            // zod's own `z`/`default` -- so marking `"default"` as a
            // namespace alias here, without needing an actual
            // `package_exports["default"]` entry at all, is enough.
            external_namespace_aliases
                .entry(pkg.name.clone())
                .or_default()
                .insert("default".to_string());
        }
        external_exports.insert(pkg.name.clone(), package_exports);
    }
    Ok((
        shim,
        native_libs,
        rewrites,
        class_rewrites,
        class_method_rewrites,
        callback_instance_rewrites,
        static_class_method_rewrites,
        class_getter_rewrites,
        class_setter_rewrites,
        static_class_getter_rewrites,
        static_class_setter_rewrites,
        factory_class_rewrites,
        fallback_function_overload_rewrites,
        external_exports,
        external_namespace_aliases,
        external_nested_namespaces,
        jit_fallback_reasons,
    ))
}
