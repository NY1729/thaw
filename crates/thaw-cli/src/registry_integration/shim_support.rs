/// `(package, name, alias)` -- see `rewrite_qualified_calls`. `package`
/// here is the *qualifier identifier* (`qualifier_identifier`), not
/// necessarily the real package name.
type QualifiedCallRewrite = (String, String, String);
/// `(qualifier, class, [(argument_count, helper, parameter_types)])`.
type ClassConstructorRewrite = (
    String,
    String,
    Vec<(usize, String, Vec<thaw_hir::HirType>)>,
);
/// `(class, method, helper, argument_count, has_callback, parameter_types,
/// return_instance_class)`.
type ClassMethodRewrite = (
    String,
    String,
    String,
    usize,
    bool,
    Vec<thaw_hir::HirType>,
    Option<String>,
);
#[cfg(test)]
type TestClassMethodRewrite = (
    String,
    String,
    String,
    usize,
    bool,
    Vec<thaw_hir::HirType>,
);
type GeneratedClassMethod = (
    String,
    String,
    usize,
    bool,
    Vec<thaw_hir::HirType>,
    Option<String>,
);
enum ClassMethodContext {
    CallbackInstance(String, usize, usize, String),
    LiteralArgument(String, usize, String),
}
/// `(function name, helper symbol, min arity, max arity, parameter
/// types)` -- a registry Fallback function with more than one `.d.ts`
/// overload (real example: uuid's `v4`, a `(options?): string` overload
/// alongside a generic `<TBuf extends Uint8Array = Uint8Array>(options,
/// buf, offset?): TBuf` one) that "first successful overload wins"
/// (below) can't fully expose through a single bare-name alias. Each
/// overload gets its own helper symbol here; `class_methods.rs`'s
/// `rewrite_external_class_methods_with_static` picks between them at
/// each real call site by arity and, on a tie, `overload_type_score`
/// against the call's actual argument types -- the same mechanism
/// already used for external class method/constructor overloads.
type FallbackFunctionOverloadRewrite = (
    String,
    String,
    usize,
    usize,
    Vec<thaw_hir::HirType>,
    Option<thaw_bridge::DtsGenericFunction>,
);
/// `(class, property, helper)` for an instance getter.
type ClassGetterRewrite = (String, String, String);
/// `(class, property, helper, value_type)` for an instance setter.
type ClassSetterRewrite = (String, String, String, thaw_hir::HirType);
/// `(qualifier, class, property, helper)` for a static getter.
type StaticClassGetterRewrite = (String, String, String, String);
/// `(qualifier, class, property, helper, value_type)` for a static setter.
type StaticClassSetterRewrite = (String, String, String, String, thaw_hir::HirType);
/// `(qualifier, class, method, helper, argument_count, has_callback, parameter_types)`.
type StaticClassMethodRewrite = (
    String,
    String,
    String,
    String,
    usize,
    bool,
    Vec<thaw_hir::HirType>,
);

fn supported_class_method_param(ty: &thaw_bridge::DtsType, index: usize, len: usize) -> bool {
    matches!(ty, thaw_bridge::DtsType::Unsupported(reason) if reason == "`any` is not supported")
        || matches!(
        ty,
        thaw_bridge::DtsType::Native(
            thaw_hir::HirType::F64
                | thaw_hir::HirType::Str
                | thaw_hir::HirType::Bool
                | thaw_hir::HirType::Json
        )
    ) || matches!(
        ty,
        thaw_bridge::DtsType::Native(thaw_hir::HirType::Object(fields))
            if fields.iter().all(|(_, field)| supported_json_collection_element(field))
    ) || matches!(
        ty,
        thaw_bridge::DtsType::Native(
            thaw_hir::HirType::Optional(payload)
                | thaw_hir::HirType::Nullable(payload)
                | thaw_hir::HirType::Nullish(payload)
        )
            if supported_json_collection_element(payload)
    ) || matches!(
        ty,
        thaw_bridge::DtsType::Native(thaw_hir::HirType::Array(element))
            if supported_json_collection_element(element)
    ) || matches!(
        ty,
        thaw_bridge::DtsType::Native(thaw_hir::HirType::Tuple(elements))
            if elements.iter().all(|element| render_dynamic_type(element).is_some())
    ) || matches!(
        ty,
        thaw_bridge::DtsType::Native(thaw_hir::HirType::Function(params, ret))
            if index + 1 == len
                && params.len() <= 2
                && params.iter().all(|param| *param == thaw_hir::HirType::Json)
                && matches!(**ret, thaw_hir::HirType::Json | thaw_hir::HirType::Void)
    )
}

fn supported_class_method_return(ty: &thaw_bridge::DtsType) -> bool {
    matches!(
        ty,
        thaw_bridge::DtsType::Native(
            thaw_hir::HirType::F64
                | thaw_hir::HirType::Str
                | thaw_hir::HirType::Bool
                | thaw_hir::HirType::Json
                | thaw_hir::HirType::JsValue
                | thaw_hir::HirType::Void
                | thaw_hir::HirType::Object(_)
        )
    ) || matches!(
        ty,
        thaw_bridge::DtsType::Native(
            thaw_hir::HirType::Optional(payload)
                | thaw_hir::HirType::Nullable(payload)
                | thaw_hir::HirType::Nullish(payload)
        )
            if supported_json_collection_element(payload)
    ) || matches!(
        ty,
        thaw_bridge::DtsType::Native(thaw_hir::HirType::Array(element))
            if supported_json_collection_element(element)
    ) || matches!(
        ty,
        thaw_bridge::DtsType::Native(thaw_hir::HirType::Tuple(elements))
            if elements.iter().all(|element| render_dynamic_type(element).is_some())
    )
}

/// `napi` picks which backend the generated declaration's symbol resolves
/// to at lower time -- `__thaw_typed_napi_`/`DynamicBackend::Napi` for a
/// real native addon class, `__thaw_typed_js_`/`DynamicBackend::QuickJs`
/// for a Fallback one (real example: hono's `Hono`) -- decoded by
/// `dynamic_symbol`, the same convention `generate_napi_class_method_
/// overloads` already uses. The `$new$Class$arityN` runtime-key scheme
/// itself is identical either way; `napi_constructor_export_name`/its
/// QuickJS-NG counterpart both strip the same prefix back off.
fn generate_napi_class_constructors(
    class: &thaw_bridge::DtsClass,
    napi: bool,
    shim: &mut String,
) -> Vec<(usize, String, Vec<thaw_hir::HirType>)> {
    if !class.constructible {
        return Vec::new();
    }
    let prefix = if napi { "napi" } else { "js" };
    let mut helpers = Vec::new();
    for (overload_index, constructor) in class.constructors.iter().enumerate() {
        // Checked per arity, not once for the whole constructor. Types the
        // declaration parser cannot classify still cross the dynamic ABI as
        // Json, matching ordinary Fallback function parameters.
        for arity in constructor.required_params..=constructor.params.len() {
            let params = &constructor.params[..arity];
            if !params.iter().all(|(_, ty)| match ty {
                thaw_bridge::DtsType::Native(native) => render_dynamic_type(native).is_some(),
                thaw_bridge::DtsType::Unsupported(_) => true,
            }) {
                continue;
            }
            let parameter_types = params
                .iter()
                .map(|(_, ty)| match ty {
                    thaw_bridge::DtsType::Native(ty) => ty.clone(),
                    thaw_bridge::DtsType::Unsupported(_) => thaw_hir::HirType::Json,
                })
                .collect::<Vec<_>>();
            if helpers.iter().any(|(existing_arity, _, existing_types)| {
                *existing_arity == arity && existing_types == &parameter_types
            }) {
                continue;
            }
            let rendered = params
                .iter()
                .map(|(name, ty)| match ty {
                    thaw_bridge::DtsType::Native(ty) => {
                        format!("{name}: {}", render_dynamic_type(ty).unwrap())
                    }
                    thaw_bridge::DtsType::Unsupported(_) => format!("{name}: Json"),
                })
                .collect::<Vec<_>>()
                .join(", ");
            let overload = if class.constructors.len() > 1 {
                format!("$overload{overload_index}")
            } else {
                String::new()
            };
            let runtime_key = format!("$new${}$arity{arity}{overload}", class.name);
            let encoded = runtime_key
                .as_bytes()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            let symbol = format!("__thaw_typed_{prefix}_{encoded}");
            shim.push_str(&format!(
                "declare function {symbol}({rendered}): JsValue;\n"
            ));
            helpers.push((arity, symbol, parameter_types));
        }
    }
    if class.constructors.is_empty() {
        let runtime_key = format!("$new${}$arity0", class.name);
        let encoded = runtime_key
            .as_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let symbol = format!("__thaw_typed_{prefix}_{encoded}");
        shim.push_str(&format!(
            "declare function {symbol}(): JsValue;\n"
        ));
        helpers.push((0, symbol, Vec::new()));
    }
    helpers
}

fn generate_napi_class_property_getter(
    class: &str,
    property: &str,
    ty: &thaw_bridge::DtsType,
    is_static: bool,
    shim: &mut String,
) -> Option<String> {
    let thaw_bridge::DtsType::Native(ty) = ty else {
        return None;
    };
    let rendered = render_dynamic_type(ty)?;
    let prefix = if is_static { "staticgetter" } else { "getter" };
    let runtime_key = format!("${prefix}${class}${property}");
    let encoded = runtime_key
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let symbol = format!("__thaw_typed_napi_{encoded}");
    shim.push_str(&format!(
        "declare function {symbol}({}): {rendered};\n",
        if is_static { "" } else { "receiver: JsValue" }
    ));
    Some(symbol)
}

fn generate_napi_class_property_setter(
    class: &str,
    property: &str,
    ty: &thaw_bridge::DtsType,
    is_static: bool,
    napi: bool,
    shim: &mut String,
) -> Option<(String, thaw_hir::HirType)> {
    let thaw_bridge::DtsType::Native(ty) = ty else {
        return None;
    };
    let rendered = render_dynamic_type(ty)?;
    let prefix = if is_static { "staticsetter" } else { "setter" };
    let runtime_key = format!("${prefix}${class}${property}");
    let encoded = runtime_key
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let symbol = format!(
        "__thaw_typed_{}_{}",
        if napi { "napi" } else { "js" },
        encoded
    );
    shim.push_str(&format!(
        "declare function {symbol}({}value: {rendered}): {rendered};\n",
        if is_static { "" } else { "receiver: JsValue, " }
    ));
    Some((symbol, ty.clone()))
}

/// Generates one `declare function __thaw_typed_{napi,js}_<hex>(...)`
/// ambient declaration per callable overload/arity of `class`'s instance
/// or static methods (matching `method.is_static == is_static`), and
/// returns the `(method_name, symbol, argument_count, has_callback,
/// parameter_types)` tuples `class_method_rewrites`/
/// `static_class_method_rewrites` need to rewrite ordinary
/// `instance.method(...)` call syntax into a call to the right one.
///
/// `napi` selects which backend the generated declaration's symbol
/// resolves to at lower time (`__thaw_typed_napi_`/`DynamicBackend::Napi`
/// vs. `__thaw_typed_js_`/`DynamicBackend::QuickJs`, decoded by
/// `dynamic_symbol`) -- the runtime-key scheme itself (`$method$Class
/// $name$overloadN$arityM`, read back by `compile_typed_napi_method`,
/// which despite the name now serves both backends) is identical either
/// way, so this one generator covers a native addon's class (`napi:
/// true`) and a Fallback/QuickJS-NG one (`napi: false`, real example:
/// dayjs's `Dayjs`) alike. A trailing callback parameter is excluded for
/// the QuickJS-NG backend, which doesn't marshal one yet (see
/// `compile_typed_napi_method`'s explicit guard).
#[cfg(test)]
fn generate_napi_class_method_overloads(
    class: &thaw_bridge::DtsClass,
    is_static: bool,
    observed_arities: &std::collections::HashMap<String, std::collections::BTreeSet<usize>>,
    shim: &mut String,
    napi: bool,
) -> Vec<(String, String, usize, bool, Vec<thaw_hir::HirType>)> {
    generate_napi_class_method_overloads_with_callback_instances(
        class,
        is_static,
        observed_arities,
        shim,
        napi,
        &mut Vec::new(),
    )
    .into_iter()
    .map(|(method, symbol, arity, callback, params, _)| {
        (method, symbol, arity, callback, params)
    })
    .collect()
}

fn generate_napi_class_method_overloads_with_callback_instances(
    class: &thaw_bridge::DtsClass,
    is_static: bool,
    observed_arities: &std::collections::HashMap<String, std::collections::BTreeSet<usize>>,
    shim: &mut String,
    napi: bool,
    method_contexts: &mut Vec<ClassMethodContext>,
) -> Vec<GeneratedClassMethod> {
    let mut generated = Vec::new();
    let mut method_names = std::collections::HashSet::new();
    for method in &class.methods {
        if method.is_static != is_static
            || method.kind != thaw_bridge::DtsMethodKind::Method
            || !method_names.insert(method.name.clone())
        {
            continue;
        }
        let overloads = class
            .methods
            .iter()
            .filter(|candidate| {
                candidate.name == method.name
                    && candidate.is_static == is_static
                    && candidate.kind == thaw_bridge::DtsMethodKind::Method
            })
            .filter(|candidate| {
                !napi || supported_class_method_return(&candidate.ret)
            })
            .collect::<Vec<_>>();
        for (overload_index, overload) in overloads.into_iter().enumerate() {
            let return_type = if overload.return_instance_class.is_some() {
                Some("JsValue".to_string())
            } else {
                match &overload.ret {
                thaw_bridge::DtsType::Native(return_type) => {
                    if *return_type == thaw_hir::HirType::Void {
                        Some("Json".to_string())
                    } else {
                        render_dynamic_type(return_type)
                    }
                }
                thaw_bridge::DtsType::Unsupported(_) if !napi => Some("JsValue".to_string()),
                thaw_bridge::DtsType::Unsupported(_) => None,
                }
            };
            let Some(return_type) = return_type else {
                continue;
            };
            let argument_counts = if overload.rest_param.is_some() {
                observed_arities
                    .get(&method.name)
                    .into_iter()
                    .flatten()
                    .copied()
                    .filter(|count| *count >= overload.required_params)
                    .collect::<Vec<_>>()
            } else {
                (overload.required_params..=overload.params.len()).collect()
            };
            for argument_count in argument_counts {
                let fixed_count = argument_count.min(overload.params.len());
                if napi
                    && (!overload.params[..fixed_count]
                        .iter()
                        .enumerate()
                        .all(|(index, (_, ty))| {
                            supported_class_method_param(ty, index, fixed_count)
                        })
                        || (argument_count > overload.params.len()
                            && overload.rest_param.as_ref().is_none_or(|(_, ty)| {
                                !supported_class_method_param(ty, 0, 1)
                            })))
                {
                    continue;
                }
                let mut included_params = overload.params[..fixed_count]
                    .iter()
                    .filter_map(|(name, ty)| match ty {
                        thaw_bridge::DtsType::Native(ty) => Some((name.clone(), ty.clone())),
                        thaw_bridge::DtsType::Unsupported(reason)
                            if !napi || reason == "`any` is not supported" =>
                        {
                            Some((name.clone(), thaw_hir::HirType::Json))
                        }
                        thaw_bridge::DtsType::Unsupported(_) => None,
                    })
                    .collect::<Vec<_>>();
                if argument_count > overload.params.len() {
                    let Some((name, rest_type)) = &overload.rest_param else {
                        continue;
                    };
                    let rest_type = match rest_type {
                        thaw_bridge::DtsType::Native(ty) => ty.clone(),
                        thaw_bridge::DtsType::Unsupported(reason)
                            if !napi || reason == "`any` is not supported" =>
                        {
                            thaw_hir::HirType::Json
                        }
                        thaw_bridge::DtsType::Unsupported(_) => continue,
                    };
                    included_params.extend(
                        (overload.params.len()..argument_count)
                            .map(|index| (format!("{name}{index}"), rest_type.clone())),
                    );
                }
                let params = (if is_static {
                    Vec::new()
                } else {
                    vec!["receiver: JsValue".to_string()]
                })
                .into_iter()
                .chain(included_params.iter().map(|(name, ty)| {
                    format!(
                        "{name}: {}",
                        render_dynamic_type(ty).expect("filtered above")
                    )
                }))
                .collect::<Vec<_>>()
                .join(", ");
                let has_callback = matches!(
                    included_params.last(),
                    Some((_, thaw_hir::HirType::Function(_, _) | thaw_hir::HirType::CallableFunction(..)))
                );
                let runtime_key = format!(
                    "{}{}${}$overload{overload_index}$arity{argument_count}",
                    match (is_static, &overload.ret) {
                        (true, thaw_bridge::DtsType::Native(thaw_hir::HirType::Void)) => {
                            "$staticmethodvoid$"
                        }
                        (true, _) => "$staticmethod$",
                        (false, thaw_bridge::DtsType::Native(thaw_hir::HirType::Void)) => {
                            "$methodvoid$"
                        }
                        (false, _) => "$method$",
                    },
                    class.name,
                    method.name
                );
                let encoded = runtime_key
                    .as_bytes()
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>();
                let symbol = format!(
                    "__thaw_typed_{}_{encoded}",
                    if napi { "napi" } else { "js" }
                );
                shim.push_str(&format!(
                    "declare function {symbol}({params}): {return_type};\n"
                ));
                for (argument_index, classes) in overload
                    .callback_instance_classes
                    .iter()
                    .take(fixed_count)
                    .enumerate()
                {
                    for (parameter_index, class) in classes.iter().enumerate() {
                        if let Some(class) = class {
                            method_contexts.push(ClassMethodContext::CallbackInstance(
                                symbol.clone(),
                                argument_index,
                                parameter_index,
                                class.clone(),
                            ));
                        }
                    }
                }
                for (argument_index, literal) in overload
                    .literal_params
                    .iter()
                    .take(fixed_count)
                    .enumerate()
                {
                    if let Some(literal) = literal {
                        method_contexts.push(ClassMethodContext::LiteralArgument(
                            symbol.clone(),
                            argument_index,
                            literal.clone(),
                        ));
                    }
                }
                generated.push((
                    method.name.clone(),
                    symbol,
                    argument_count,
                    has_callback,
                    included_params.into_iter().map(|(_, ty)| ty).collect(),
                    overload.return_instance_class.clone(),
                ));
            }
        }
    }
    generated
}
type ExternalExports = std::collections::HashMap<String, std::collections::HashMap<String, String>>;
/// `package name -> names that package's own `.d.ts` re-exports as a
/// self-referential namespace alias for its own export table` -- see
/// `thaw_bridge::self_referential_namespace_aliases`'s own doc comment.
/// Only ever populated for a package that actually has one (most
/// packages don't, so this stays empty for them).
type ExternalNamespaceAliases = std::collections::HashMap<String, std::collections::HashSet<String>>;
/// `package name -> { namespace name -> { member name -> real flattened
/// function name } }` -- see `thaw_bridge::nested_namespace_members`'s
/// own doc comment. Only ever populated for a package that actually has
/// one (most packages don't, so this stays empty for them).
type ExternalNestedNamespaces = std::collections::HashMap<
    String,
    std::collections::HashMap<String, std::collections::HashMap<String, String>>,
>;
type RegistryShims = (
    String,
    Vec<PathBuf>,
    Vec<QualifiedCallRewrite>,
    Vec<ClassConstructorRewrite>,
    Vec<ClassMethodRewrite>,
    Vec<ClassMethodContext>,
    Vec<StaticClassMethodRewrite>,
    Vec<ClassGetterRewrite>,
    Vec<ClassSetterRewrite>,
    Vec<StaticClassGetterRewrite>,
    Vec<StaticClassSetterRewrite>,
    Vec<FactoryClassRewrite>,
    Vec<FallbackFunctionOverloadRewrite>,
    ExternalExports,
    ExternalNamespaceAliases,
    ExternalNestedNamespaces,
    JitFallbackReasons,
);
type JitFallbackReasons = std::collections::HashMap<(String, String), String>;

/// `(factory function name, class name)` -- a Fallback factory function
/// call (real example: dayjs's `dayjs(...)`) that should be tracked as
/// producing an instance of that class, the same way `new ClassName(...)`
/// already is (see `class_methods.rs`'s `constructed_class`), even
/// though the call itself doesn't say `new`. No qualifier: matched by
/// bare callee name only, same simplification `ClassConstructorRewrite`
/// already makes for a bare `new ClassName(...)`.
type FactoryClassRewrite = (String, String);

/// The identifier a user writes as the object in `pkg.name(...)`
/// qualified-call syntax for a `--use`d package. A scoped package's real
/// name (`@hapi/hoek`) isn't a valid identifier at all (`@`, `/`), so
/// this uses its last path segment (`hoek`) instead -- an unscoped name
/// has no `/` to split on and passes through unchanged. Two different
fn qualifier_identifier(package: &str) -> &str {
    package.rsplit('/').next().unwrap_or(package)
}

fn package_qualifier_identifiers<'a>(
    packages: impl IntoIterator<Item = &'a str>,
) -> std::collections::HashMap<String, String> {
    let packages = packages
        .into_iter()
        .map(str::to_string)
        .collect::<std::collections::BTreeSet<_>>();
    let mut counts = std::collections::HashMap::new();
    for package in &packages {
        *counts
            .entry(qualifier_identifier(package).to_string())
            .or_insert(0usize) += 1;
    }
    let candidates = packages
        .into_iter()
        .map(|package| {
            let base = qualifier_identifier(&package);
            let qualifier = if counts[base] == 1 {
                base.to_string()
            } else {
                sanitize_identifier(&package)
            };
            (package, qualifier)
        })
        .collect::<Vec<_>>();
    let mut candidate_counts = std::collections::HashMap::new();
    for (_, candidate) in &candidates {
        *candidate_counts.entry(candidate.clone()).or_insert(0usize) += 1;
    }
    candidates
        .into_iter()
        .map(|(package, candidate)| {
            if candidate_counts[candidate.as_str()] == 1 {
                (package, candidate)
            } else {
                let encoded = package
                    .as_bytes()
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>();
                (package, format!("{candidate}__{encoded}"))
            }
        })
        .collect()
}

fn is_native_builtin(package: &str) -> bool {
    matches!(package, "node:fs" | "node:http")
}

fn observed_member_call_arities(
    source: &str,
) -> Result<std::collections::HashMap<String, std::collections::BTreeSet<usize>>, String> {
    use swc_ecma_visit::{Visit, VisitWith};
    use thaw_parser::ast::{CallExpr, Callee, Expr, MemberProp};

    #[derive(Default)]
    struct Finder {
        arities: std::collections::HashMap<String, std::collections::BTreeSet<usize>>,
    }

    impl Visit for Finder {
        fn visit_call_expr(&mut self, call: &CallExpr) {
            if let Callee::Expr(callee) = &call.callee {
                if let Expr::Member(member) = callee.as_ref() {
                    if let MemberProp::Ident(method) = &member.prop {
                        self.arities
                            .entry(method.sym.to_string())
                            .or_default()
                            .insert(call.args.len());
                    }
                }
            }
            call.visit_children_with(self);
        }
    }

    let module = thaw_parser::parse_typescript(source)?;
    let mut finder = Finder::default();
    module.visit_with(&mut finder);
    Ok(finder.arities)
}

/// Like `observed_member_call_arities`, but for a plain call through a bare
/// identifier (`clsx(...)`) rather than `obj.method(...)` -- the shape a
/// package's own top-level Fallback function is actually called through
/// once imported. Keyed by that bare name only, same limitation as the
/// member version: two same-named imports from different packages share
/// one observed-arity set.
fn observed_identifier_call_arities(
    source: &str,
) -> Result<std::collections::HashMap<String, std::collections::BTreeSet<usize>>, String> {
    use swc_ecma_visit::{Visit, VisitWith};
    use thaw_parser::ast::{CallExpr, Callee, Expr};

    #[derive(Default)]
    struct Finder {
        arities: std::collections::HashMap<String, std::collections::BTreeSet<usize>>,
    }

    impl Visit for Finder {
        fn visit_call_expr(&mut self, call: &CallExpr) {
            if let Callee::Expr(callee) = &call.callee {
                if let Expr::Ident(name) = callee.as_ref() {
                    self.arities
                        .entry(name.sym.to_string())
                        .or_default()
                        .insert(call.args.len());
                }
            }
            call.visit_children_with(self);
        }
    }

    let module = thaw_parser::parse_typescript(source)?;
    let mut finder = Finder::default();
    module.visit_with(&mut finder);
    Ok(finder.arities)
}

/// The name a package's own bundled `.d.ts` binds as its "default"
/// export, when there's a single unambiguous one -- `export = x;`
/// (CommonJS-style, the original shape this covered), but also either
/// ESM shape for a *named* default export: `export default x;` (an
/// existing identifier, resolved to that name) and `export default
/// function name(...): T;` (the function's own name, exactly what
/// `parse_dts`'s `extract_fn_decls` already registers it under -- see
/// its own doc comment). Without this, a package exporting more than one
/// function this way (real example: `leven`'s default `leven` alongside
/// its named `closestMatch`) had no way to resolve which one `import
/// leven from "leven"` actually means: the only other source for
/// `external_exports`'s `"default"` key is a package with *exactly one*
/// function total, a fallback this bypasses entirely once it names one.
fn commonjs_export_name(source: &str) -> Result<Option<String>, String> {
    use thaw_parser::ast::{DefaultDecl, Expr, ModuleDecl, ModuleItem};

    let module = thaw_parser::parse_typescript(source)?;
    Ok(module.body.iter().find_map(|item| match item {
        ModuleItem::ModuleDecl(ModuleDecl::TsExportAssignment(export)) => {
            if let Expr::Ident(identifier) = export.expr.as_ref() {
                Some(identifier.sym.to_string())
            } else {
                None
            }
        }
        ModuleItem::ModuleDecl(ModuleDecl::ExportDefaultExpr(export)) => {
            if let Expr::Ident(identifier) = export.expr.as_ref() {
                Some(identifier.sym.to_string())
            } else {
                None
            }
        }
        ModuleItem::ModuleDecl(ModuleDecl::ExportDefaultDecl(export)) => match &export.decl {
            DefaultDecl::Fn(fn_expr) => fn_expr.ident.as_ref().map(|ident| ident.sym.to_string()),
            _ => None,
        },
        _ => None,
    }))
}

/// Function-valued properties merged onto a callable CommonJS export.
/// DefinitelyTyped commonly spells these as `namespace e { var json:
/// typeof bodyParser.json; }` (rather than a namespace `function`), so
/// `parse_dts` cannot recover their imported signature from this file.
/// Only return properties the program actually calls; the dynamic host
/// already handles their values and argument packing.
fn called_commonjs_namespace_properties(
    source: &str,
    namespace: Option<&str>,
    observed: &std::collections::HashMap<String, std::collections::BTreeSet<usize>>,
) -> Result<Vec<String>, String> {
    use thaw_parser::ast::{Decl, ModuleDecl, ModuleItem, Pat, Stmt, TsModuleName, TsNamespaceBody, TsType};

    let Some(namespace) = namespace else {
        return Ok(Vec::new());
    };
    let module = thaw_parser::parse_typescript(source)?;
    let mut names = Vec::new();
    for item in &module.body {
        let ModuleItem::Stmt(Stmt::Decl(Decl::TsModule(module_decl))) = item else {
            continue;
        };
        let TsModuleName::Ident(module_name) = &module_decl.id else {
            continue;
        };
        if module_name.sym.as_str() != namespace {
            continue;
        }
        let Some(TsNamespaceBody::TsModuleBlock(block)) = &module_decl.body else {
            continue;
        };
        for member in &block.body {
            let declaration = match member {
                ModuleItem::Stmt(Stmt::Decl(Decl::Var(declaration))) => Some(declaration),
                ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => {
                    if let Decl::Var(declaration) = &export.decl {
                        Some(declaration)
                    } else {
                        None
                    }
                }
                _ => None,
            };
            let Some(declaration) = declaration else {
                continue;
            };
            for declarator in &declaration.decls {
                let Pat::Ident(binding) = &declarator.name else {
                    continue;
                };
                let name = binding.id.sym.to_string();
                if observed.contains_key(&name)
                    && matches!(
                        binding.type_ann.as_ref().map(|annotation| annotation.type_ann.as_ref()),
                        Some(TsType::TsTypeQuery(_))
                    )
                {
                    names.push(name);
                }
            }
        }
    }
    names.sort();
    names.dedup();
    Ok(names)
}
