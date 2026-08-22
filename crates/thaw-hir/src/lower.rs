//! SWC AST -> Thaw HIR lowering.
//!
//! Phase 0 scope (still true here): no type inference for function
//! params/returns -- those need explicit primitive annotations. Phase 1
//! adds `if`/`while`/classic `for`/`throw`/`try`/`catch`, local
//! `let`/`const`, assignment/`++`/`--`, and number arrays. Phase 2 adds
//! `process.env` and object types (`{ x: number; y: number }`-style
//! records, `f64` fields only at codegen time -- see hir_codegen).
//!
//! Object support is why this module carries a type *scope* now instead of
//! being purely syntax-directed: resolving `obj.field` needs to know
//! whether `obj` is an array (`.length`) or an object (which field, at
//! which offset) without a real type checker. The scope is one flat map per
//! function (params + every `let` seen so far, regardless of block
//! nesting) -- it doesn't model block scoping, matching hir_codegen's own
//! flat variable table, so lowering and codegen agree on what "scope" means.
//!
//! A single SWC `Stmt` can lower to *several* HIR statements (`for` becomes
//! a `Let` followed by a `While`), so the statement lowering entry point is
//! `lower_stmt_seq`, not a single-statement `lower_stmt`.

use std::collections::HashMap;

use swc_ecma_ast::{
    AssignOp, AssignTarget, BinaryOp, CallExpr, Callee, ComputedPropName, Decl, Expr, FnDecl,
    KeyValueProp, Lit, MemberExpr, MemberProp, Module, ModuleItem, ObjectLit as SwcObjectLit, Pat,
    Prop, PropName, PropOrSpread, SimpleAssignTarget, Stmt, TsInterfaceDecl, TsKeywordTypeKind,
    TsType, TsTypeElement, UpdateOp, VarDecl, VarDeclOrExpr,
};

use crate::{
    BinOp, FfiSignature, HirExpr, HirFunction, HirLit, HirParam, HirProgram, HirStmt, HirType,
    Symbol,
};

/// Signature info needed to type calls to other top-level functions during
/// lowering, collected in a pre-pass over the whole module before any
/// function body is lowered (so forward references and mutual calls work).
#[derive(Clone)]
struct FnSignature {
    params: Vec<HirType>,
    ret: HirType,
    /// A function declared with no body (`declare function foo(...): T;`,
    /// or the same syntax without `declare` in a regular `.ts` file --
    /// SWC represents both identically, `body: None`). See
    /// docs/design/bridge.md section 6: calls to these lower to
    /// `HirExpr::FfiCall`, not `HirExpr::Call`.
    is_extern: bool,
}

pub fn lower_module(module: &Module) -> Result<HirProgram, String> {
    let (interfaces, generic_interfaces) = resolve_interfaces(module)?;

    let mut signatures: HashMap<Symbol, FnSignature> = HashMap::new();
    let mut fn_decls = Vec::new();

    for item in &module.body {
        match item {
            ModuleItem::Stmt(Stmt::Decl(Decl::Fn(fn_decl))) => {
                let name = fn_decl.ident.sym.to_string();
                let func = &fn_decl.function;
                let is_extern = func.body.is_none();
                if is_extern && func.is_async {
                    return Err(format!("ambient function `{name}` cannot be async"));
                }
                let params = func
                    .params
                    .iter()
                    .map(|p| lower_param(&p.pat, &interfaces, &generic_interfaces).map(|p| p.ty))
                    .collect::<Result<Vec<_>, _>>()?;
                let ret = lower_fn_return_type(
                    func.is_async,
                    &func.return_type,
                    &name,
                    &interfaces,
                    &generic_interfaces,
                )?;
                signatures.insert(
                    name,
                    FnSignature {
                        params,
                        ret,
                        is_extern,
                    },
                );
                if !is_extern {
                    fn_decls.push(fn_decl);
                }
            }
            // Already consumed by `resolve_interfaces` above.
            ModuleItem::Stmt(Stmt::Decl(Decl::TsInterface(_))) => {}
            ModuleItem::Stmt(_) => {
                return Err(
                    "Phase 0/1/2 only support top-level function declarations and `interface`s; wrap other code in a function"
                        .into(),
                )
            }
            ModuleItem::ModuleDecl(_) => {
                return Err("import/export declarations are not supported yet".into())
            }
        }
    }

    let extern_functions = signatures
        .iter()
        .filter(|(_, sig)| sig.is_extern)
        .map(|(name, sig)| FfiSignature {
            symbol: name.clone(),
            params: sig.params.clone(),
            ret: sig.ret.clone(),
        })
        .collect();

    let functions = fn_decls
        .into_iter()
        .map(|fn_decl| lower_fn_decl(fn_decl, &signatures, &interfaces, &generic_interfaces))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(HirProgram {
        functions,
        extern_functions,
    })
}

/// Non-generic interfaces (fully resolved up front into `HirType::Object`,
/// the first map) plus generic interfaces (kept raw, resolved on demand via
/// substitution at each `Name<ConcreteArgs>` use site -- see
/// `resolve_generic_interface`, second map). No instantiation cache is
/// needed: `HirType::Object` equality is structural, so resolving the same
/// `Box<number>` twice just produces two equal values, not two different
/// ones.
///
/// Scope note: a generic interface may only be referenced directly from a
/// function signature/`let` annotation/etc. (wherever `lower_ts_type` is
/// normally called) -- not from *inside another interface's field* (e.g.
/// `interface Wrapper { box: Box<number>; }` is not resolved specially;
/// `resolve_interface`/`resolve_type_with_interfaces` below never consult
/// the generic map). Supporting that needs the eager resolution pass
/// itself to be substitution-aware, deferred until a real use case asks
/// for it.
type GenericInterfaces<'a> = HashMap<Symbol, &'a TsInterfaceDecl>;

/// Resolves every top-level `interface` declaration, so `lower_ts_type` can
/// treat a `TsTypeRef` naming one exactly like an inline `{ ... }` type
/// literal. Interfaces may be declared in any order and may reference each
/// other; a true cycle (an interface whose field chain refers back to
/// itself) is rejected, since Thaw's flat, fixed-size object layout has no
/// way to represent one.
fn resolve_interfaces(
    module: &Module,
) -> Result<(HashMap<Symbol, HirType>, GenericInterfaces<'_>), String> {
    let mut raw: HashMap<Symbol, &TsInterfaceDecl> = HashMap::new();
    let mut generic: GenericInterfaces = HashMap::new();
    for item in &module.body {
        if let ModuleItem::Stmt(Stmt::Decl(Decl::TsInterface(iface))) = item {
            let name = iface.id.sym.to_string();
            if iface.type_params.is_some() {
                generic.insert(name, iface.as_ref());
            } else {
                raw.insert(name, iface.as_ref());
            }
        }
    }

    let mut resolved = HashMap::new();
    let names: Vec<Symbol> = raw.keys().cloned().collect();
    for name in names {
        resolve_interface(&name, &raw, &mut resolved, &mut Vec::new())?;
    }
    Ok((resolved, generic))
}

fn resolve_interface(
    name: &str,
    raw: &HashMap<Symbol, &TsInterfaceDecl>,
    resolved: &mut HashMap<Symbol, HirType>,
    in_progress: &mut Vec<Symbol>,
) -> Result<HirType, String> {
    if let Some(ty) = resolved.get(name) {
        return Ok(ty.clone());
    }
    if in_progress.iter().any(|n| n == name) {
        return Err(format!(
            "interface `{name}` is (indirectly) self-referential, which Thaw's fixed-size object layout can't represent"
        ));
    }

    let iface = raw
        .get(name)
        .ok_or_else(|| format!("unknown interface `{name}`"))?;

    if iface.type_params.is_some() {
        return Err(format!("generic interfaces are not supported yet (`{name}`)"));
    }

    in_progress.push(name.to_string());

    // `extends`: each base's fields come first, in `extends`-list order,
    // each base's own fields in its own declared order, followed by this
    // interface's own fields. A colliding field name (between two bases,
    // or between a base and this interface's own body) is rejected rather
    // than guessing an override/merge rule.
    let mut fields: Vec<(Symbol, HirType)> = Vec::new();
    for base in &iface.extends {
        if base.type_args.is_some() {
            return Err(format!(
                "interface `{name}` extends a base with type arguments, which is not supported yet"
            ));
        }
        let Expr::Ident(base_ident) = base.expr.as_ref() else {
            return Err(format!(
                "interface `{name}` has an unsupported `extends` target (only a plain interface name is supported)"
            ));
        };
        let base_name = base_ident.sym.to_string();
        let HirType::Object(base_fields) =
            resolve_interface(&base_name, raw, resolved, in_progress)?
        else {
            unreachable!("resolve_interface always returns HirType::Object or an Err")
        };
        for (field_name, field_ty) in base_fields {
            if fields.iter().any(|(n, _)| *n == field_name) {
                return Err(format!(
                    "interface `{name}` inherits field `{field_name}` from `{base_name}`, which collides with an earlier field of the same name"
                ));
            }
            fields.push((field_name, field_ty));
        }
    }

    for member in &iface.body.body {
        let TsTypeElement::TsPropertySignature(prop) = member else {
            return Err(format!(
                "interface `{name}` has an unsupported member (only plain properties are supported, no methods/index signatures)"
            ));
        };
        let field_name = match prop.key.as_ref() {
            Expr::Ident(ident) => ident.sym.to_string(),
            _ => return Err(format!("interface `{name}` has an unsupported property key")),
        };
        if fields.iter().any(|(n, _)| *n == field_name) {
            return Err(format!(
                "interface `{name}` declares field `{field_name}`, which collides with an inherited field of the same name"
            ));
        }
        let ann = prop.type_ann.as_ref().ok_or_else(|| {
            format!("field `{field_name}` on interface `{name}` needs an explicit type annotation")
        })?;
        let field_ty = resolve_type_with_interfaces(&ann.type_ann, raw, resolved, in_progress)?;
        fields.push((field_name, field_ty));
    }

    in_progress.pop();

    let hir_ty = HirType::Object(fields);
    resolved.insert(name.to_string(), hir_ty.clone());
    Ok(hir_ty)
}

/// Like `lower_ts_type`, but additionally resolves a `TsTypeRef` naming a
/// not-yet-resolved interface, recursively. Used only while building the
/// interface table (`resolve_interfaces`); everywhere else, `lower_ts_type`
/// consults the finished, read-only table instead.
fn resolve_type_with_interfaces(
    ty: &TsType,
    raw: &HashMap<Symbol, &TsInterfaceDecl>,
    resolved: &mut HashMap<Symbol, HirType>,
    in_progress: &mut Vec<Symbol>,
) -> Result<HirType, String> {
    if let TsType::TsTypeRef(ty_ref) = ty {
        if let swc_ecma_ast::TsEntityName::Ident(id) = &ty_ref.type_name {
            let ref_name = id.sym.as_str();
            if raw.contains_key(ref_name) {
                return resolve_interface(ref_name, raw, resolved, in_progress);
            }
        }
    }
    // Not an interface reference -- fall through to the ordinary rules.
    // Any nested `TsTypeRef` to another (non-generic) interface inside
    // e.g. an object type literal's field is still caught, since
    // `lower_ts_type` also consults `resolved` for `TsTypeRef` lookups.
    // No generic interfaces here by design -- see the scope note on
    // `GenericInterfaces`.
    lower_ts_type(ty, resolved, &GenericInterfaces::new())
}

fn lower_fn_decl(
    fn_decl: &FnDecl,
    signatures: &HashMap<Symbol, FnSignature>,
    interfaces: &HashMap<Symbol, HirType>,
    generic_interfaces: &GenericInterfaces,
) -> Result<HirFunction, String> {
    let name = fn_decl.ident.sym.to_string();
    let func = &fn_decl.function;

    let params = func
        .params
        .iter()
        .map(|param| lower_param(&param.pat, interfaces, generic_interfaces))
        .collect::<Result<Vec<_>, _>>()?;

    let ret = lower_fn_return_type(
        func.is_async,
        &func.return_type,
        &name,
        interfaces,
        generic_interfaces,
    )?;

    let body_block = func
        .body
        .as_ref()
        .ok_or_else(|| format!("function `{name}` has no body (ambient/overload decl?)"))?;

    let mut lowerer = FnLowerer::new(signatures, interfaces, generic_interfaces, ret.clone());
    for param in &params {
        lowerer.scope.insert(param.name.clone(), param.ty.clone());
    }
    let body = lowerer.lower_stmts(&body_block.stmts)?;

    Ok(HirFunction {
        name,
        params,
        ret,
        is_async: func.is_async,
        body,
    })
}

/// Computes a function's *unwrapped* return type: `async function`s must be
/// declared as returning `Promise<T>`, and this returns `T` -- V1
/// async/await erases `Promise` entirely at lowering time (see
/// docs/design/async-await.md). Non-async functions are unaffected.
fn lower_fn_return_type(
    is_async: bool,
    return_type: &Option<Box<swc_ecma_ast::TsTypeAnn>>,
    fn_name: &str,
    interfaces: &HashMap<Symbol, HirType>,
    generic_interfaces: &GenericInterfaces,
) -> Result<HirType, String> {
    let declared = match return_type {
        Some(ann) => lower_ts_type(&ann.type_ann, interfaces, generic_interfaces)?,
        None => HirType::Void,
    };
    if !is_async {
        return Ok(declared);
    }
    match declared {
        HirType::Promise(inner) => Ok(*inner),
        other => Err(format!(
            "async function `{fn_name}` must be declared as returning `Promise<T>`, found {other:?}"
        )),
    }
}

fn lower_param(
    pat: &Pat,
    interfaces: &HashMap<Symbol, HirType>,
    generic_interfaces: &GenericInterfaces,
) -> Result<HirParam, String> {
    let Pat::Ident(binding) = pat else {
        return Err("only simple identifier parameters are supported".into());
    };
    let name = binding.id.sym.to_string();
    let ty = match &binding.type_ann {
        Some(ann) => lower_ts_type(&ann.type_ann, interfaces, generic_interfaces)?,
        None => {
            return Err(format!(
                "parameter `{name}` needs an explicit type annotation (no type inference for params)"
            ))
        }
    };
    Ok(HirParam { name, ty })
}

fn lower_ts_type(
    ty: &TsType,
    interfaces: &HashMap<Symbol, HirType>,
    generic_interfaces: &GenericInterfaces,
) -> Result<HirType, String> {
    match ty {
        TsType::TsKeywordType(kw) => match kw.kind {
            TsKeywordTypeKind::TsNumberKeyword => Ok(HirType::F64),
            TsKeywordTypeKind::TsStringKeyword => Ok(HirType::Str),
            TsKeywordTypeKind::TsBooleanKeyword => Ok(HirType::Bool),
            TsKeywordTypeKind::TsVoidKeyword => Ok(HirType::Void),
            other => Err(format!(
                "unsupported type keyword {other:?} (supports number/string/boolean/void)"
            )),
        },
        TsType::TsArrayType(arr) => Ok(HirType::Array(Box::new(lower_ts_type(
            &arr.elem_type,
            interfaces,
            generic_interfaces,
        )?))),
        TsType::TsTypeRef(ty_ref) => {
            let ref_name = match &ty_ref.type_name {
                swc_ecma_ast::TsEntityName::Ident(id) => Some(id.sym.as_str()),
                swc_ecma_ast::TsEntityName::TsQualifiedName(_) => None,
            };

            // A name matching a resolved (non-generic) `interface` --
            // treated exactly like an inline `{ ... }` type literal.
            if let Some(name) = ref_name {
                if let Some(resolved) = interfaces.get(name) {
                    return Ok(resolved.clone());
                }
                // A generic interface, referenced with concrete type
                // arguments -- resolved on demand via substitution. See
                // `resolve_generic_interface`'s doc comment for scope
                // limits (no nested-inside-another-interface use, no
                // `extends` on the generic interface itself).
                if let Some(decl) = generic_interfaces.get(name) {
                    return resolve_generic_interface(
                        name,
                        decl,
                        ty_ref,
                        interfaces,
                        generic_interfaces,
                        &mut Vec::new(),
                    );
                }
            }

            // Otherwise, accept `Array<T>` / `Promise<T>` as the two
            // other built-in generic spellings we recognize.
            let single_type_param = ty_ref
                .type_params
                .as_ref()
                .and_then(|params| match params.params.as_slice() {
                    [elem] => Some(elem.as_ref()),
                    _ => None,
                });

            match (ref_name, single_type_param) {
                (Some("Array"), Some(elem)) => Ok(HirType::Array(Box::new(lower_ts_type(
                    elem,
                    interfaces,
                    generic_interfaces,
                )?))),
                (Some("Promise"), Some(inner)) => Ok(HirType::Promise(Box::new(lower_ts_type(
                    inner,
                    interfaces,
                    generic_interfaces,
                )?))),
                _ => Err("unsupported type reference (generics are not supported yet)".into()),
            }
        }
        TsType::TsTypeLit(type_lit) => {
            let fields = type_lit
                .members
                .iter()
                .map(|member| {
                    let TsTypeElement::TsPropertySignature(prop) = member else {
                        return Err(
                            "only plain properties are supported in object type literals (no methods/index signatures)"
                                .to_string(),
                        );
                    };
                    let name = match prop.key.as_ref() {
                        Expr::Ident(ident) => ident.sym.to_string(),
                        _ => return Err("unsupported object type literal key".into()),
                    };
                    let ann = prop.type_ann.as_ref().ok_or_else(|| {
                        format!("field `{name}` needs an explicit type annotation")
                    })?;
                    Ok((name, lower_ts_type(&ann.type_ann, interfaces, generic_interfaces)?))
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok(HirType::Object(fields))
        }
        other => Err(format!(
            "unsupported type annotation {other:?} (supports primitive keywords, T[]/Array<T>, interfaces, and object type literals)"
        )),
    }
}

/// Resolves `Name<ConcreteArg, ...>` for a generic interface `Name`, by
/// substituting each type parameter with its corresponding concrete
/// argument's `HirType` throughout the interface's field types (see
/// `resolve_ts_type_with_substitution`). `in_progress` guards against a
/// generic interface that references itself (directly, or through another
/// generic interface) -- freshly created at each top-level `lower_ts_type`
/// call, so it only needs to catch a cycle within one such call tree.
///
/// Scope limits (see `GenericInterfaces`'s doc comment for the broader
/// one): the generic interface itself cannot use `extends`; a field type
/// that references *another* generic interface using one of *this*
/// interface's own type parameters as an argument (e.g. `interface
/// Wrapper<T> { boxed: Box<T>; }` where `Box` is also generic) is not
/// substituted into -- only `T` used directly, in `T[]`/`Array<T>`/
/// `Promise<T>`, or as an object type literal field is.
fn resolve_generic_interface(
    name: &str,
    decl: &TsInterfaceDecl,
    ty_ref: &swc_ecma_ast::TsTypeRef,
    interfaces: &HashMap<Symbol, HirType>,
    generic_interfaces: &GenericInterfaces,
    in_progress: &mut Vec<Symbol>,
) -> Result<HirType, String> {
    if in_progress.iter().any(|n| n == name) {
        return Err(format!(
            "generic interface `{name}` is (indirectly) self-referential, which Thaw's fixed-size object layout can't represent"
        ));
    }
    if !decl.extends.is_empty() {
        return Err(format!("generic interface `{name}` cannot use `extends` yet"));
    }

    let type_param_decl = decl
        .type_params
        .as_ref()
        .expect("caller only reaches here for a generic interface");
    let type_param_names: Vec<Symbol> = type_param_decl
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
        return Err(format!(
            "interface `{name}` expects {} type argument(s), got {}",
            type_param_names.len(),
            type_args.len()
        ));
    }
    let resolved_args = type_args
        .iter()
        .map(|arg| lower_ts_type(arg, interfaces, generic_interfaces))
        .collect::<Result<Vec<_>, String>>()?;
    let substitution: HashMap<Symbol, HirType> =
        type_param_names.into_iter().zip(resolved_args).collect();

    in_progress.push(name.to_string());

    let mut fields = Vec::with_capacity(decl.body.body.len());
    for member in &decl.body.body {
        let TsTypeElement::TsPropertySignature(prop) = member else {
            return Err(format!(
                "interface `{name}` has an unsupported member (only plain properties are supported, no methods/index signatures)"
            ));
        };
        let field_name = match prop.key.as_ref() {
            Expr::Ident(ident) => ident.sym.to_string(),
            _ => return Err(format!("interface `{name}` has an unsupported property key")),
        };
        let ann = prop.type_ann.as_ref().ok_or_else(|| {
            format!("field `{field_name}` on interface `{name}` needs an explicit type annotation")
        })?;
        let field_ty = resolve_ts_type_with_substitution(
            &ann.type_ann,
            &substitution,
            interfaces,
            generic_interfaces,
            in_progress,
        )?;
        fields.push((field_name, field_ty));
    }

    in_progress.pop();

    Ok(HirType::Object(fields))
}

/// Like `lower_ts_type`, but a bare `TsTypeRef` matching one of `Name`'s
/// type parameters resolves to the corresponding concrete `HirType`
/// instead of erroring as an unknown reference. Recurses into itself (not
/// plain `lower_ts_type`) for `T[]`/`Array<T>`/`Promise<T>`/object type
/// literal sub-parts, so a type parameter used deeper inside those still
/// gets substituted.
fn resolve_ts_type_with_substitution(
    ty: &TsType,
    substitution: &HashMap<Symbol, HirType>,
    interfaces: &HashMap<Symbol, HirType>,
    generic_interfaces: &GenericInterfaces,
    in_progress: &mut Vec<Symbol>,
) -> Result<HirType, String> {
    if let TsType::TsTypeRef(ty_ref) = ty {
        if let swc_ecma_ast::TsEntityName::Ident(id) = &ty_ref.type_name {
            let ref_name = id.sym.as_str();
            if let Some(concrete) = substitution.get(ref_name) {
                return Ok(concrete.clone());
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
                    )?;
                    match ref_name {
                        "Array" => return Ok(HirType::Array(Box::new(resolved_elem))),
                        "Promise" => return Ok(HirType::Promise(Box::new(resolved_elem))),
                        _ => {}
                    }
                }
            }
        }
        return lower_ts_type(ty, interfaces, generic_interfaces);
    }

    match ty {
        TsType::TsArrayType(arr) => Ok(HirType::Array(Box::new(resolve_ts_type_with_substitution(
            &arr.elem_type,
            substitution,
            interfaces,
            generic_interfaces,
            in_progress,
        )?))),
        TsType::TsTypeLit(type_lit) => {
            let fields = type_lit
                .members
                .iter()
                .map(|member| {
                    let TsTypeElement::TsPropertySignature(prop) = member else {
                        return Err(
                            "only plain properties are supported in object type literals (no methods/index signatures)"
                                .to_string(),
                        );
                    };
                    let name = match prop.key.as_ref() {
                        Expr::Ident(ident) => ident.sym.to_string(),
                        _ => return Err("unsupported object type literal key".to_string()),
                    };
                    let ann = prop.type_ann.as_ref().ok_or_else(|| {
                        format!("field `{name}` needs an explicit type annotation")
                    })?;
                    let field_ty = resolve_ts_type_with_substitution(
                        &ann.type_ann,
                        substitution,
                        interfaces,
                        generic_interfaces,
                        in_progress,
                    )?;
                    Ok((name, field_ty))
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok(HirType::Object(fields))
        }
        other => lower_ts_type(other, interfaces, generic_interfaces),
    }
}

/// An assignment target, resolved down to one of the three shapes
/// `lower_assign`/`lower_update` support.
enum Target {
    Var(Symbol),
    Index(HirExpr, HirExpr),
    Prop(HirExpr, HirType, Symbol),
}

fn target_to_read_expr(target: &Target) -> HirExpr {
    match target {
        Target::Var(name) => HirExpr::Var(name.clone()),
        Target::Index(arr, idx) => HirExpr::Index(Box::new(arr.clone()), Box::new(idx.clone())),
        Target::Prop(obj, ty, field) => {
            HirExpr::PropAccess(Box::new(obj.clone()), ty.clone(), field.clone())
        }
    }
}

fn build_assign(target: Target, value: HirExpr) -> HirExpr {
    match target {
        Target::Var(name) => HirExpr::Assign(name, Box::new(value)),
        Target::Index(arr, idx) => {
            HirExpr::IndexAssign(Box::new(arr), Box::new(idx), Box::new(value))
        }
        Target::Prop(obj, ty, field) => {
            HirExpr::PropAssign(Box::new(obj), ty, field, Box::new(value))
        }
    }
}

fn compound_op(op: AssignOp) -> Option<BinOp> {
    match op {
        AssignOp::AddAssign => Some(BinOp::Add),
        AssignOp::SubAssign => Some(BinOp::Sub),
        AssignOp::MulAssign => Some(BinOp::Mul),
        AssignOp::DivAssign => Some(BinOp::Div),
        _ => None,
    }
}

fn lower_bin_op(op: BinaryOp) -> Result<BinOp, String> {
    match op {
        BinaryOp::Add => Ok(BinOp::Add),
        BinaryOp::Sub => Ok(BinOp::Sub),
        BinaryOp::Mul => Ok(BinOp::Mul),
        BinaryOp::Div => Ok(BinOp::Div),
        BinaryOp::Lt => Ok(BinOp::Lt),
        BinaryOp::Gt => Ok(BinOp::Gt),
        BinaryOp::EqEqEq => Ok(BinOp::EqEqEq),
        other => Err(format!("unsupported binary operator {other:?}")),
    }
}

/// Lowers one function body. Holds the type scope (params + `let`s seen so
/// far) and the whole module's function signatures, needed to resolve
/// member access (`arr.length` vs `obj.field`) and to type-check/reorder
/// object literals against their declared shape.
struct FnLowerer<'a> {
    scope: HashMap<Symbol, HirType>,
    signatures: &'a HashMap<Symbol, FnSignature>,
    interfaces: &'a HashMap<Symbol, HirType>,
    generic_interfaces: &'a GenericInterfaces<'a>,
    ret_type: HirType,
}

impl<'a> FnLowerer<'a> {
    fn new(
        signatures: &'a HashMap<Symbol, FnSignature>,
        interfaces: &'a HashMap<Symbol, HirType>,
        generic_interfaces: &'a GenericInterfaces<'a>,
        ret_type: HirType,
    ) -> Self {
        Self {
            scope: HashMap::new(),
            signatures,
            interfaces,
            generic_interfaces,
            ret_type,
        }
    }

    fn lower_stmts(&mut self, stmts: &[Stmt]) -> Result<Vec<HirStmt>, String> {
        let mut out = Vec::new();
        for stmt in stmts {
            out.extend(self.lower_stmt_seq(stmt)?);
        }
        Ok(out)
    }

    /// Normalizes a `for`/`while`/`if` body, which SWC represents as a
    /// single `Stmt` (either a `{ ... }` block or one bare statement), into
    /// a flat HIR statement list.
    fn lower_body(&mut self, stmt: &Stmt) -> Result<Vec<HirStmt>, String> {
        match stmt {
            Stmt::Block(block) => self.lower_stmts(&block.stmts),
            other => self.lower_stmt_seq(other),
        }
    }

    fn lower_stmt_seq(&mut self, stmt: &Stmt) -> Result<Vec<HirStmt>, String> {
        match stmt {
            Stmt::Return(ret) => {
                let value = match &ret.arg {
                    Some(arg) => {
                        let value = self.lower_expr(arg)?;
                        Some(self.coerce_to_declared(&self.ret_type.clone(), value)?)
                    }
                    None => None,
                };
                Ok(vec![HirStmt::Return(value)])
            }
            Stmt::Expr(expr_stmt) => Ok(vec![HirStmt::Expr(self.lower_expr(&expr_stmt.expr)?)]),
            Stmt::Block(block) => self.lower_stmts(&block.stmts),
            Stmt::Decl(Decl::Var(var_decl)) => self.lower_var_decl(var_decl),

            Stmt::If(if_stmt) => {
                let cond = self.lower_expr(&if_stmt.test)?;
                let then_branch = self.lower_body(&if_stmt.cons)?;
                let else_branch = match &if_stmt.alt {
                    Some(alt) => self.lower_body(alt)?,
                    None => Vec::new(),
                };
                Ok(vec![HirStmt::If(cond, then_branch, else_branch)])
            }

            Stmt::While(while_stmt) => {
                let cond = self.lower_expr(&while_stmt.test)?;
                let body = self.lower_body(&while_stmt.body)?;
                Ok(vec![HirStmt::While(cond, body)])
            }

            Stmt::For(for_stmt) => {
                let mut out = Vec::new();
                if let Some(init) = &for_stmt.init {
                    match init {
                        VarDeclOrExpr::VarDecl(var_decl) => out.extend(self.lower_var_decl(var_decl)?),
                        VarDeclOrExpr::Expr(expr) => out.push(HirStmt::Expr(self.lower_expr(expr)?)),
                    }
                }

                let cond = match &for_stmt.test {
                    Some(test) => self.lower_expr(test)?,
                    None => HirExpr::Lit(HirLit::Bool(true)),
                };

                let mut body = self.lower_body(&for_stmt.body)?;
                if let Some(update) = &for_stmt.update {
                    body.push(HirStmt::Expr(self.lower_expr(update)?));
                }

                out.push(HirStmt::While(cond, body));
                Ok(out)
            }

            Stmt::Throw(throw_stmt) => Ok(vec![HirStmt::Throw(self.lower_expr(&throw_stmt.arg)?)]),

            Stmt::Try(try_stmt) => {
                if try_stmt.finalizer.is_some() {
                    return Err("`finally` is not supported yet".into());
                }
                let handler = try_stmt
                    .handler
                    .as_ref()
                    .ok_or("`try` without `catch` is not supported yet")?;
                let catch_name = match &handler.param {
                    Some(Pat::Ident(binding)) => binding.id.sym.to_string(),
                    Some(_) => {
                        return Err("only a simple identifier catch binding is supported".into())
                    }
                    None => "_".to_string(),
                };
                let body = self.lower_stmts(&try_stmt.block.stmts)?;
                self.scope.insert(catch_name.clone(), HirType::Str);
                let catch_body = self.lower_stmts(&handler.body.stmts)?;
                Ok(vec![HirStmt::Try(body, catch_name, catch_body)])
            }

            other => Err(format!(
                "unsupported statement {other:?} (Phase 0/1/2 support return/expr/let/if/while/for/throw/try)"
            )),
        }
    }

    fn lower_var_decl(&mut self, var_decl: &VarDecl) -> Result<Vec<HirStmt>, String> {
        var_decl
            .decls
            .iter()
            .map(|decl| {
                let Pat::Ident(binding) = &decl.name else {
                    return Err(
                        "only simple identifier bindings are supported in `let`/`const`".into(),
                    );
                };
                let name = binding.id.sym.to_string();
                let init = decl
                    .init
                    .as_deref()
                    .ok_or_else(|| format!("`{name}` needs an initializer"))?;
                let value = self.lower_expr(init)?;

                let ty = match &binding.type_ann {
                    Some(ann) => lower_ts_type(&ann.type_ann, self.interfaces, self.generic_interfaces)?,
                    None => self.infer_expr_type(&value).map_err(|e| {
                        format!(
                            "cannot infer the type of `{name}`: {e} \
                             (add an explicit type annotation)"
                        )
                    })?,
                };
                let value = self.coerce_to_declared(&ty, value)?;

                self.scope.insert(name.clone(), ty.clone());
                Ok(HirStmt::Let(name, ty, value))
            })
            .collect()
    }

    /// If `declared` is an object type and `value` is an object literal,
    /// reorders the literal's fields to match the declared field order and
    /// checks each field's type -- so codegen only ever has to deal with
    /// one canonical field order (the declared one), never the literal's
    /// source order. A no-op for every other combination.
    fn coerce_to_declared(&self, declared: &HirType, value: HirExpr) -> Result<HirExpr, String> {
        let (HirType::Object(declared_fields), HirExpr::ObjectLit(lit_fields)) = (declared, &value)
        else {
            return Ok(value);
        };

        if declared_fields.len() != lit_fields.len() {
            return Err(format!(
                "object literal has {} field(s), expected {} for this type",
                lit_fields.len(),
                declared_fields.len()
            ));
        }

        let reordered = declared_fields
            .iter()
            .map(|(name, expected_ty)| {
                let (_, field_value) = lit_fields
                    .iter()
                    .find(|(n, _)| n == name)
                    .ok_or_else(|| format!("object literal is missing field `{name}`"))?;
                let actual_ty = self.infer_expr_type(field_value)?;
                if actual_ty != *expected_ty {
                    return Err(format!(
                        "field `{name}` has type {actual_ty:?}, expected {expected_ty:?}"
                    ));
                }
                Ok((name.clone(), field_value.clone()))
            })
            .collect::<Result<Vec<_>, String>>()?;

        Ok(HirExpr::ObjectLit(reordered))
    }

    /// Minimal type inference, used only to resolve member access
    /// (`obj.field`) and to check/reorder object literals -- not a general
    /// type checker. Every expression shape lowering can currently produce
    /// is covered; anything else is a lowering bug, not user error.
    fn infer_expr_type(&self, expr: &HirExpr) -> Result<HirType, String> {
        match expr {
            HirExpr::Lit(HirLit::F64(_)) => Ok(HirType::F64),
            HirExpr::Lit(HirLit::Str(_)) => Ok(HirType::Str),
            HirExpr::Lit(HirLit::Bool(_)) => Ok(HirType::Bool),
            HirExpr::Var(name) | HirExpr::Assign(name, _) => self
                .scope
                .get(name)
                .cloned()
                .ok_or_else(|| format!("unknown variable `{name}`")),
            HirExpr::BinOp(op, ..) => match op {
                BinOp::Lt | BinOp::Gt | BinOp::EqEqEq => Ok(HirType::Bool),
                _ => Ok(HirType::F64),
            },
            HirExpr::Call(callee, _) => {
                let HirExpr::Var(name) = callee.as_ref() else {
                    return Err("cannot infer the type of a call through a non-name callee".into());
                };
                match name.as_str() {
                    "console.log" => return Ok(HirType::F64),
                    "fetch" => return Ok(HirType::Str),
                    "JSON.parse" => return Ok(HirType::Json),
                    "JSON.stringify" => return Ok(HirType::Str),
                    _ => {}
                }
                self.signatures
                    .get(name)
                    .map(|sig| sig.ret.clone())
                    .ok_or_else(|| format!("call to unknown function `{name}`"))
            }
            HirExpr::ArrayLit(_) => Ok(HirType::Array(Box::new(HirType::F64))),
            HirExpr::Index(arr, _) => match self.infer_expr_type(arr)? {
                HirType::Array(elem) => Ok(*elem),
                other => Err(format!("cannot index into a value of type {other:?}")),
            },
            HirExpr::IndexAssign(_, _, value) => self.infer_expr_type(value),
            HirExpr::ArrayLen(_) => Ok(HirType::F64),
            HirExpr::EnvVar(_) => Ok(HirType::Str),
            HirExpr::ObjectLit(fields) => {
                let fields = fields
                    .iter()
                    .map(|(name, value)| Ok((name.clone(), self.infer_expr_type(value)?)))
                    .collect::<Result<Vec<_>, String>>()?;
                Ok(HirType::Object(fields))
            }
            HirExpr::PropAccess(_, object_ty, field) => match object_ty {
                HirType::Object(fields) => fields
                    .iter()
                    .find(|(name, _)| name == field)
                    .map(|(_, ty)| ty.clone())
                    .ok_or_else(|| format!("object has no field `{field}`")),
                other => Err(format!("cannot access `.{field}` on a value of type {other:?}")),
            },
            HirExpr::PropAssign(_, _, _, value) => self.infer_expr_type(value),
            HirExpr::JsonGet(_, _) | HirExpr::JsonIndex(_, _) => Ok(HirType::Json),
            HirExpr::JsonAsNumber(_) => Ok(HirType::F64),
            HirExpr::JsonAsString(_) => Ok(HirType::Str),
            HirExpr::JsonAsBool(_) => Ok(HirType::Bool),
            HirExpr::FfiCall(sig, _) => Ok(sig.ret.clone()),
            // V1 erases Promise entirely: a call's return type in
            // `self.signatures` is already unwrapped for async functions,
            // so `await` is transparent here too.
            HirExpr::Await(inner) => self.infer_expr_type(inner),
            other => Err(format!(
                "cannot infer the type of {other:?} (needs an explicit type annotation)"
            )),
        }
    }

    fn lower_expr(&mut self, expr: &Expr) -> Result<HirExpr, String> {
        match expr {
            Expr::Lit(Lit::Num(n)) => Ok(HirExpr::Lit(HirLit::F64(n.value))),
            Expr::Lit(Lit::Str(s)) => Ok(HirExpr::Lit(HirLit::Str(
                s.value.to_string_lossy().into_owned(),
            ))),
            Expr::Lit(Lit::Bool(b)) => Ok(HirExpr::Lit(HirLit::Bool(b.value))),
            Expr::Ident(ident) => Ok(HirExpr::Var(ident.sym.to_string())),
            Expr::Paren(paren) => self.lower_expr(&paren.expr),

            Expr::Bin(bin) => {
                let op = lower_bin_op(bin.op)?;
                let lhs = self.lower_expr(&bin.left)?;
                let rhs = self.lower_expr(&bin.right)?;
                Ok(HirExpr::BinOp(op, Box::new(lhs), Box::new(rhs)))
            }

            Expr::Call(call) => self.lower_call(call),

            Expr::Array(array_lit) => {
                let elems = array_lit
                    .elems
                    .iter()
                    .map(|elem| match elem {
                        Some(e) if e.spread.is_none() => self.lower_expr(&e.expr),
                        Some(_) => {
                            Err("spread elements are not supported in array literals".to_string())
                        }
                        None => Err("elisions are not supported in array literals".to_string()),
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(HirExpr::ArrayLit(elems))
            }

            Expr::Object(obj_lit) => self.lower_object_lit(obj_lit),

            Expr::Member(member) => self.lower_member_read(member),

            Expr::Assign(assign) => self.lower_assign(assign),

            Expr::Update(update) => self.lower_update(update),

            Expr::Await(await_expr) => {
                Ok(HirExpr::Await(Box::new(self.lower_expr(&await_expr.arg)?)))
            }

            other => Err(format!(
                "unsupported expression {other:?} (Phase 0/1/2 support literals, identifiers, binary ops, calls, arrays, objects, member access, assignment, ++/--)"
            )),
        }
    }

    fn lower_object_lit(&mut self, obj_lit: &SwcObjectLit) -> Result<HirExpr, String> {
        let fields = obj_lit
            .props
            .iter()
            .map(|prop| {
                let PropOrSpread::Prop(prop) = prop else {
                    return Err("spread properties are not supported in object literals".to_string());
                };
                let Prop::KeyValue(KeyValueProp { key, value }) = prop.as_ref() else {
                    return Err(
                        "only `key: value` object literal properties are supported".to_string(),
                    );
                };
                let name = match key {
                    PropName::Ident(ident) => ident.sym.to_string(),
                    PropName::Str(s) => s.value.to_string_lossy().into_owned(),
                    _ => return Err("unsupported object literal key".to_string()),
                };
                Ok((name, self.lower_expr(value)?))
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(HirExpr::ObjectLit(fields))
    }

    fn lower_member_read(&mut self, member: &MemberExpr) -> Result<HirExpr, String> {
        // `process.env.NAME` -- checked before the general cases since it's
        // a fixed two-level member chain, not a general property access.
        if let MemberProp::Ident(name_prop) = &member.prop {
            if let Expr::Member(inner) = member.obj.as_ref() {
                if let (Expr::Ident(obj), MemberProp::Ident(env_prop)) =
                    (inner.obj.as_ref(), &inner.prop)
                {
                    if obj.sym == *"process" && env_prop.sym == *"env" {
                        return Ok(HirExpr::EnvVar(name_prop.sym.to_string()));
                    }
                }
            }
        }

        match &member.prop {
            MemberProp::Computed(computed) => {
                let obj = self.lower_expr(&member.obj)?;
                let obj_ty = self.infer_expr_type(&obj)?;
                let index = self.lower_expr(&computed.expr)?;
                match obj_ty {
                    HirType::Array(_) => Ok(HirExpr::Index(Box::new(obj), Box::new(index))),
                    HirType::Json => Ok(HirExpr::JsonIndex(Box::new(obj), Box::new(index))),
                    other => Err(format!("cannot index into a value of type {other:?}")),
                }
            }
            MemberProp::Ident(prop) => {
                let obj = self.lower_expr(&member.obj)?;
                let obj_ty = self.infer_expr_type(&obj)?;
                match &obj_ty {
                    HirType::Array(_) if prop.sym == *"length" => {
                        Ok(HirExpr::ArrayLen(Box::new(obj)))
                    }
                    HirType::Object(fields) => {
                        if fields.iter().any(|(name, _)| name == prop.sym.as_str()) {
                            Ok(HirExpr::PropAccess(Box::new(obj), obj_ty.clone(), prop.sym.to_string()))
                        } else {
                            Err(format!("object has no field `{}`", prop.sym))
                        }
                    }
                    HirType::Json => Ok(HirExpr::JsonGet(Box::new(obj), prop.sym.to_string())),
                    other => Err(format!(
                        "unsupported property access `.{}` on a value of type {other:?}",
                        prop.sym
                    )),
                }
            }
            _ => Err("unsupported property access".into()),
        }
    }

    /// `array[index]` used as an assignment/`++`/`--` target. Checks
    /// `array` is actually a `number[]` first -- `Str`/`Object`/`Json` all
    /// share the same pointer representation as arrays at the LLVM level,
    /// so building a `Target::Index` for one of those would silently
    /// misinterpret its bytes as array elements at runtime instead of
    /// failing to compile.
    fn lower_index_target(
        &mut self,
        member: &MemberExpr,
        computed: &ComputedPropName,
    ) -> Result<Target, String> {
        let obj = self.lower_expr(&member.obj)?;
        let obj_ty = self.infer_expr_type(&obj)?;
        if obj_ty != HirType::Array(Box::new(HirType::F64)) {
            return Err(format!(
                "cannot assign to a computed index on a value of type {obj_ty:?} (only number[] supports this)"
            ));
        }
        let index = self.lower_expr(&computed.expr)?;
        Ok(Target::Index(obj, index))
    }

    fn lower_assign_target(&mut self, target: &AssignTarget) -> Result<Target, String> {
        let AssignTarget::Simple(simple) = target else {
            return Err("destructuring assignment targets are not supported".into());
        };
        match simple {
            SimpleAssignTarget::Ident(binding) => Ok(Target::Var(binding.id.sym.to_string())),
            SimpleAssignTarget::Member(member) => match &member.prop {
                MemberProp::Computed(computed) => self.lower_index_target(member, computed),
                MemberProp::Ident(prop) => {
                    let obj = self.lower_expr(&member.obj)?;
                    let obj_ty = self.infer_expr_type(&obj)?;
                    match &obj_ty {
                        HirType::Object(fields) if fields.iter().any(|(n, _)| n == prop.sym.as_str()) => {
                            Ok(Target::Prop(obj, obj_ty.clone(), prop.sym.to_string()))
                        }
                        other => Err(format!(
                            "cannot assign to `.{}` on a value of type {other:?}",
                            prop.sym
                        )),
                    }
                }
                _ => Err("only `arr[i] = ...` / `obj.field = ...` member assignment is supported".into()),
            },
            _ => Err("unsupported assignment target".into()),
        }
    }

    fn lower_assign(&mut self, assign: &swc_ecma_ast::AssignExpr) -> Result<HirExpr, String> {
        let target = self.lower_assign_target(&assign.left)?;
        let rhs = self.lower_expr(&assign.right)?;

        let value = if assign.op == AssignOp::Assign {
            rhs
        } else if let Some(op) = compound_op(assign.op) {
            HirExpr::BinOp(op, Box::new(target_to_read_expr(&target)), Box::new(rhs))
        } else {
            return Err(format!(
                "unsupported compound assignment operator {:?}",
                assign.op
            ));
        };

        // Reorder/typecheck an object literal against the target's
        // declared shape, same as a `let`/call-argument assignment --
        // needed now that a field can itself be an object (`p.corner =
        // { y: 2, x: 1 }`), not just a plain variable.
        let value = match &target {
            Target::Var(name) => match self.scope.get(name).cloned() {
                Some(ty) => self.coerce_to_declared(&ty, value)?,
                None => value,
            },
            Target::Prop(_, HirType::Object(fields), field) => {
                match fields.iter().find(|(n, _)| n == field) {
                    Some((_, ty)) => self.coerce_to_declared(&ty.clone(), value)?,
                    None => value,
                }
            }
            _ => value,
        };

        Ok(build_assign(target, value))
    }

    fn lower_update(&mut self, update: &swc_ecma_ast::UpdateExpr) -> Result<HirExpr, String> {
        // Phase 1/2 always yield the *new* value (prefix semantics), even
        // for postfix `i++`/`i--`. This only matters when the expression's
        // value is used, which doesn't happen in the
        // `for (...; ...; i++)` / bare `i++;` forms this is meant to support.
        let target = match update.arg.as_ref() {
            Expr::Ident(ident) => Target::Var(ident.sym.to_string()),
            Expr::Member(member) => match &member.prop {
                MemberProp::Computed(computed) => self.lower_index_target(member, computed)?,
                _ => return Err("unsupported ++/-- target".into()),
            },
            _ => return Err("unsupported ++/-- target".into()),
        };

        let op = match update.op {
            UpdateOp::PlusPlus => BinOp::Add,
            UpdateOp::MinusMinus => BinOp::Sub,
        };
        let one = HirExpr::Lit(HirLit::F64(1.0));
        let value = HirExpr::BinOp(op, Box::new(target_to_read_expr(&target)), Box::new(one));
        Ok(build_assign(target, value))
    }

    fn lower_call(&mut self, call: &CallExpr) -> Result<HirExpr, String> {
        let Callee::Expr(callee_expr) = &call.callee else {
            return Err("unsupported callee (super/import calls not supported)".into());
        };

        let callee_name = match callee_expr.as_ref() {
            Expr::Ident(ident) => ident.sym.to_string(),
            // `console.log` has no dedicated HIR node; it's encoded as a
            // call to the synthetic name "console.log" and codegen
            // special-cases it.
            Expr::Member(member) => {
                let Expr::Ident(obj) = member.obj.as_ref() else {
                    return Err("unsupported member call target".into());
                };
                let MemberProp::Ident(prop) = &member.prop else {
                    return Err("unsupported member call property".into());
                };
                format!("{}.{}", obj.sym, prop.sym)
            }
            _ => {
                return Err(
                    "unsupported call target (only plain identifiers and console.log are supported)"
                        .into(),
                )
            }
        };

        // `Number`/`String`/`Boolean` convert a `Json` leaf to a concrete
        // value. Unlike `console.log` (whose codegen can disambiguate its
        // argument by LLVM value shape -- f64 vs. pointer), `Str`/`Array`/
        // `Object`/`Json` all share the same pointer representation, so
        // this has to be resolved here at lowering time using the
        // argument's inferred type, not deferred to codegen.
        if matches!(callee_name.as_str(), "Number" | "String" | "Boolean") {
            let [arg] = call.args.as_slice() else {
                return Err(format!("`{callee_name}` expects exactly one argument"));
            };
            if arg.spread.is_some() {
                return Err("spread arguments are not supported".into());
            }
            let value = self.lower_expr(&arg.expr)?;
            let ty = self.infer_expr_type(&value)?;
            if ty != HirType::Json {
                return Err(format!(
                    "`{callee_name}(...)` is only supported on a JSON value for now (got {ty:?})"
                ));
            }
            return Ok(match callee_name.as_str() {
                "Number" => HirExpr::JsonAsNumber(Box::new(value)),
                "String" => HirExpr::JsonAsString(Box::new(value)),
                _ => HirExpr::JsonAsBool(Box::new(value)),
            });
        }

        let signature = self.signatures.get(&callee_name).cloned();
        let param_types = signature.as_ref().map(|sig| sig.params.clone());

        let args = call
            .args
            .iter()
            .enumerate()
            .map(|(i, arg)| {
                if arg.spread.is_some() {
                    return Err("spread arguments are not supported".to_string());
                }
                let value = self.lower_expr(&arg.expr)?;
                match param_types.as_ref().and_then(|p| p.get(i)) {
                    Some(declared) => self.coerce_to_declared(declared, value),
                    None => Ok(value),
                }
            })
            .collect::<Result<Vec<_>, _>>()?;

        if let Some(sig) = signature.filter(|sig| sig.is_extern) {
            let ffi_signature = FfiSignature {
                symbol: callee_name,
                params: sig.params,
                ret: sig.ret,
            };
            return Ok(HirExpr::FfiCall(ffi_signature, args));
        }

        Ok(HirExpr::Call(Box::new(HirExpr::Var(callee_name)), args))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HirType;

    fn lower(source: &str) -> HirProgram {
        let module = thaw_parser::parse_typescript(source).expect("parse error");
        lower_module(&module).expect("lowering error")
    }

    #[test]
    fn lowers_typed_function_with_binary_op() {
        let program = lower("function add(a: number, b: number): number { return a + b; }");
        assert_eq!(program.functions.len(), 1);
        let f = &program.functions[0];
        assert_eq!(f.name, "add");
        assert_eq!(
            f.params,
            vec![
                HirParam {
                    name: "a".into(),
                    ty: HirType::F64
                },
                HirParam {
                    name: "b".into(),
                    ty: HirType::F64
                },
            ]
        );
        assert_eq!(f.ret, HirType::F64);
        assert_eq!(
            f.body,
            vec![HirStmt::Return(Some(HirExpr::BinOp(
                BinOp::Add,
                Box::new(HirExpr::Var("a".into())),
                Box::new(HirExpr::Var("b".into())),
            )))]
        );
    }

    #[test]
    fn lowers_console_log_of_a_string_literal() {
        let program = lower(r#"function main(): void { console.log("Hello, Thaw!"); }"#);
        let f = &program.functions[0];
        assert_eq!(
            f.body,
            vec![HirStmt::Expr(HirExpr::Call(
                Box::new(HirExpr::Var("console.log".into())),
                vec![HirExpr::Lit(HirLit::Str("Hello, Thaw!".into()))],
            ))]
        );
    }

    #[test]
    fn rejects_missing_parameter_type_annotation() {
        let module = thaw_parser::parse_typescript("function f(a) { return a; }").unwrap();
        assert!(lower_module(&module).is_err());
    }

    #[test]
    fn desugars_classic_for_loop_into_let_and_while() {
        let program = lower(
            "function main(): void { for (let i = 0; i < 10; i = i + 1) { console.log(i); } }",
        );
        let f = &program.functions[0];
        assert_eq!(f.body.len(), 2);
        assert!(matches!(f.body[0], HirStmt::Let(ref n, HirType::F64, _) if n == "i"));
        let HirStmt::While(ref cond, ref body) = f.body[1] else {
            panic!("expected desugared while loop, got {:?}", f.body[1]);
        };
        assert_eq!(
            *cond,
            HirExpr::BinOp(
                BinOp::Lt,
                Box::new(HirExpr::Var("i".into())),
                Box::new(HirExpr::Lit(HirLit::F64(10.0))),
            )
        );
        // console.log(i) + the `i = i + 1` update appended to the body.
        assert_eq!(body.len(), 2);
        assert!(matches!(body[1], HirStmt::Expr(HirExpr::Assign(ref n, _)) if n == "i"));
    }

    #[test]
    fn lowers_array_literal_index_and_length() {
        let program = lower(
            "function main(): void { const xs: number[] = [1, 2, 3]; console.log(xs[1]); console.log(xs.length); }",
        );
        let f = &program.functions[0];
        assert_eq!(
            f.body[0],
            HirStmt::Let(
                "xs".into(),
                HirType::Array(Box::new(HirType::F64)),
                HirExpr::ArrayLit(vec![
                    HirExpr::Lit(HirLit::F64(1.0)),
                    HirExpr::Lit(HirLit::F64(2.0)),
                    HirExpr::Lit(HirLit::F64(3.0)),
                ]),
            )
        );
        assert_eq!(
            f.body[1],
            HirStmt::Expr(HirExpr::Call(
                Box::new(HirExpr::Var("console.log".into())),
                vec![HirExpr::Index(
                    Box::new(HirExpr::Var("xs".into())),
                    Box::new(HirExpr::Lit(HirLit::F64(1.0))),
                )],
            ))
        );
        assert_eq!(
            f.body[2],
            HirStmt::Expr(HirExpr::Call(
                Box::new(HirExpr::Var("console.log".into())),
                vec![HirExpr::ArrayLen(Box::new(HirExpr::Var("xs".into())))],
            ))
        );
    }

    #[test]
    fn lowers_process_env_access() {
        let program = lower(r#"function main(): void { console.log(process.env.STAGE); }"#);
        let f = &program.functions[0];
        assert_eq!(
            f.body,
            vec![HirStmt::Expr(HirExpr::Call(
                Box::new(HirExpr::Var("console.log".into())),
                vec![HirExpr::EnvVar("STAGE".into())],
            ))]
        );
    }

    #[test]
    fn lowers_try_catch() {
        let program = lower(
            r#"function main(): void {
                try {
                    throw "boom";
                } catch (e) {
                    console.log(e);
                }
            }"#,
        );
        let f = &program.functions[0];
        assert_eq!(
            f.body,
            vec![HirStmt::Try(
                vec![HirStmt::Throw(HirExpr::Lit(HirLit::Str("boom".into())))],
                "e".into(),
                vec![HirStmt::Expr(HirExpr::Call(
                    Box::new(HirExpr::Var("console.log".into())),
                    vec![HirExpr::Var("e".into())],
                ))],
            )]
        );
    }

    #[test]
    fn lowers_object_literal_field_access_and_mutation() {
        let program = lower(
            r#"function main(): void {
                const p: { x: number; y: number } = { x: 1, y: 2 };
                console.log(p.x);
                p.y = p.y + 1;
            }"#,
        );
        let f = &program.functions[0];
        let obj_ty = HirType::Object(vec![("x".into(), HirType::F64), ("y".into(), HirType::F64)]);

        assert_eq!(
            f.body[0],
            HirStmt::Let(
                "p".into(),
                obj_ty.clone(),
                HirExpr::ObjectLit(vec![
                    ("x".into(), HirExpr::Lit(HirLit::F64(1.0))),
                    ("y".into(), HirExpr::Lit(HirLit::F64(2.0))),
                ]),
            )
        );
        assert_eq!(
            f.body[1],
            HirStmt::Expr(HirExpr::Call(
                Box::new(HirExpr::Var("console.log".into())),
                vec![HirExpr::PropAccess(
                    Box::new(HirExpr::Var("p".into())),
                    obj_ty.clone(),
                    "x".into(),
                )],
            ))
        );
        assert_eq!(
            f.body[2],
            HirStmt::Expr(HirExpr::PropAssign(
                Box::new(HirExpr::Var("p".into())),
                obj_ty.clone(),
                "y".into(),
                Box::new(HirExpr::BinOp(
                    BinOp::Add,
                    Box::new(HirExpr::PropAccess(
                        Box::new(HirExpr::Var("p".into())),
                        obj_ty,
                        "y".into(),
                    )),
                    Box::new(HirExpr::Lit(HirLit::F64(1.0))),
                )),
            ))
        );
    }

    #[test]
    fn reorders_object_literal_fields_to_match_the_declared_type() {
        // Written as {y, x} but the declared type says {x, y} -- lowering
        // should reorder so codegen only ever sees the declared order.
        let program = lower(
            r#"function main(): void {
                const p: { x: number; y: number } = { y: 2, x: 1 };
                console.log(p.x);
            }"#,
        );
        let f = &program.functions[0];
        assert_eq!(
            f.body[0],
            HirStmt::Let(
                "p".into(),
                HirType::Object(vec![("x".into(), HirType::F64), ("y".into(), HirType::F64)]),
                HirExpr::ObjectLit(vec![
                    ("x".into(), HirExpr::Lit(HirLit::F64(1.0))),
                    ("y".into(), HirExpr::Lit(HirLit::F64(2.0))),
                ]),
            )
        );
    }

    #[test]
    fn rejects_object_literal_with_wrong_field_type() {
        let module = thaw_parser::parse_typescript(
            r#"function main(): void {
                const p: { x: number } = { x: "not a number" };
            }"#,
        )
        .unwrap();
        assert!(lower_module(&module).is_err());
    }

    #[test]
    fn coerces_object_literal_argument_to_the_parameter_shape() {
        let program = lower(
            r#"function dist(p: { x: number; y: number }): number {
                return p.x + p.y;
            }
            function main(): void {
                console.log(dist({ y: 2, x: 1 }));
            }"#,
        );
        let main = &program.functions[1];
        let HirStmt::Expr(HirExpr::Call(_, args)) = &main.body[0] else {
            panic!("expected a console.log call, got {:?}", main.body[0]);
        };
        let [console_arg] = args.as_slice() else {
            panic!("expected one argument to console.log");
        };
        let HirExpr::Call(_, dist_args) = console_arg else {
            panic!("expected a call to `dist`, got {console_arg:?}");
        };
        assert_eq!(
            dist_args,
            &vec![HirExpr::ObjectLit(vec![
                ("x".into(), HirExpr::Lit(HirLit::F64(1.0))),
                ("y".into(), HirExpr::Lit(HirLit::F64(2.0))),
            ])]
        );
    }

    #[test]
    fn lowers_async_function_unwrapping_promise_and_await() {
        let program = lower(
            r#"async function fetchStage(): Promise<string> {
                const s: string = process.env.STAGE;
                return s;
            }
            async function main(): Promise<void> {
                const stage: string = await fetchStage();
                console.log(stage);
            }"#,
        );

        let fetch_stage = &program.functions[0];
        assert!(fetch_stage.is_async);
        // `Promise<string>` is unwrapped to `string` -- Promise never
        // appears in the compiled HIR (V1 design).
        assert_eq!(fetch_stage.ret, HirType::Str);

        let main = &program.functions[1];
        assert!(main.is_async);
        assert_eq!(main.ret, HirType::Void);
        assert_eq!(
            main.body[0],
            HirStmt::Let(
                "stage".into(),
                HirType::Str,
                HirExpr::Await(Box::new(HirExpr::Call(
                    Box::new(HirExpr::Var("fetchStage".into())),
                    vec![],
                ))),
            )
        );
    }

    #[test]
    fn rejects_async_function_not_declared_as_returning_promise() {
        let module = thaw_parser::parse_typescript(
            "async function f(): number { return 1; }",
        )
        .unwrap();
        let err = lower_module(&module).unwrap_err();
        assert!(err.contains("Promise"), "unexpected error: {err}");
    }

    #[test]
    fn lowers_fetch_and_json_parse_field_access() {
        let program = lower(
            r#"function main(): void {
                const text: string = fetch("https://example.com/api");
                const data = JSON.parse(text);
                const name: string = String(data.name);
                const count: number = Number(data.items[0]);
                console.log(name);
                console.log(count);
            }"#,
        );
        let f = &program.functions[0];

        assert_eq!(
            f.body[0],
            HirStmt::Let(
                "text".into(),
                HirType::Str,
                HirExpr::Call(
                    Box::new(HirExpr::Var("fetch".into())),
                    vec![HirExpr::Lit(HirLit::Str("https://example.com/api".into()))],
                ),
            )
        );
        assert_eq!(
            f.body[1],
            HirStmt::Let(
                "data".into(),
                HirType::Json,
                HirExpr::Call(
                    Box::new(HirExpr::Var("JSON.parse".into())),
                    vec![HirExpr::Var("text".into())],
                ),
            )
        );
        assert_eq!(
            f.body[2],
            HirStmt::Let(
                "name".into(),
                HirType::Str,
                HirExpr::JsonAsString(Box::new(HirExpr::JsonGet(
                    Box::new(HirExpr::Var("data".into())),
                    "name".into(),
                ))),
            )
        );
        assert_eq!(
            f.body[3],
            HirStmt::Let(
                "count".into(),
                HirType::F64,
                HirExpr::JsonAsNumber(Box::new(HirExpr::JsonIndex(
                    Box::new(HirExpr::JsonGet(
                        Box::new(HirExpr::Var("data".into())),
                        "items".into(),
                    )),
                    Box::new(HirExpr::Lit(HirLit::F64(0.0))),
                ))),
            )
        );
    }

    #[test]
    fn rejects_number_conversion_on_a_non_json_value() {
        let module = thaw_parser::parse_typescript(
            "function main(): void { const x: number = Number(1); }",
        )
        .unwrap();
        let err = lower_module(&module).unwrap_err();
        assert!(err.contains("JSON"), "unexpected error: {err}");
    }

    #[test]
    fn rejects_indexed_assignment_into_a_non_array() {
        let module = thaw_parser::parse_typescript(
            r#"function main(): void {
                const data = JSON.parse("[]");
                data[0] = 1;
            }"#,
        )
        .unwrap();
        assert!(lower_module(&module).is_err());
    }

    #[test]
    fn lowers_ambient_declaration_call_to_ffi_call() {
        let program = lower(
            r#"declare function native_add(a: number, b: number): number;

            function main(): void {
                console.log(native_add(2, 3));
            }"#,
        );

        assert_eq!(program.functions.len(), 1, "the ambient decl has no body to lower");
        assert_eq!(
            program.extern_functions,
            vec![crate::FfiSignature {
                symbol: "native_add".into(),
                params: vec![HirType::F64, HirType::F64],
                ret: HirType::F64,
            }]
        );

        let main = &program.functions[0];
        assert_eq!(
            main.body[0],
            HirStmt::Expr(HirExpr::Call(
                Box::new(HirExpr::Var("console.log".into())),
                vec![HirExpr::FfiCall(
                    crate::FfiSignature {
                        symbol: "native_add".into(),
                        params: vec![HirType::F64, HirType::F64],
                        ret: HirType::F64,
                    },
                    vec![
                        HirExpr::Lit(HirLit::F64(2.0)),
                        HirExpr::Lit(HirLit::F64(3.0)),
                    ],
                )],
            ))
        );
    }

    #[test]
    fn lowers_interface_as_a_named_object_type() {
        let program = lower(
            r#"interface Point {
                x: number;
                y: number;
            }

            function dist(p: Point): number {
                return p.x + p.y;
            }

            function main(): void {
                const p: Point = { y: 2, x: 1 };
                console.log(dist(p));
            }"#,
        );

        let point_ty = HirType::Object(vec![("x".into(), HirType::F64), ("y".into(), HirType::F64)]);

        let dist = &program.functions[0];
        assert_eq!(dist.params, vec![HirParam { name: "p".into(), ty: point_ty.clone() }]);

        let main = &program.functions[1];
        // Declared via the interface name, but the literal is still
        // reordered to the interface's field order (same machinery as
        // inline object type literals).
        assert_eq!(
            main.body[0],
            HirStmt::Let(
                "p".into(),
                point_ty,
                HirExpr::ObjectLit(vec![
                    ("x".into(), HirExpr::Lit(HirLit::F64(1.0))),
                    ("y".into(), HirExpr::Lit(HirLit::F64(2.0))),
                ]),
            )
        );
    }

    #[test]
    fn interfaces_can_reference_each_other_regardless_of_declaration_order() {
        // `Line` is declared before `Point`, and refers to it -- the
        // resolver must not depend on source order.
        let program = lower(
            r#"interface Line {
                start: Point;
                length: number;
            }

            interface Point {
                x: number;
                y: number;
            }

            function main(): void {
                const l: Line = { start: { x: 1, y: 2 }, length: 5 };
                console.log(l.length);
            }"#,
        );

        let point_ty = HirType::Object(vec![("x".into(), HirType::F64), ("y".into(), HirType::F64)]);
        let line_ty = HirType::Object(vec![
            ("start".into(), point_ty),
            ("length".into(), HirType::F64),
        ]);

        assert_eq!(
            program.functions[0].body[0],
            HirStmt::Let(
                "l".into(),
                line_ty,
                HirExpr::ObjectLit(vec![
                    (
                        "start".into(),
                        HirExpr::ObjectLit(vec![
                            ("x".into(), HirExpr::Lit(HirLit::F64(1.0))),
                            ("y".into(), HirExpr::Lit(HirLit::F64(2.0))),
                        ]),
                    ),
                    ("length".into(), HirExpr::Lit(HirLit::F64(5.0))),
                ]),
            )
        );
    }

    #[test]
    fn rejects_self_referential_interface() {
        let module = thaw_parser::parse_typescript(
            r#"interface Node {
                value: number;
                next: Node;
            }
            function main(): void {}"#,
        )
        .unwrap();
        let err = lower_module(&module).unwrap_err();
        assert!(err.contains("self-referential"), "unexpected error: {err}");
    }

    #[test]
    fn lowers_generic_interface_instantiated_with_a_concrete_type() {
        let program = lower(
            r#"interface Box<T> {
                value: T;
            }
            function unwrap(b: Box<number>): number {
                return b.value;
            }
            function main(): void {
                const b: Box<number> = { value: 5 };
                console.log(unwrap(b));
            }"#,
        );

        let box_number_ty = HirType::Object(vec![("value".into(), HirType::F64)]);
        assert_eq!(
            program.functions[0].params,
            vec![HirParam {
                name: "b".into(),
                ty: box_number_ty.clone()
            }]
        );
        assert_eq!(
            program.functions[1].body[0],
            HirStmt::Let(
                "b".into(),
                box_number_ty,
                HirExpr::ObjectLit(vec![("value".into(), HirExpr::Lit(HirLit::F64(5.0)))]),
            )
        );
    }

    #[test]
    fn generic_interface_instantiations_with_different_arguments_are_distinct_shapes() {
        let program = lower(
            r#"interface Box<T> { value: T; }
            function f(a: Box<number>, b: Box<string>): void {}
            function main(): void {}"#,
        );
        assert_eq!(
            program.functions[0].params[0].ty,
            HirType::Object(vec![("value".into(), HirType::F64)])
        );
        assert_eq!(
            program.functions[0].params[1].ty,
            HirType::Object(vec![("value".into(), HirType::Str)])
        );
    }

    #[test]
    fn generic_interface_field_can_be_an_array_or_object_literal_of_the_type_parameter() {
        let program = lower(
            r#"interface Box<T> {
                items: T[];
            }
            function main(): void {
                const b: Box<number> = { items: [1, 2, 3] };
                console.log(b.items.length);
            }"#,
        );
        let box_ty = HirType::Object(vec![("items".into(), HirType::Array(Box::new(HirType::F64)))]);
        assert!(matches!(&program.functions[0].body[0], HirStmt::Let(_, ty, _) if *ty == box_ty));
    }

    #[test]
    fn rejects_wrong_number_of_generic_type_arguments() {
        let module = thaw_parser::parse_typescript(
            r#"interface Pair<A, B> { first: A; second: B; }
            function main(): void {
                const p: Pair<number> = { first: 1, second: 2 };
            }"#,
        )
        .unwrap();
        let err = lower_module(&module).unwrap_err();
        assert!(err.contains("type argument"), "unexpected error: {err}");
    }

    #[test]
    fn rejects_self_referential_generic_interface() {
        // Triggered via a parameter type (not a `let`) so the error comes
        // from resolving `Node<number>` itself, not from lowering some
        // initializer expression first.
        let module = thaw_parser::parse_typescript(
            r#"interface Node<T> {
                value: T;
                next: Node<T>;
            }
            function f(n: Node<number>): void {}
            function main(): void {}"#,
        )
        .unwrap();
        let err = lower_module(&module).unwrap_err();
        assert!(err.contains("self-referential"), "unexpected error: {err}");
    }

    #[test]
    fn interface_extends_prepends_base_fields() {
        let program = lower(
            r#"interface Shape {
                color: number;
            }
            interface Circle extends Shape {
                radius: number;
            }
            function main(): void {
                const c: Circle = { color: 1, radius: 2 };
                console.log(c.radius);
            }"#,
        );

        let circle_ty = HirType::Object(vec![
            ("color".into(), HirType::F64),
            ("radius".into(), HirType::F64),
        ]);
        assert_eq!(
            program.functions[0].body[0],
            HirStmt::Let(
                "c".into(),
                circle_ty,
                HirExpr::ObjectLit(vec![
                    ("color".into(), HirExpr::Lit(HirLit::F64(1.0))),
                    ("radius".into(), HirExpr::Lit(HirLit::F64(2.0))),
                ]),
            )
        );
    }

    #[test]
    fn interface_can_extend_multiple_bases_in_order() {
        let program = lower(
            r#"interface A { a: number; }
            interface B { b: number; }
            interface C extends A, B {
                c: number;
            }
            function main(): void {
                const v: C = { a: 1, b: 2, c: 3 };
                console.log(v.a);
            }"#,
        );
        let c_ty = HirType::Object(vec![
            ("a".into(), HirType::F64),
            ("b".into(), HirType::F64),
            ("c".into(), HirType::F64),
        ]);
        assert_eq!(
            program.functions[0].body[0],
            HirStmt::Let(
                "v".into(),
                c_ty,
                HirExpr::ObjectLit(vec![
                    ("a".into(), HirExpr::Lit(HirLit::F64(1.0))),
                    ("b".into(), HirExpr::Lit(HirLit::F64(2.0))),
                    ("c".into(), HirExpr::Lit(HirLit::F64(3.0))),
                ]),
            )
        );
    }

    #[test]
    fn rejects_extends_field_name_collision() {
        let module = thaw_parser::parse_typescript(
            r#"interface A { x: number; }
            interface B extends A { x: number; }
            function main(): void {}"#,
        )
        .unwrap();
        let err = lower_module(&module).unwrap_err();
        assert!(err.contains("collides"), "unexpected error: {err}");
    }

    #[test]
    fn extends_chains_work_transitively() {
        let program = lower(
            r#"interface A { a: number; }
            interface B extends A { b: number; }
            interface C extends B { c: number; }
            function main(): void {
                const v: C = { a: 1, b: 2, c: 3 };
                console.log(v.a);
            }"#,
        );
        let c_ty = HirType::Object(vec![
            ("a".into(), HirType::F64),
            ("b".into(), HirType::F64),
            ("c".into(), HirType::F64),
        ]);
        assert!(matches!(&program.functions[0].body[0], HirStmt::Let(_, ty, _) if *ty == c_ty));
    }
}
