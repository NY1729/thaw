/// Parses a `.d.ts` source string and extracts every top-level function
/// signature (`declare function foo(...): T;` and
/// `export declare function foo(...): T;` -- `.d.ts` files don't have
/// function bodies to begin with, so plain `export function foo(...): T;`
/// is equally ambient here). `interface` declarations are resolved first
/// (see `resolve_interfaces`) so a signature using one classifies as
/// `Native` just like an inline `{ ... }` type literal would. Anything else
/// at the top level (classes, `const`, re-exports, ...) is silently
/// skipped: this is a function-signature extractor, not a full `.d.ts`
/// model.
pub fn parse_dts(source: &str) -> Result<Vec<DtsFunction>, String> {
    let module = thaw_parser::parse_typescript(source)?;
    let (interfaces, generic_interfaces) = resolve_interfaces(&module);
    Ok(module
        .body
        .iter()
        .flat_map(extract_fn_decls)
        .map(|(name, func)| lower_dts_function(name, func, &interfaces, &generic_interfaces))
        .collect())
}

pub fn parse_dts_classes(source: &str) -> Result<Vec<DtsClass>, String> {
    let module = thaw_parser::parse_typescript(source)?;
    let (interfaces, generic_interfaces) = resolve_interfaces(&module);
    let mut classes = module
        .body
        .iter()
        .filter_map(extract_class_decl)
        .map(|(name, class)| lower_dts_class(name, class, &interfaces, &generic_interfaces))
        .collect::<Vec<_>>();
    for class in &mut classes {
        let constructor_overloaded = class.constructors.len() > 1;
        for constructor in &mut class.constructors {
            constructor.overloaded = constructor_overloaded;
        }
        let mut counts = HashMap::<(String, bool), usize>::new();
        for method in &class.methods {
            *counts
                .entry((method.name.clone(), method.is_static))
                .or_default() += 1;
        }
        for method in &mut class.methods {
            method.overloaded = counts[&(method.name.clone(), method.is_static)] > 1;
        }
    }
    Ok(classes)
}

fn extract_class_decl(item: &ModuleItem) -> Option<(&str, &Class)> {
    match item {
        ModuleItem::Stmt(swc_ecma_ast::Stmt::Decl(Decl::Class(class))) => {
            Some((class.ident.sym.as_str(), &class.class))
        }
        ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => match &export.decl {
            Decl::Class(class) => Some((class.ident.sym.as_str(), &class.class)),
            _ => None,
        },
        ModuleItem::ModuleDecl(ModuleDecl::ExportDefaultDecl(export)) => match &export.decl {
            DefaultDecl::Class(class) => class
                .ident
                .as_ref()
                .map(|ident| (ident.sym.as_str(), class.class.as_ref())),
            _ => None,
        },
        _ => None,
    }
}

fn property_name(key: &PropName) -> Option<String> {
    match key {
        PropName::Ident(name) => Some(name.sym.to_string()),
        PropName::Str(name) => Some(name.value.to_string_lossy().into_owned()),
        _ => None,
    }
}

fn lower_class_params(
    params: &[ParamOrTsParamProp],
    interfaces: &HashMap<String, DtsType>,
    generic_interfaces: &GenericInterfaces,
) -> Vec<(String, DtsType)> {
    params
        .iter()
        .enumerate()
        .map(|(index, param)| {
            let binding = match param {
                ParamOrTsParamProp::Param(param) => match &param.pat {
                    Pat::Ident(binding) => Some(binding),
                    _ => None,
                },
                ParamOrTsParamProp::TsParamProp(property) => match &property.param {
                    TsParamPropParam::Ident(binding) => Some(binding),
                    TsParamPropParam::Assign(_) => None,
                },
            };
            let Some(binding) = binding else {
                return (
                    format!("arg{index}"),
                    DtsType::Unsupported("unsupported constructor parameter pattern".into()),
                );
            };
            let ty = binding
                .type_ann
                .as_ref()
                .map(|annotation| {
                    classify_ts_type(&annotation.type_ann, interfaces, generic_interfaces)
                })
                .unwrap_or_else(|| DtsType::Unsupported("missing type annotation".into()));
            (binding.id.sym.to_string(), ty)
        })
        .collect()
}

fn lower_dts_class(
    name: &str,
    class: &Class,
    interfaces: &HashMap<String, DtsType>,
    generic_interfaces: &GenericInterfaces,
) -> DtsClass {
    let extends = class
        .super_class
        .as_deref()
        .and_then(|super_class| match super_class {
            Expr::Ident(name) => Some(name.sym.to_string()),
            _ => None,
        });
    let mut constructors = Vec::new();
    let mut methods = Vec::new();
    let mut properties = Vec::new();
    for member in &class.body {
        match member {
            ClassMember::Constructor(constructor) => constructors.push(DtsConstructor {
                params: lower_class_params(&constructor.params, interfaces, generic_interfaces),
                overloaded: false,
            }),
            ClassMember::Method(method) => {
                let Some(method_name) = property_name(&method.key) else {
                    continue;
                };
                let function = lower_dts_function(
                    &method_name,
                    &method.function,
                    interfaces,
                    generic_interfaces,
                );
                let params = function.params;
                let rest_param = function.rest_param;
                methods.push(DtsMethod {
                    name: function.name,
                    params,
                    required_params: method
                        .function
                        .params
                        .iter()
                        .take_while(|param| match &param.pat {
                            Pat::Ident(binding) => !binding.optional,
                            Pat::Assign(_) | Pat::Rest(_) => false,
                            _ => true,
                        })
                        .count(),
                    rest_param,
                    ret: function.ret,
                    is_static: method.is_static,
                    kind: match method.kind {
                        MethodKind::Method => DtsMethodKind::Method,
                        MethodKind::Getter => DtsMethodKind::Getter,
                        MethodKind::Setter => DtsMethodKind::Setter,
                    },
                    overloaded: false,
                });
            }
            ClassMember::ClassProp(property) => {
                let Some(property_name) = property_name(&property.key) else {
                    continue;
                };
                let ty = property
                    .type_ann
                    .as_ref()
                    .map(|annotation| {
                        classify_ts_type(&annotation.type_ann, interfaces, generic_interfaces)
                    })
                    .unwrap_or_else(|| DtsType::Unsupported("missing type annotation".into()));
                properties.push(DtsProperty {
                    name: property_name,
                    ty,
                    is_static: property.is_static,
                    readonly: property.readonly,
                });
            }
            _ => {}
        }
    }
    DtsClass {
        name: name.to_string(),
        extends,
        constructors,
        methods,
        properties,
    }
}

/// A single top-level `declare function`/`export declare function`
/// extracts one `(name, function)` pair; a `declare namespace Foo {
/// function bar(...): ...; }` recurses into its body and extracts every
/// function found inside (at any nesting depth -- a namespace can itself
/// contain a nested namespace). Found necessary by a real npm package
/// (`qs`), whose entire type surface -- including every function --
/// lives inside `declare namespace QueryString { ... }` rather than at
/// the top level; without this, `parse_dts` found zero functions in it.
/// The extracted `DtsFunction.name` is the bare function name (`parse`,
/// not `QueryString.parse`) -- that's also what `wrap_as_commonjs_module`'s
/// object-export hoisting binds it to at runtime (`qs`'s own
/// `module.exports = { parse, stringify, ... }`), so the two already
/// agree without any extra namespace-qualification logic.
///
/// Also handles a *named* `export default function foo(...): T;` (an
/// `ExportDefaultDecl`, a different AST shape than `ExportDecl` --
/// found necessary by real ESM packages, whose `.d.ts` commonly uses
/// this form, e.g. `escape-string-regexp`'s `export default function
/// escapeStringRegexp(string: string): string;`). An *anonymous*
/// `export default function(...): T;` has no name to extract a callable
/// `DtsFunction` under and is silently skipped -- `.d.ts` authors
/// essentially always name it in practice specifically so it's
/// referenceable, so this isn't expected to matter.
fn extract_fn_decls(item: &ModuleItem) -> Vec<(&str, &Function)> {
    match item {
        ModuleItem::Stmt(swc_ecma_ast::Stmt::Decl(decl)) => extract_fn_decls_from_decl(decl),
        ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => {
            extract_fn_decls_from_decl(&export.decl)
        }
        ModuleItem::ModuleDecl(ModuleDecl::ExportDefaultDecl(export)) => match &export.decl {
            DefaultDecl::Fn(fn_expr) => match &fn_expr.ident {
                Some(ident) => vec![(ident.sym.as_str(), &fn_expr.function)],
                None => Vec::new(),
            },
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

fn extract_fn_decls_from_decl(decl: &Decl) -> Vec<(&str, &Function)> {
    match decl {
        Decl::Fn(fn_decl) => vec![(fn_decl.ident.sym.as_str(), &fn_decl.function)],
        Decl::TsModule(module_decl) => {
            let Some(TsNamespaceBody::TsModuleBlock(block)) = &module_decl.body else {
                return Vec::new();
            };
            block.body.iter().flat_map(extract_fn_decls).collect()
        }
        _ => Vec::new(),
    }
}

fn extract_interface_decl(item: &ModuleItem) -> Option<&TsInterfaceDecl> {
    match item {
        ModuleItem::Stmt(swc_ecma_ast::Stmt::Decl(Decl::TsInterface(iface))) => Some(iface),
        ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => match &export.decl {
            Decl::TsInterface(iface) => Some(iface),
            _ => None,
        },
        _ => None,
    }
}

type GenericInterfaces<'a> = HashMap<String, &'a TsInterfaceDecl>;

/// Resolves every top-level *non-generic* `interface` into a `DtsType`
/// (first map), mirroring `thaw_hir::lower::resolve_interfaces` but
/// degrading to `DtsType::Unsupported` (with a reason) instead of erroring
/// on a self-referential/otherwise-unrepresentable interface -- one broken
/// interface should make signatures that use it fall back, not abort
/// classifying the rest of the `.d.ts` file. Generic interfaces are kept
/// raw (second map), resolved on demand via substitution -- see
/// `resolve_generic_interface`, and `thaw_hir::lower::GenericInterfaces`'s
/// doc comment for the scope limits this mirrors (no nested-inside-
/// another-interface use, no `extends` on the generic interface itself).
fn resolve_interfaces(module: &Module) -> (HashMap<String, DtsType>, GenericInterfaces<'_>) {
    let mut raw: HashMap<String, &TsInterfaceDecl> = HashMap::new();
    let mut generic: GenericInterfaces = HashMap::new();
    for iface in module.body.iter().filter_map(extract_interface_decl) {
        let name = iface.id.sym.to_string();
        if iface.type_params.is_some() {
            generic.insert(name, iface);
        } else {
            raw.insert(name, iface);
        }
    }

    let mut resolved = HashMap::new();
    for name in raw.keys().cloned().collect::<Vec<_>>() {
        resolve_interface(&name, &raw, &mut resolved, &mut Vec::new());
    }
    (resolved, generic)
}

fn resolve_interface(
    name: &str,
    raw: &HashMap<String, &TsInterfaceDecl>,
    resolved: &mut HashMap<String, DtsType>,
    in_progress: &mut Vec<String>,
) -> DtsType {
    if let Some(ty) = resolved.get(name) {
        return ty.clone();
    }
    if in_progress.iter().any(|n| n == name) {
        let ty = DtsType::Unsupported(format!(
            "interface `{name}` is (indirectly) self-referential"
        ));
        resolved.insert(name.to_string(), ty.clone());
        return ty;
    }
    let Some(iface) = raw.get(name) else {
        return DtsType::Unsupported(format!("unknown or generic interface `{name}`"));
    };

    if iface
        .body
        .body
        .iter()
        .any(|member| matches!(member, TsTypeElement::TsCallSignatureDecl(_)))
    {
        let ty = DtsType::Native(HirType::JsValue);
        resolved.insert(name.to_string(), ty.clone());
        return ty;
    }

    in_progress.push(name.to_string());

    // `extends`: same rule as thaw-hir's `resolve_interface` -- base
    // fields first (in `extends`-list, then declaration, order), then this
    // interface's own fields; any name collision degrades the whole
    // interface to `Unsupported` rather than guessing an override rule.
    let mut fields: Vec<(String, HirType)> = Vec::new();
    let mut failure = None;
    'extends: for base in &iface.extends {
        if base.type_args.is_some() {
            failure =
                Some("extends a base with type arguments, which is not classified yet".to_string());
            break;
        }
        let Expr::Ident(base_ident) = base.expr.as_ref() else {
            failure = Some(
                "has an unsupported `extends` target (only a plain interface name)".to_string(),
            );
            break;
        };
        let base_name = base_ident.sym.to_string();
        match resolve_interface(&base_name, raw, resolved, in_progress) {
            DtsType::Native(HirType::Object(base_fields)) => {
                for (field_name, field_ty) in base_fields {
                    if fields.iter().any(|(n, _)| *n == field_name) {
                        failure = Some(format!(
                            "inherits field `{field_name}` from `{base_name}`, which collides with an earlier field"
                        ));
                        break 'extends;
                    }
                    fields.push((field_name, field_ty));
                }
            }
            DtsType::Native(_) => {
                unreachable!("resolve_interface always returns an Object or Unsupported")
            }
            DtsType::Unsupported(reason) => {
                failure = Some(format!("extends unresolvable base `{base_name}`: {reason}"));
                break;
            }
        }
    }

    if failure.is_none() {
        for member in &iface.body.body {
            let TsTypeElement::TsPropertySignature(prop) = member else {
                failure = Some("has a non-property member (method/index signature)".to_string());
                break;
            };
            let field_name = match prop.key.as_ref() {
                Expr::Ident(ident) => ident.sym.to_string(),
                _ => {
                    failure = Some("has an unsupported property key".to_string());
                    break;
                }
            };
            if fields.iter().any(|(n, _)| *n == field_name) {
                failure = Some(format!(
                    "declares field `{field_name}`, which collides with an inherited field"
                ));
                break;
            }
            let field_ty = match &prop.type_ann {
                Some(ann) => {
                    resolve_type_with_interfaces(&ann.type_ann, raw, resolved, in_progress)
                }
                None => {
                    DtsType::Unsupported(format!("field `{field_name}` has no type annotation"))
                }
            };
            match field_ty {
                // Any type hir_codegen's `basic_type` can represent is fine
                // as a field now (fields are word-sized regardless of
                // their own type -- see hir_codegen.rs's `basic_type` for
                // the `HirType::Object` case), including a nested object.
                DtsType::Native(ty) => fields.push((field_name, ty)),
                DtsType::Unsupported(reason) => {
                    failure = Some(format!("field `{field_name}`: {reason}"));
                    break;
                }
            }
        }
    }

    in_progress.pop();

    let result = match failure {
        Some(reason) => DtsType::Unsupported(format!("interface `{name}` {reason}")),
        None => DtsType::Native(HirType::Object(fields)),
    };
    resolved.insert(name.to_string(), result.clone());
    result
}

/// Like `classify_ts_type`, but additionally resolves a `TsTypeRef` naming
/// a not-yet-resolved interface, recursively. Used only while building the
/// interface table (`resolve_interfaces`).
fn resolve_type_with_interfaces(
    ty: &TsType,
    raw: &HashMap<String, &TsInterfaceDecl>,
    resolved: &mut HashMap<String, DtsType>,
    in_progress: &mut Vec<String>,
) -> DtsType {
    if let TsType::TsTypeRef(ty_ref) = ty {
        if let TsEntityName::Ident(id) = &ty_ref.type_name {
            let ref_name = id.sym.as_str();
            if raw.contains_key(ref_name) {
                return resolve_interface(ref_name, raw, resolved, in_progress);
            }
        }
    }
    // No generic interfaces here by design -- see `GenericInterfaces`'s
    // scope note.
    classify_ts_type(ty, resolved, &GenericInterfaces::new())
}

/// Never fails: an unsupported parameter pattern (e.g. destructuring)
/// degrades that one parameter to `DtsType::Unsupported`, same as any
/// other unclassifiable type, rather than aborting this function -- and,
/// since `parse_dts` used to propagate that abort as a `Result::Err`
/// covering *every* function in the file, rather than aborting every
/// other function in the same `.d.ts` file along with it. Validated
/// against a real npm package's `.d.ts` corpus (date-fns): one function
/// with a destructured parameter (`function milliseconds({ years, ... }:
/// Duration)`) used to silently delete every other function in that file
/// from `parse_dts`'s result.
fn lower_dts_function(
    name: &str,
    func: &Function,
    interfaces: &HashMap<String, DtsType>,
    generic_interfaces: &GenericInterfaces,
) -> DtsFunction {
    let name = name.to_string();

    let rest_param = func.params.last().and_then(|param| {
        let Pat::Rest(rest) = &param.pat else {
            return None;
        };
        let name = match rest.arg.as_ref() {
            Pat::Ident(binding) => binding.id.sym.to_string(),
            _ => "rest".to_string(),
        };
        let ty = match rest.type_ann.as_ref() {
            Some(annotation) => match annotation.type_ann.as_ref() {
                TsType::TsArrayType(array) => {
                    classify_variadic_ts_type(&array.elem_type, interfaces, generic_interfaces)
                }
                other => DtsType::Unsupported(format!(
                    "rest parameter must have an array type, found {}",
                    describe_ts_type(other)
                )),
            },
            None => DtsType::Unsupported("missing rest parameter type annotation".into()),
        };
        Some((name, ty))
    });
    let fixed_param_count = func.params.len() - usize::from(rest_param.is_some());
    let params = func
        .params
        .iter()
        .take(fixed_param_count)
        .enumerate()
        .map(|(i, param)| {
            let Pat::Ident(binding) = &param.pat else {
                let reason =
                    "unsupported parameter pattern (only simple identifiers are classified yet)"
                        .to_string();
                return (format!("arg{i}"), DtsType::Unsupported(reason));
            };
            let param_name = binding.id.sym.to_string();
            let ty = match &binding.type_ann {
                Some(ann) => classify_ts_type(&ann.type_ann, interfaces, generic_interfaces),
                None => DtsType::Unsupported("missing type annotation".to_string()),
            };
            (param_name, ty)
        })
        .collect::<Vec<_>>();

    let ret = match &func.return_type {
        Some(ann) => classify_ts_type(&ann.type_ann, interfaces, generic_interfaces),
        None => DtsType::Native(HirType::Void),
    };

    DtsFunction {
        name,
        params,
        rest_param,
        ret,
    }
}

/// Renders an arbitrary `.d.ts` type back to a short, TS-like expression,
/// for use in a `Classification::Fallback` `reason`. Validated against a
/// real npm package's `.d.ts` corpus (date-fns): without this, reasons for
/// anything beyond the simplest unsupported types were unreadable `{:?}`
/// dumps of the full AST node (spans, nested `Box`es, etc.) -- e.g.
/// `unsupported type TsUnionOrIntersectionType(TsIntersectionType(TsIntersectionType
/// { span: 1063..1081, types: [...] }))` for a type that's really just
/// `DateArg<Date> & {}`. This never needs to be exhaustive or fully
/// faithful (it's a diagnostic message, not something re-parsed), so
/// unusual type forms fall back to a short generic label instead of
/// recursing further.
fn describe_ts_type(ty: &TsType) -> String {
    match ty {
        TsType::TsKeywordType(kw) => keyword_name(kw.kind).to_string(),
        TsType::TsThisType(_) => "this".to_string(),
        TsType::TsFnOrConstructorType(_) => "a function type".to_string(),
        TsType::TsTypeRef(ty_ref) => {
            let name = match &ty_ref.type_name {
                TsEntityName::Ident(id) => id.sym.to_string(),
                TsEntityName::TsQualifiedName(_) => "<qualified name>".to_string(),
            };
            match &ty_ref.type_params {
                Some(params) => {
                    let args = params
                        .params
                        .iter()
                        .map(|p| describe_ts_type(p))
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("{name}<{args}>")
                }
                None => name,
            }
        }
        TsType::TsTypeQuery(_) => "a `typeof` query type".to_string(),
        TsType::TsTypeLit(lit) if lit.members.is_empty() => "{}".to_string(),
        TsType::TsTypeLit(_) => "{ ... }".to_string(),
        TsType::TsArrayType(arr) => format!("{}[]", describe_ts_type(&arr.elem_type)),
        TsType::TsTupleType(_) => "a tuple type".to_string(),
        TsType::TsOptionalType(opt) => format!("{}?", describe_ts_type(&opt.type_ann)),
        TsType::TsRestType(rest) => format!("...{}", describe_ts_type(&rest.type_ann)),
        TsType::TsUnionOrIntersectionType(TsUnionOrIntersectionType::TsUnionType(u)) => u
            .types
            .iter()
            .map(|t| describe_ts_type(t))
            .collect::<Vec<_>>()
            .join(" | "),
        TsType::TsUnionOrIntersectionType(TsUnionOrIntersectionType::TsIntersectionType(i)) => i
            .types
            .iter()
            .map(|t| describe_ts_type(t))
            .collect::<Vec<_>>()
            .join(" & "),
        TsType::TsConditionalType(_) => "a conditional type".to_string(),
        TsType::TsInferType(_) => "an `infer` type".to_string(),
        TsType::TsParenthesizedType(p) => format!("({})", describe_ts_type(&p.type_ann)),
        TsType::TsTypeOperator(op) => {
            let op_name = match op.op {
                TsTypeOperatorOp::KeyOf => "keyof",
                TsTypeOperatorOp::Unique => "unique",
                TsTypeOperatorOp::ReadOnly => "readonly",
            };
            format!("{op_name} {}", describe_ts_type(&op.type_ann))
        }
        TsType::TsIndexedAccessType(_) => "an indexed access type".to_string(),
        TsType::TsMappedType(_) => "a mapped type".to_string(),
        TsType::TsLitType(lit) => match &lit.lit {
            // `Wtf8Atom` has no `Display`; its `Debug` already renders as
            // a quoted string, which is exactly what's wanted here.
            TsLit::Str(s) => format!("{:?}", s.value),
            TsLit::Bool(b) => b.value.to_string(),
            TsLit::Number(n) => n.value.to_string(),
            _ => "a literal type".to_string(),
        },
        TsType::TsTypePredicate(_) => "a type predicate".to_string(),
        TsType::TsImportType(_) => "an `import()` type".to_string(),
    }
}

/// The TS keyword spelling for a `TsKeywordTypeKind` (`number`/`string`/
/// `boolean`/`void` are handled natively by `classify_ts_type` and never
/// reach here; this covers the rest for `describe_ts_type`/error messages).
fn keyword_name(kind: TsKeywordTypeKind) -> &'static str {
    match kind {
        TsKeywordTypeKind::TsAnyKeyword => "any",
        TsKeywordTypeKind::TsUnknownKeyword => "unknown",
        TsKeywordTypeKind::TsNumberKeyword => "number",
        TsKeywordTypeKind::TsObjectKeyword => "object",
        TsKeywordTypeKind::TsBooleanKeyword => "boolean",
        TsKeywordTypeKind::TsBigIntKeyword => "bigint",
        TsKeywordTypeKind::TsStringKeyword => "string",
        TsKeywordTypeKind::TsSymbolKeyword => "symbol",
        TsKeywordTypeKind::TsVoidKeyword => "void",
        TsKeywordTypeKind::TsUndefinedKeyword => "undefined",
        TsKeywordTypeKind::TsNullKeyword => "null",
        TsKeywordTypeKind::TsNeverKeyword => "never",
        TsKeywordTypeKind::TsIntrinsicKeyword => "intrinsic",
    }
}

/// Mirrors `thaw_hir::lower::lower_ts_type`'s mapping rules, but never
/// fails: anything it can't map becomes `DtsType::Unsupported` with a
/// reason, for `classify` to report per-parameter/return instead of
/// aborting the whole `.d.ts` file over one unsupported signature.
fn classify_ts_type(
    ty: &TsType,
    interfaces: &HashMap<String, DtsType>,
    generic_interfaces: &GenericInterfaces,
) -> DtsType {
    match ty {
        TsType::TsParenthesizedType(parenthesized) => {
            classify_ts_type(&parenthesized.type_ann, interfaces, generic_interfaces)
        }
        TsType::TsTypeOperator(operator) if operator.op == TsTypeOperatorOp::ReadOnly => {
            classify_ts_type(&operator.type_ann, interfaces, generic_interfaces)
        }
        TsType::TsKeywordType(kw) => match kw.kind {
            TsKeywordTypeKind::TsNumberKeyword => DtsType::Native(HirType::F64),
            TsKeywordTypeKind::TsStringKeyword => DtsType::Native(HirType::Str),
            TsKeywordTypeKind::TsBooleanKeyword => DtsType::Native(HirType::Bool),
            TsKeywordTypeKind::TsVoidKeyword => DtsType::Native(HirType::Void),
            other => DtsType::Unsupported(format!("`{}` is not supported", keyword_name(other))),
        },

        TsType::TsLitType(literal) => match &literal.lit {
            TsLit::Number(_) => DtsType::Native(HirType::F64),
            TsLit::Str(_) => DtsType::Native(HirType::Str),
            TsLit::Bool(_) => DtsType::Native(HirType::Bool),
            _ => DtsType::Unsupported("unsupported literal type".into()),
        },

        TsType::TsUnionOrIntersectionType(TsUnionOrIntersectionType::TsUnionType(union)) => {
            let mut native = None;
            for element in &union.types {
                match classify_ts_type(element, interfaces, generic_interfaces) {
                    DtsType::Native(ty) if native.as_ref().is_none_or(|current| current == &ty) => {
                        native = Some(ty);
                    }
                    DtsType::Native(_) => {
                        return DtsType::Unsupported(format!(
                            "unsupported type `{}`",
                            describe_ts_type(ty)
                        ))
                    }
                    unsupported => return unsupported,
                }
            }
            native
                .map(DtsType::Native)
                .unwrap_or_else(|| DtsType::Unsupported("empty union type is not supported".into()))
        }

        TsType::TsUnionOrIntersectionType(TsUnionOrIntersectionType::TsIntersectionType(
            intersection,
        )) => {
            let mut native = Vec::new();
            for element in &intersection.types {
                match classify_ts_type(element, interfaces, generic_interfaces) {
                    DtsType::Native(ty) => native.push(ty),
                    DtsType::Unsupported(_) => {
                        return DtsType::Unsupported(format!(
                            "unsupported type `{}`",
                            describe_ts_type(ty)
                        ))
                    }
                }
            }
            let Some(first) = native.first() else {
                return DtsType::Unsupported("empty intersection type is not supported".into());
            };
            if native.iter().all(|element| element == first) {
                return DtsType::Native(first.clone());
            }
            if native
                .iter()
                .all(|element| matches!(element, HirType::Object(_)))
            {
                let mut merged = Vec::new();
                for element in native {
                    let HirType::Object(fields) = element else {
                        unreachable!()
                    };
                    for (name, ty) in fields {
                        if let Some((_, existing)) =
                            merged.iter().find(|(existing, _)| existing == &name)
                        {
                            if existing != &ty {
                                return DtsType::Unsupported(format!(
                                    "intersection field `{name}` has conflicting native layouts"
                                ));
                            }
                        } else {
                            merged.push((name, ty));
                        }
                    }
                }
                return DtsType::Native(HirType::Object(merged));
            }
            DtsType::Unsupported(format!("unsupported type `{}`", describe_ts_type(ty)))
        }

        TsType::TsArrayType(arr) => {
            match classify_ts_type(&arr.elem_type, interfaces, generic_interfaces) {
                DtsType::Native(
                    element @ (HirType::F64 | HirType::Str | HirType::Bool | HirType::JsValue),
                ) => {
                    DtsType::Native(HirType::Array(Box::new(element)))
                }
                DtsType::Native(other) => DtsType::Unsupported(format!(
                    "array element type {other:?} is not supported yet (supports number[], string[], boolean[], and JsValue[])"
                )),
                DtsType::Unsupported(reason) => {
                    DtsType::Unsupported(format!("array element type: {reason}"))
                }
            }
        }

        TsType::TsTupleType(tuple) => {
            let elements = tuple
                .elem_types
                .iter()
                .enumerate()
                .map(|(index, element)| {
                    match classify_ts_type(&element.ty, interfaces, generic_interfaces) {
                        DtsType::Native(element) => Ok(element),
                        DtsType::Unsupported(reason) => {
                            Err(format!("tuple element {index}: {reason}"))
                        }
                    }
                })
                .collect::<Result<Vec<_>, _>>();
            match elements {
                Ok(elements) => DtsType::Native(HirType::Tuple(elements)),
                Err(reason) => DtsType::Unsupported(reason),
            }
        }

        TsType::TsFnOrConstructorType(TsFnOrConstructorType::TsFnType(function)) => {
            if function.type_params.is_some() {
                return DtsType::Unsupported("generic callback types are not supported".into());
            }
            let mut params = Vec::with_capacity(function.params.len());
            let mut optional = Vec::with_capacity(function.params.len());
            let mut rest = None;
            for (index, parameter) in function.params.iter().enumerate() {
                let (annotation, is_optional) = match parameter {
                    TsFnParam::Ident(parameter) => {
                        let Some(annotation) = &parameter.type_ann else {
                            return DtsType::Unsupported(format!(
                                "callback parameter `{}` has no type annotation",
                                parameter.id.sym
                            ));
                        };
                        (annotation, parameter.id.optional)
                    }
                    TsFnParam::Rest(parameter) if index + 1 == function.params.len() => {
                        let Some(annotation) = &parameter.type_ann else {
                            return DtsType::Unsupported(
                                "callback rest parameter has no type annotation".into(),
                            );
                        };
                        let TsType::TsArrayType(array) = annotation.type_ann.as_ref() else {
                            return DtsType::Unsupported(
                                "callback rest parameter must use an array type".into(),
                            );
                        };
                        match classify_ts_type(&array.elem_type, interfaces, generic_interfaces) {
                            DtsType::Native(ty) => rest = Some(ty),
                            DtsType::Unsupported(reason) => {
                                return DtsType::Unsupported(format!(
                                    "callback rest element type: {reason}"
                                ));
                            }
                        }
                        continue;
                    }
                    TsFnParam::Rest(_) => {
                        return DtsType::Unsupported("callback rest parameter must be last".into());
                    }
                    _ => {
                        return DtsType::Unsupported(
                            "callback parameters must be identifiers or a trailing rest parameter"
                                .into(),
                        );
                    }
                };
                match classify_ts_type(&annotation.type_ann, interfaces, generic_interfaces) {
                    DtsType::Native(mut ty) => {
                        if is_optional {
                            ty = match ty {
                                HirType::Optional(_) | HirType::Nullish(_) => ty,
                                HirType::Nullable(payload) => HirType::Nullish(payload),
                                other => HirType::Optional(Box::new(other)),
                            };
                        }
                        params.push(ty);
                    }
                    // Callback values cross the JavaScript/N-API boundary as
                    // dynamic JSON. In real Node declarations the error slot
                    // is normally `Error | null` and result slots are often
                    // `any`; neither has a native AOT layout, but both have a
                    // faithful dynamic representation at this boundary.
                    DtsType::Unsupported(_) => params.push(HirType::Json),
                }
                optional.push(is_optional);
            }
            match classify_ts_type(&function.type_ann.type_ann, interfaces, generic_interfaces) {
                DtsType::Native(ret) => {
                    DtsType::Native(if rest.is_some() || optional.iter().any(|value| *value) {
                        let optional = HirOptionalMask::from_bools(&optional);
                        HirType::CallableFunction(
                            params,
                            optional,
                            rest.map(Box::new),
                            Box::new(ret),
                        )
                    } else {
                        HirType::Function(params, Box::new(ret))
                    })
                }
                DtsType::Unsupported(reason) => {
                    DtsType::Unsupported(format!("callback return type: {reason}"))
                }
            }
        }
        TsType::TsFnOrConstructorType(TsFnOrConstructorType::TsConstructorType(_)) => {
            DtsType::Unsupported("constructor callback types are not supported".into())
        }

        TsType::TsTypeLit(type_lit) => {
            let mut fields = Vec::with_capacity(type_lit.members.len());
            for member in &type_lit.members {
                let TsTypeElement::TsPropertySignature(prop) = member else {
                    return DtsType::Unsupported(
                        "object type literal has a non-property member (method/index signature)"
                            .to_string(),
                    );
                };
                let field_name = match prop.key.as_ref() {
                    Expr::Ident(ident) => ident.sym.to_string(),
                    _ => {
                        return DtsType::Unsupported(
                            "unsupported object type literal key".to_string(),
                        )
                    }
                };
                let field_ty = match &prop.type_ann {
                    Some(ann) => classify_ts_type(&ann.type_ann, interfaces, generic_interfaces),
                    None => {
                        DtsType::Unsupported(format!("field `{field_name}` has no type annotation"))
                    }
                };
                match field_ty {
                    // See the parallel comment in `resolve_interface`:
                    // any representable type works as a field now.
                    DtsType::Native(ty) => fields.push((field_name, ty)),
                    DtsType::Unsupported(reason) => {
                        return DtsType::Unsupported(format!(
                            "object field `{field_name}`: {reason}"
                        ))
                    }
                }
            }
            DtsType::Native(HirType::Object(fields))
        }

        TsType::TsTypeRef(ty_ref) => {
            let ref_name = match &ty_ref.type_name {
                TsEntityName::Ident(id) => id.sym.to_string(),
                TsEntityName::TsQualifiedName(_) => {
                    return DtsType::Unsupported(
                        "qualified type names are not supported yet".to_string(),
                    )
                }
            };

            // A name matching a resolved (non-generic) `interface` --
            // treated exactly like an inline `{ ... }` type literal.
            if let Some(resolved) = interfaces.get(&ref_name) {
                return resolved.clone();
            }
            // A generic interface, referenced with concrete type
            // arguments -- resolved on demand via substitution.
            if let Some(decl) = generic_interfaces.get(&ref_name) {
                return resolve_generic_interface(
                    &ref_name,
                    decl,
                    ty_ref,
                    interfaces,
                    generic_interfaces,
                    &mut Vec::new(),
                );
            }
            if ref_name == "Json" && ty_ref.type_params.is_none() {
                return DtsType::Native(HirType::Json);
            }
            if ref_name == "JsValue" && ty_ref.type_params.is_none() {
                return DtsType::Native(HirType::JsValue);
            }
            if ref_name == "Array" {
                let Some(element) = ty_ref
                    .type_params
                    .as_ref()
                    .and_then(|params| params.params.first())
                else {
                    return DtsType::Unsupported("Array<T> needs one type argument".to_string());
                };
                return match classify_ts_type(element, interfaces, generic_interfaces) {
                    DtsType::Native(
                        element @ (HirType::F64 | HirType::Str | HirType::Bool | HirType::JsValue),
                    ) => {
                        DtsType::Native(HirType::Array(Box::new(element)))
                    }
                    _ => DtsType::Unsupported(
                        "only number[], string[], boolean[], and JsValue[] have native dynamic layouts".to_string(),
                    ),
                };
            }

            // Note: no `Promise<T>` recognition here (unlike
            // thaw-hir's `lower_ts_type`) -- a `.d.ts` signature using
            // either still needs a real decision about arena lifetime
            // (arrays) or the async ABI (promises) across a *foreign* FFI
            // boundary that section 5 of the design doc explicitly defers.
            DtsType::Unsupported(format!(
                "type reference `{ref_name}` is not classified yet (Array<T>/Promise<T>)"
            ))
        }

        other => DtsType::Unsupported(format!("unsupported type `{}`", describe_ts_type(other))),
    }
}

/// Resolves `Name<ConcreteArg, ...>` for a generic interface `Name`,
/// mirroring `thaw_hir::lower::resolve_generic_interface` but degrading to
/// `DtsType::Unsupported` instead of erroring (wrong argument count,
/// self-reference, `extends`, or an unsupported member all degrade rather
/// than abort). Same scope limits as the thaw-hir version.
fn resolve_generic_interface(
    name: &str,
    decl: &TsInterfaceDecl,
    ty_ref: &swc_ecma_ast::TsTypeRef,
    interfaces: &HashMap<String, DtsType>,
    generic_interfaces: &GenericInterfaces,
    in_progress: &mut Vec<String>,
) -> DtsType {
    if in_progress.iter().any(|n| n == name) {
        return DtsType::Unsupported(format!(
            "generic interface `{name}` is (indirectly) self-referential"
        ));
    }
    if !decl.extends.is_empty() {
        return DtsType::Unsupported(format!(
            "generic interface `{name}` cannot use `extends` yet"
        ));
    }

    let type_param_decl = decl
        .type_params
        .as_ref()
        .expect("caller only reaches here for a generic interface");
    let type_param_names: Vec<String> = type_param_decl
        .params
        .iter()
        .map(|p| p.name.sym.to_string())
        .collect();

    let type_args: &[Box<TsType>] = ty_ref
        .type_params
        .as_ref()
        .map(|params| params.params.as_slice())
        .unwrap_or(&[]);
    if type_args.len() != type_param_names.len() {
        return DtsType::Unsupported(format!(
            "interface `{name}` expects {} type argument(s), got {}",
            type_param_names.len(),
            type_args.len()
        ));
    }

    let mut substitution: HashMap<String, HirType> = HashMap::new();
    for (param_name, arg) in type_param_names.into_iter().zip(type_args) {
        match classify_ts_type(arg, interfaces, generic_interfaces) {
            DtsType::Native(ty) => {
                substitution.insert(param_name, ty);
            }
            DtsType::Unsupported(reason) => {
                return DtsType::Unsupported(format!("type argument for `{param_name}`: {reason}"))
            }
        }
    }

    in_progress.push(name.to_string());

    let mut fields = Vec::with_capacity(decl.body.body.len());
    let mut failure = None;
    for member in &decl.body.body {
        let TsTypeElement::TsPropertySignature(prop) = member else {
            failure = Some("has a non-property member (method/index signature)".to_string());
            break;
        };
        let field_name = match prop.key.as_ref() {
            Expr::Ident(ident) => ident.sym.to_string(),
            _ => {
                failure = Some("has an unsupported property key".to_string());
                break;
            }
        };
        let field_ty = match &prop.type_ann {
            Some(ann) => resolve_ts_type_with_substitution(
                &ann.type_ann,
                &substitution,
                interfaces,
                generic_interfaces,
                in_progress,
            ),
            None => DtsType::Unsupported(format!("field `{field_name}` has no type annotation")),
        };
        match field_ty {
            DtsType::Native(ty) => fields.push((field_name, ty)),
            DtsType::Unsupported(reason) => {
                failure = Some(format!("field `{field_name}`: {reason}"));
                break;
            }
        }
    }

    in_progress.pop();

    match failure {
        Some(reason) => DtsType::Unsupported(format!("interface `{name}` {reason}")),
        None => DtsType::Native(HirType::Object(fields)),
    }
}

/// Like `classify_ts_type`, but a bare `TsTypeRef` matching one of `Name`'s
/// type parameters resolves to the corresponding concrete type instead of
/// an unknown-reference `Unsupported`. Mirrors
/// `thaw_hir::lower::resolve_ts_type_with_substitution`.
fn resolve_ts_type_with_substitution(
    ty: &TsType,
    substitution: &HashMap<String, HirType>,
    interfaces: &HashMap<String, DtsType>,
    generic_interfaces: &GenericInterfaces,
    in_progress: &mut Vec<String>,
) -> DtsType {
    if let TsType::TsTypeRef(ty_ref) = ty {
        if let TsEntityName::Ident(id) = &ty_ref.type_name {
            let ref_name = id.sym.as_str();
            if let Some(concrete) = substitution.get(ref_name) {
                return DtsType::Native(concrete.clone());
            }
            if let Some(decl) = generic_interfaces.get(ref_name) {
                return resolve_generic_interface(
                    ref_name,
                    decl,
                    ty_ref,
                    interfaces,
                    generic_interfaces,
                    in_progress,
                );
            }
            if let Some(params) = &ty_ref.type_params {
                if let [elem] = params.params.as_slice() {
                    let resolved_elem = resolve_ts_type_with_substitution(
                        elem,
                        substitution,
                        interfaces,
                        generic_interfaces,
                        in_progress,
                    );
                    match (ref_name, resolved_elem) {
                        ("Array", DtsType::Native(element @ (HirType::F64 | HirType::Str | HirType::Bool | HirType::JsValue))) => {
                            return DtsType::Native(HirType::Array(Box::new(element)))
                        }
                        ("Array", DtsType::Native(other)) => {
                            return DtsType::Unsupported(format!(
                                "array element type {other:?} is not supported yet (supports number[], string[], boolean[], and JsValue[])"
                            ))
                        }
                        ("Array", DtsType::Unsupported(reason)) => {
                            return DtsType::Unsupported(format!("array element type: {reason}"))
                        }
                        _ => {}
                    }
                }
            }
        }
        return classify_ts_type(ty, interfaces, generic_interfaces);
    }

    match ty {
        TsType::TsParenthesizedType(parenthesized) => resolve_ts_type_with_substitution(
            &parenthesized.type_ann,
            substitution,
            interfaces,
            generic_interfaces,
            in_progress,
        ),
        TsType::TsTypeOperator(operator) if operator.op == TsTypeOperatorOp::ReadOnly => {
            resolve_ts_type_with_substitution(
                &operator.type_ann,
                substitution,
                interfaces,
                generic_interfaces,
                in_progress,
            )
        }
        TsType::TsArrayType(arr) => {
            match resolve_ts_type_with_substitution(
                &arr.elem_type,
                substitution,
                interfaces,
                generic_interfaces,
                in_progress,
            ) {
                DtsType::Native(
                    element @ (HirType::F64 | HirType::Str | HirType::Bool | HirType::JsValue),
                ) => {
                    DtsType::Native(HirType::Array(Box::new(element)))
                }
                DtsType::Native(other) => DtsType::Unsupported(format!(
                    "array element type {other:?} is not supported yet (supports number[], string[], boolean[], and JsValue[])"
                )),
                DtsType::Unsupported(reason) => {
                    DtsType::Unsupported(format!("array element type: {reason}"))
                }
            }
        }
        TsType::TsTupleType(tuple) => {
            let elements = tuple
                .elem_types
                .iter()
                .enumerate()
                .map(|(index, element)| {
                    match resolve_ts_type_with_substitution(
                        &element.ty,
                        substitution,
                        interfaces,
                        generic_interfaces,
                        in_progress,
                    ) {
                        DtsType::Native(element) => Ok(element),
                        DtsType::Unsupported(reason) => {
                            Err(format!("tuple element {index}: {reason}"))
                        }
                    }
                })
                .collect::<Result<Vec<_>, _>>();
            match elements {
                Ok(elements) => DtsType::Native(HirType::Tuple(elements)),
                Err(reason) => DtsType::Unsupported(reason),
            }
        }
        TsType::TsTypeLit(type_lit) => {
            let mut fields = Vec::with_capacity(type_lit.members.len());
            for member in &type_lit.members {
                let TsTypeElement::TsPropertySignature(prop) = member else {
                    return DtsType::Unsupported(
                        "object type literal has a non-property member (method/index signature)"
                            .to_string(),
                    );
                };
                let field_name = match prop.key.as_ref() {
                    Expr::Ident(ident) => ident.sym.to_string(),
                    _ => {
                        return DtsType::Unsupported(
                            "unsupported object type literal key".to_string(),
                        )
                    }
                };
                let field_ty = match &prop.type_ann {
                    Some(ann) => resolve_ts_type_with_substitution(
                        &ann.type_ann,
                        substitution,
                        interfaces,
                        generic_interfaces,
                        in_progress,
                    ),
                    None => {
                        DtsType::Unsupported(format!("field `{field_name}` has no type annotation"))
                    }
                };
                match field_ty {
                    DtsType::Native(ty) => fields.push((field_name, ty)),
                    DtsType::Unsupported(reason) => {
                        return DtsType::Unsupported(format!(
                            "object field `{field_name}`: {reason}"
                        ))
                    }
                }
            }
            DtsType::Native(HirType::Object(fields))
        }
        other => classify_ts_type(other, interfaces, generic_interfaces),
    }
}
