//! SWC AST -> Thaw HIR lowering.
//!
//! Function parameters are inferred from their module-local call sites when
//! every call agrees; otherwise they need annotations. Local initializers and
//! function returns are inferred too, including forward call chains, and the
//! resulting concrete types are checked at assignments, returns, operators,
//! indexes, and call boundaries before LLVM lowering. Phase 1
//! adds `if`/`while`/classic `for`/`throw`/`try`/`catch`, local
//! `let`/`const`, assignment/`++`/`--`, and number arrays. Phase 2 adds
//! `process.env` and object types (`{ x: number; y: number }`-style
//! records, `f64` fields only at codegen time -- see hir_codegen).
//!
//! Object support is why this module carries a type *scope* now instead of
//! being purely syntax-directed: resolving `obj.field` needs to know
//! whether `obj` is an array (`.length`) or an object (which field, at
//! which offset) without a real type checker. Source-level bindings are
//! tracked lexically and renamed to unique HIR symbols when shadowed; the
//! type table remains flat because those HIR symbols never collide.
//!
//! A single SWC `Stmt` can lower to *several* HIR statements (`for` becomes
//! a `Let` followed by a `While`), so the statement lowering entry point is
//! `lower_stmt_seq`, not a single-statement `lower_stmt`.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::fmt;

use swc_common::{BytePos, SourceMap};
use swc_ecma_ast::{
    ArrowFunctionBody, AssignOp, AssignTarget, BinaryOp, CallExpr, Callee, ComputedPropName, Decl,
    Expr, FnDecl, KeyValueProp, Lit, MemberExpr, MemberProp, Module, ModuleItem,
    ObjectLit as SwcObjectLit, Pat, Prop, PropName, PropOrSpread, SimpleAssignTarget, Stmt,
    TsFnOrConstructorType, TsFnParam, TsInterfaceDecl, TsKeywordTypeKind, TsType, TsTypeElement,
    UpdateOp, VarDecl, VarDeclOrExpr,
};

use crate::{
    BinOp, DynamicBackend, DynamicSignature, FfiErrorAbi, FfiOwnership, FfiSignature, HirExpr,
    HirFunction, HirLit, HirParam, HirProgram, HirStmt, HirType, Symbol,
};

fn dynamic_symbol(name: &str) -> Option<(DynamicBackend, String)> {
    let (backend, hex) = name
        .strip_prefix("__thaw_typed_js_")
        .map(|hex| (DynamicBackend::QuickJs, hex))
        .or_else(|| {
            name.strip_prefix("__thaw_typed_napi_")
                .map(|hex| (DynamicBackend::Napi, hex))
        })?;
    if hex.len() % 2 != 0 {
        return None;
    }
    let bytes = (0..hex.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).ok())
        .collect::<Option<Vec<_>>>()?;
    String::from_utf8(bytes)
        .ok()
        .map(|symbol| (backend, symbol))
}

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
    source_range: (u32, u32),
    generic_type_params: Vec<Symbol>,
    generic_param_patterns: Vec<GenericTypePattern>,
    generic_return_type: Option<Box<TsType>>,
}

#[derive(Clone, Debug)]
enum GenericTypePattern {
    Variable(Symbol),
    Concrete(HirType),
    Array(Box<GenericTypePattern>),
    Promise(Box<GenericTypePattern>),
    Object(Vec<(Symbol, GenericTypePattern)>),
}

#[derive(Clone)]
enum CallConstraint {
    Parameter(Symbol, usize, HirType, (u32, u32)),
    Generic(Symbol, Vec<HirType>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRange {
    pub file: String,
    pub line: usize,
    pub column: usize,
    pub end_line: usize,
    pub end_column: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LowerDiagnostic {
    pub message: String,
    pub range: Option<SourceRange>,
}

impl fmt::Display for LowerDiagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.range {
            Some(range) => write!(
                formatter,
                "{}:{}:{}: {}",
                range.file, range.line, range.column, self.message
            ),
            None => formatter.write_str(&self.message),
        }
    }
}

impl std::error::Error for LowerDiagnostic {}

/// Structured companion to [`lower_module`]. Existing callers can keep the
/// string API, while CLI/tooling callers get a stable message plus a source
/// range resolved through the parser's `SourceMap`.
pub fn lower_module_with_source_map(
    module: &Module,
    source_map: &SourceMap,
    file_name: impl Into<String>,
) -> Result<HirProgram, LowerDiagnostic> {
    lower_module(module).map_err(|raw| {
        let file = file_name.into();
        let Some((message, bytes)) = raw.rsplit_once(" at bytes ") else {
            return LowerDiagnostic {
                message: raw,
                range: None,
            };
        };
        let Some((lo, hi)) = bytes.split_once("..") else {
            return LowerDiagnostic {
                message: raw,
                range: None,
            };
        };
        let (Ok(lo), Ok(hi)) = (lo.parse::<u32>(), hi.parse::<u32>()) else {
            return LowerDiagnostic {
                message: raw,
                range: None,
            };
        };
        let start = source_map.lookup_char_pos(BytePos(lo));
        let end = source_map.lookup_char_pos(BytePos(hi));
        LowerDiagnostic {
            message: message.to_string(),
            range: Some(SourceRange {
                file,
                line: start.line,
                column: start.col_display + 1,
                end_line: end.line,
                end_column: end.col_display + 1,
            }),
        }
    })
}

pub fn lower_module(module: &Module) -> Result<HirProgram, String> {
    let (interfaces, generic_interfaces) = resolve_interfaces(module)?;

    let mut signatures: HashMap<Symbol, FnSignature> = HashMap::new();
    let mut fn_decls = Vec::new();
    let mut generic_instantiations: HashMap<Symbol, Vec<Vec<HirType>>> = HashMap::new();

    for item in &module.body {
        match item {
            ModuleItem::Stmt(Stmt::Decl(Decl::Fn(fn_decl))) => {
                let name = fn_decl.ident.sym.to_string();
                let func = &fn_decl.function;
                let is_extern = func.body.is_none();
                if is_extern && func.is_async {
                    return Err(format!("ambient function `{name}` cannot be async"));
                }
                if is_extern && func.type_params.is_some() {
                    return Err(format!(
                        "ambient generic function `{name}` needs an explicitly monomorphic native ABI"
                    ));
                }
                let type_substitution = function_type_substitution(func);
                let generic_type_params = validate_generic_function(fn_decl)?;
                let generic_param_patterns = if generic_type_params.is_empty() {
                    Vec::new()
                } else {
                    let substitutions = generic_type_params
                        .iter()
                        .map(|name| (name.clone(), GenericTypePattern::Variable(name.clone())))
                        .collect::<HashMap<_, _>>();
                    func.params
                        .iter()
                        .map(|param| match &param.pat {
                            Pat::Ident(binding) => binding
                                .type_ann
                                .as_ref()
                                .ok_or_else(|| format!("generic function `{name}` needs parameter type annotations"))
                                .and_then(|ann| generic_type_pattern(
                                    &ann.type_ann,
                                    &substitutions,
                                    &interfaces,
                                    &generic_interfaces,
                                    &mut Vec::new(),
                                )),
                            _ => Err(format!("generic function `{name}` requires identifier parameters")),
                        })
                        .collect::<Result<Vec<_>, _>>()?
                };
                let params = func
                    .params
                    .iter()
                    .map(|p| {
                        lower_param(
                            &p.pat,
                            &interfaces,
                            &generic_interfaces,
                            !is_extern,
                            &type_substitution,
                        )
                        .map(|p| p.ty)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let ret = if func.return_type.is_none() && !is_extern {
                    HirType::Dynamic
                } else {
                    lower_fn_return_type(
                        func.is_async,
                        &func.return_type,
                        &name,
                        &interfaces,
                        &generic_interfaces,
                        &type_substitution,
                    )?
                };
                signatures.insert(
                    name,
                    FnSignature {
                        params,
                        ret,
                        is_extern,
                        source_range: (func.span.lo.0, func.span.hi.0),
                        generic_type_params,
                        generic_param_patterns,
                        generic_return_type: func.return_type.as_ref().map(|ann| ann.type_ann.clone()),
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

    // Missing parameter/return annotations are type variables. Re-lower against the
    // signatures discovered in the previous round until forward calls and
    // mutually recursive functions reach a fixed point.
    for _ in 0..=(fn_decls.len() * 2 + 1) {
        let mut changed = false;
        let call_constraints = RefCell::new(Vec::new());
        for fn_decl in &fn_decls {
            let name = fn_decl.ident.sym.to_string();
            let function = lower_fn_decl(
                fn_decl,
                &signatures,
                &interfaces,
                &generic_interfaces,
                Some(&call_constraints),
            )?;
            if signatures[&name].ret == HirType::Dynamic && function.ret != HirType::Dynamic {
                signatures.get_mut(&name).unwrap().ret = function.ret;
                changed = true;
            }
        }
        for constraint in call_constraints.into_inner() {
            match constraint {
                CallConstraint::Generic(callee, types) => {
                    if types.iter().any(|ty| *ty == HirType::Dynamic) {
                        continue;
                    }
                    let instances = generic_instantiations.entry(callee).or_default();
                    if !instances.contains(&types) {
                        instances.push(types);
                    }
                }
                CallConstraint::Parameter(callee, index, actual, call_range) => {
                    if actual == HirType::Dynamic {
                        continue;
                    }
                    let param = &mut signatures.get_mut(&callee).unwrap().params[index];
                    if *param == HirType::Dynamic {
                        *param = actual;
                        changed = true;
                    } else if *param != actual {
                        return Err(format!(
                            "conflicting inferred types for parameter {} of `{callee}`: {param:?} and {actual:?} at bytes {}..{}",
                            index + 1, call_range.0, call_range.1,
                        ));
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
    if let Some((name, signature)) = signatures.iter().find(|(_, signature)| {
        !signature.is_extern
            && signature.generic_type_params.is_empty()
            && signature.params.contains(&HirType::Dynamic)
    }) {
        let index = signature
            .params
            .iter()
            .position(|ty| *ty == HirType::Dynamic)
            .unwrap();
        return Err(format!(
            "cannot infer parameter {} of function `{name}` from its call sites; add an explicit type annotation at bytes {}..{}",
            index + 1,
            signature.source_range.0,
            signature.source_range.1,
        ));
    }
    let unresolved = signatures.iter().find(|(_, signature)| {
        !signature.is_extern
            && signature.generic_type_params.is_empty()
            && signature.ret == HirType::Dynamic
    });
    if let Some((name, signature)) = unresolved {
        return Err(format!(
            "cannot infer the return type of function `{name}`; add an explicit return annotation at bytes {}..{}",
            signature.source_range.0,
            signature.source_range.1,
        ));
    }

    let extern_functions = signatures
        .iter()
        .filter(|(name, sig)| sig.is_extern && dynamic_symbol(name).is_none())
        .map(|(name, sig)| FfiSignature {
            symbol: name.clone(),
            params: sig.params.clone(),
            ret: sig.ret.clone(),
            error_abi: FfiErrorAbi::Direct,
            return_ownership: FfiOwnership::Borrowed,
            error_ownership: FfiOwnership::Borrowed,
        })
        .collect();

    let functions = fn_decls
        .into_iter()
        .map(|fn_decl| lower_fn_decl(fn_decl, &signatures, &interfaces, &generic_interfaces, None))
        .collect::<Result<Vec<_>, _>>()?;
    let mut specialized = functions
        .into_iter()
        .filter(|function| signatures[&function.name].generic_type_params.is_empty())
        .collect::<Vec<_>>();
    let mut pending = generic_instantiations
        .iter()
        .flat_map(|(name, instances)| instances.iter().cloned().map(|types| (name.clone(), types)))
        .collect::<Vec<_>>();
    let mut completed: Vec<(Symbol, Vec<HirType>)> = Vec::new();
    while let Some((name, types)) = pending.pop() {
        if completed
            .iter()
            .any(|done| done == &(name.clone(), types.clone()))
        {
            continue;
        }
        let decl = module
            .body
            .iter()
            .find_map(|item| match item {
                ModuleItem::Stmt(Stmt::Decl(Decl::Fn(decl))) if decl.ident.sym.as_ref() == name => {
                    Some(decl)
                }
                _ => None,
            })
            .ok_or_else(|| format!("missing declaration for `{name}`"))?;
        let nested_constraints = RefCell::new(Vec::new());
        let instance = lower_generic_instance(
            decl,
            &types,
            &signatures,
            &interfaces,
            &generic_interfaces,
            Some(&nested_constraints),
        )?;
        completed.push((name, types));
        specialized.push(instance);
        for constraint in nested_constraints.into_inner() {
            if let CallConstraint::Generic(nested_name, nested_types) = constraint {
                if !nested_types.iter().any(|ty| *ty == HirType::Dynamic) {
                    pending.push((nested_name, nested_types));
                }
            }
        }
    }

    Ok(HirProgram {
        functions: specialized,
        extern_functions,
    })
}

fn validate_generic_function(fn_decl: &FnDecl) -> Result<Vec<Symbol>, String> {
    let Some(type_params) = &fn_decl.function.type_params else {
        return Ok(Vec::new());
    };
    let name = fn_decl.ident.sym.as_str();
    if type_params.params.is_empty() || fn_decl.function.return_type.is_none() {
        return Err(format!(
            "generic function `{name}` needs type parameters and a return annotation"
        ));
    }
    Ok(type_params
        .params
        .iter()
        .map(|param| param.name.sym.to_string())
        .collect())
}

fn supports_generic_native_layout(ty: &HirType) -> bool {
    match ty {
        HirType::F64 | HirType::I64 | HirType::Bool | HirType::Str => true,
        HirType::Array(inner) => **inner == HirType::F64,
        HirType::Object(fields) => fields
            .iter()
            .all(|(_, ty)| supports_generic_native_layout(ty)),
        _ => false,
    }
}

fn specialized_generic_name(name: &str, types: &[HirType]) -> Symbol {
    fn fingerprint(ty: &HirType) -> String {
        match ty {
            HirType::F64 => "f64".into(),
            HirType::I64 => "i64".into(),
            HirType::Bool => "bool".into(),
            HirType::Str => "str".into(),
            HirType::Json => "json".into(),
            HirType::Array(inner) => format!("array_{}", fingerprint(inner)),
            HirType::Object(fields) => format!(
                "object_{}",
                fields
                    .iter()
                    .map(|(name, ty)| format!("{name}_{}", fingerprint(ty)))
                    .collect::<Vec<_>>()
                    .join("_")
            ),
            other => panic!("unsupported generic specialization type: {other:?}"),
        }
    }
    format!(
        "{name}__thaw_{}",
        types.iter().map(fingerprint).collect::<Vec<_>>().join("__")
    )
}

fn generic_type_pattern(
    ty: &TsType,
    substitutions: &HashMap<Symbol, GenericTypePattern>,
    interfaces: &HashMap<Symbol, HirType>,
    generic_interfaces: &GenericInterfaces,
    in_progress: &mut Vec<Symbol>,
) -> Result<GenericTypePattern, String> {
    if let TsType::TsTypeRef(reference) = ty {
        if let swc_ecma_ast::TsEntityName::Ident(id) = &reference.type_name {
            let name = id.sym.as_str();
            if let Some(pattern) = substitutions.get(name) {
                return Ok(pattern.clone());
            }
            if let Some(interface) = generic_interfaces.get(name) {
                if in_progress.iter().any(|active| active == name) {
                    return Err(format!("generic interface `{name}` is self-referential"));
                }
                if !interface.extends.is_empty() {
                    return Err(format!(
                        "generic interface `{name}` cannot use `extends` yet"
                    ));
                }
                let parameter_names = interface
                    .type_params
                    .as_ref()
                    .expect("generic interface type parameters")
                    .params
                    .iter()
                    .map(|param| param.name.sym.to_string())
                    .collect::<Vec<_>>();
                let arguments = reference
                    .type_params
                    .as_ref()
                    .map(|params| params.params.as_slice())
                    .unwrap_or_default();
                if arguments.len() != parameter_names.len() {
                    return Err(format!(
                        "generic interface `{name}` expects {} type argument(s), got {}",
                        parameter_names.len(),
                        arguments.len()
                    ));
                }
                let argument_patterns = arguments
                    .iter()
                    .map(|argument| {
                        generic_type_pattern(
                            argument,
                            substitutions,
                            interfaces,
                            generic_interfaces,
                            in_progress,
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let nested_substitutions = parameter_names
                    .into_iter()
                    .zip(argument_patterns)
                    .collect::<HashMap<_, _>>();
                in_progress.push(name.to_string());
                let fields = interface
                    .body
                    .body
                    .iter()
                    .map(|member| {
                        let TsTypeElement::TsPropertySignature(property) = member else {
                            return Err(format!(
                                "generic interface `{name}` only supports plain properties"
                            ));
                        };
                        let Expr::Ident(field) = property.key.as_ref() else {
                            return Err(format!(
                                "generic interface `{name}` has an unsupported property key"
                            ));
                        };
                        let annotation = property.type_ann.as_ref().ok_or_else(|| {
                            format!("field `{}` needs a type annotation", field.sym)
                        })?;
                        Ok((
                            field.sym.to_string(),
                            generic_type_pattern(
                                &annotation.type_ann,
                                &nested_substitutions,
                                interfaces,
                                generic_interfaces,
                                in_progress,
                            )?,
                        ))
                    })
                    .collect::<Result<Vec<_>, String>>();
                in_progress.pop();
                return Ok(GenericTypePattern::Object(fields?));
            }
            if let Some(inner) = reference
                .type_params
                .as_ref()
                .and_then(|params| params.params.as_slice().first())
            {
                let inner = Box::new(generic_type_pattern(
                    inner,
                    substitutions,
                    interfaces,
                    generic_interfaces,
                    in_progress,
                )?);
                match name {
                    "Array" => return Ok(GenericTypePattern::Array(inner)),
                    "Promise" => return Ok(GenericTypePattern::Promise(inner)),
                    _ => {}
                }
            }
        }
    }
    match ty {
        TsType::TsArrayType(array) => {
            Ok(GenericTypePattern::Array(Box::new(generic_type_pattern(
                &array.elem_type,
                substitutions,
                interfaces,
                generic_interfaces,
                in_progress,
            )?)))
        }
        TsType::TsTypeLit(literal) => Ok(GenericTypePattern::Object(
            literal
                .members
                .iter()
                .map(|member| {
                    let TsTypeElement::TsPropertySignature(property) = member else {
                        return Err("generic object patterns only support plain properties".into());
                    };
                    let Expr::Ident(field) = property.key.as_ref() else {
                        return Err("generic object pattern has an unsupported key".into());
                    };
                    let annotation = property
                        .type_ann
                        .as_ref()
                        .ok_or_else(|| format!("field `{}` needs a type annotation", field.sym))?;
                    Ok((
                        field.sym.to_string(),
                        generic_type_pattern(
                            &annotation.type_ann,
                            substitutions,
                            interfaces,
                            generic_interfaces,
                            in_progress,
                        )?,
                    ))
                })
                .collect::<Result<Vec<_>, String>>()?,
        )),
        other => Ok(GenericTypePattern::Concrete(lower_ts_type(
            other,
            interfaces,
            generic_interfaces,
        )?)),
    }
}

fn match_generic_pattern(
    pattern: &GenericTypePattern,
    actual: &HirType,
    inferred: &mut HashMap<Symbol, HirType>,
) -> Result<(), String> {
    match (pattern, actual) {
        (GenericTypePattern::Variable(name), actual) => {
            if let Some(previous) = inferred.get(name) {
                if previous != actual {
                    return Err(format!(
                    "generic type parameter has conflicting call-site types {previous:?} and {actual:?}"
                ));
                }
            } else {
                inferred.insert(name.clone(), actual.clone());
            }
            Ok(())
        }
        (GenericTypePattern::Array(expected), HirType::Array(value))
        | (GenericTypePattern::Promise(expected), HirType::Promise(value)) => {
            match_generic_pattern(expected, value, inferred)
        }
        (GenericTypePattern::Object(expected), HirType::Object(value))
            if expected.len() == value.len() =>
        {
            for ((expected_name, expected_ty), (actual_name, actual_ty)) in
                expected.iter().zip(value)
            {
                if expected_name != actual_name {
                    return Err(format!(
                        "generic argument object field `{actual_name}` does not match `{expected_name}`"
                    ));
                }
                match_generic_pattern(expected_ty, actual_ty, inferred)?;
            }
            Ok(())
        }
        (GenericTypePattern::Concrete(expected), actual)
            if *expected == HirType::Dynamic || expected == actual =>
        {
            Ok(())
        }
        _ => Err(format!(
            "generic argument has type {actual:?}, incompatible with parameter pattern {pattern:?}"
        )),
    }
}

fn infer_generic_type_tuple(
    signature: &FnSignature,
    actual_params: &[HirType],
) -> Result<Vec<HirType>, String> {
    let mut inferred = HashMap::new();
    for (pattern, actual) in signature.generic_param_patterns.iter().zip(actual_params) {
        match_generic_pattern(pattern, actual, &mut inferred)?;
    }
    signature
        .generic_type_params
        .iter()
        .map(|name| {
            inferred.get(name).cloned().ok_or_else(|| {
                format!("cannot infer generic type parameter `{name}` from this call")
            })
        })
        .collect()
}

fn lower_generic_instance(
    fn_decl: &FnDecl,
    types: &[HirType],
    signatures: &HashMap<Symbol, FnSignature>,
    interfaces: &HashMap<Symbol, HirType>,
    generic_interfaces: &GenericInterfaces,
    call_constraints: Option<&RefCell<Vec<CallConstraint>>>,
) -> Result<HirFunction, String> {
    let base_name = fn_decl.ident.sym.to_string();
    let signature = &signatures[&base_name];
    let substitution = signature
        .generic_type_params
        .iter()
        .cloned()
        .zip(types.iter().cloned())
        .collect::<HashMap<_, _>>();
    let params = fn_decl
        .function
        .params
        .iter()
        .map(|param| {
            lower_param(
                &param.pat,
                interfaces,
                generic_interfaces,
                false,
                &substitution,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let ret = lower_fn_return_type(
        fn_decl.function.is_async,
        &fn_decl.function.return_type,
        &base_name,
        interfaces,
        generic_interfaces,
        &substitution,
    )?;
    let mut concrete_signatures = signatures.clone();
    let concrete = concrete_signatures.get_mut(&base_name).unwrap();
    concrete.params = params.iter().map(|param| param.ty.clone()).collect();
    concrete.ret = ret.clone();
    let mut lowerer = FnLowerer::new(
        &concrete_signatures,
        interfaces,
        generic_interfaces,
        ret.clone(),
        call_constraints,
    );
    for param in &params {
        lowerer.scope.insert(param.name.clone(), param.ty.clone());
        lowerer
            .bindings
            .entry(param.name.clone())
            .or_default()
            .push(param.name.clone());
    }
    let body = lowerer.lower_stmts(
        &fn_decl
            .function
            .body
            .as_ref()
            .ok_or_else(|| format!("function `{base_name}` has no body"))?
            .stmts,
    )?;
    Ok(HirFunction {
        name: specialized_generic_name(
            &base_name,
            &params
                .iter()
                .map(|param| param.ty.clone())
                .collect::<Vec<_>>(),
        ),
        params,
        ret,
        is_async: fn_decl.function.is_async,
        body,
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
        return Err(format!(
            "generic interfaces are not supported yet (`{name}`)"
        ));
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
            _ => {
                return Err(format!(
                    "interface `{name}` has an unsupported property key"
                ))
            }
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
    call_constraints: Option<&RefCell<Vec<CallConstraint>>>,
) -> Result<HirFunction, String> {
    let name = fn_decl.ident.sym.to_string();
    let func = &fn_decl.function;
    let type_substitution = function_type_substitution(func);

    let mut params = func
        .params
        .iter()
        .map(|param| {
            lower_param(
                &param.pat,
                interfaces,
                generic_interfaces,
                true,
                &type_substitution,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    for (param, inferred) in params.iter_mut().zip(&signatures[&name].params) {
        param.ty = inferred.clone();
    }

    let declared_ret = signatures[&name].ret.clone();

    let body_block = func
        .body
        .as_ref()
        .ok_or_else(|| format!("function `{name}` has no body (ambient/overload decl?)"))?;

    let mut lowerer = FnLowerer::new(
        signatures,
        interfaces,
        generic_interfaces,
        declared_ret.clone(),
        call_constraints,
    );
    for param in &params {
        lowerer.scope.insert(param.name.clone(), param.ty.clone());
        lowerer
            .bindings
            .entry(param.name.clone())
            .or_default()
            .push(param.name.clone());
    }
    let body = lowerer.lower_stmts(&body_block.stmts)?;
    let ret = if declared_ret == HirType::Dynamic {
        lowerer.infer_return_type(&body)?
    } else {
        declared_ret
    };

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
    type_substitution: &HashMap<Symbol, HirType>,
) -> Result<HirType, String> {
    let declared = match return_type {
        Some(ann) => resolve_ts_type_with_substitution(
            &ann.type_ann,
            type_substitution,
            interfaces,
            generic_interfaces,
            &mut Vec::new(),
        )?,
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
    allow_inference: bool,
    type_substitution: &HashMap<Symbol, HirType>,
) -> Result<HirParam, String> {
    let Pat::Ident(binding) = pat else {
        return Err("only simple identifier parameters are supported".into());
    };
    let name = binding.id.sym.to_string();
    let ty = match &binding.type_ann {
        Some(ann) => resolve_ts_type_with_substitution(
            &ann.type_ann,
            type_substitution,
            interfaces,
            generic_interfaces,
            &mut Vec::new(),
        )?,
        None if allow_inference => HirType::Dynamic,
        None => {
            return Err(format!(
            "parameter `{name}` needs an explicit type annotation (no type inference for params)"
        ))
        }
    };
    Ok(HirParam { name, ty })
}

fn function_type_substitution(function: &swc_ecma_ast::Function) -> HashMap<Symbol, HirType> {
    function
        .type_params
        .as_ref()
        .map(|params| {
            params
                .params
                .iter()
                .map(|param| (param.name.sym.to_string(), HirType::Dynamic))
                .collect()
        })
        .unwrap_or_default()
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
        TsType::TsFnOrConstructorType(TsFnOrConstructorType::TsFnType(function)) => {
            if function.type_params.is_some() {
                return Err("generic function types are not supported yet".into());
            }
            let params = function
                .params
                .iter()
                .map(|param| {
                    let TsFnParam::Ident(param) = param else {
                        return Err("function types only support identifier parameters".into());
                    };
                    let annotation = param.type_ann.as_ref().ok_or_else(|| {
                        format!("function parameter `{}` needs a type annotation", param.id.sym)
                    })?;
                    lower_ts_type(&annotation.type_ann, interfaces, generic_interfaces)
                })
                .collect::<Result<Vec<_>, String>>()?;
            let ret = lower_ts_type(
                &function.type_ann.type_ann,
                interfaces,
                generic_interfaces,
            )?;
            Ok(HirType::Function(params, Box::new(ret)))
        }
        TsType::TsFnOrConstructorType(TsFnOrConstructorType::TsConstructorType(_)) => {
            Err("constructor types are not supported yet".into())
        }
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
                        None,
                        &mut Vec::new(),
                    );
                }
            }

            // `Json`, with no type arguments -- the annotation spelling for
            // `HirType::Json` (a dynamic value, e.g. from `JSON.parse`).
            // Without this there was no way to *write* a `Json`-typed
            // parameter/`let` annotation; it could only ever be inferred
            // as an expression's type. Needed for e.g. thaw-bridge's
            // generated Fallback wrappers (`function f(args: Json): Json`).
            if ref_name == Some("Json") && ty_ref.type_params.is_none() {
                return Ok(HirType::Json);
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
/// The generic interface itself cannot use `extends` yet. Type arguments are
/// resolved through an optional outer substitution, so function type variables
/// in `Box<T>` and nested forms such as `Wrapper<Box<T>>` are concrete before
/// the interface's own fields are expanded.
fn resolve_generic_interface(
    name: &str,
    decl: &TsInterfaceDecl,
    ty_ref: &swc_ecma_ast::TsTypeRef,
    interfaces: &HashMap<Symbol, HirType>,
    generic_interfaces: &GenericInterfaces,
    outer_substitution: Option<&HashMap<Symbol, HirType>>,
    in_progress: &mut Vec<Symbol>,
) -> Result<HirType, String> {
    if in_progress.iter().any(|n| n == name) {
        return Err(format!(
            "generic interface `{name}` is (indirectly) self-referential, which Thaw's fixed-size object layout can't represent"
        ));
    }
    if !decl.extends.is_empty() {
        return Err(format!(
            "generic interface `{name}` cannot use `extends` yet"
        ));
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
        .map(|arg| match outer_substitution {
            Some(outer) => resolve_ts_type_with_substitution(
                arg,
                outer,
                interfaces,
                generic_interfaces,
                in_progress,
            ),
            None => lower_ts_type(arg, interfaces, generic_interfaces),
        })
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
            _ => {
                return Err(format!(
                    "interface `{name}` has an unsupported property key"
                ))
            }
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
                    Some(substitution),
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
        TsType::TsArrayType(arr) => {
            Ok(HirType::Array(Box::new(resolve_ts_type_with_substitution(
                &arr.elem_type,
                substitution,
                interfaces,
                generic_interfaces,
                in_progress,
            )?)))
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

fn collect_referenced_bindings(expr: &HirExpr, names: &mut BTreeSet<Symbol>) {
    match expr {
        HirExpr::Var(name) => {
            names.insert(name.clone());
        }
        HirExpr::Assign(name, value) => {
            names.insert(name.clone());
            collect_referenced_bindings(value, names);
        }
        HirExpr::BinOp(_, left, right) | HirExpr::Index(left, right) => {
            collect_referenced_bindings(left, names);
            collect_referenced_bindings(right, names);
        }
        HirExpr::Call(callee, args) => {
            collect_referenced_bindings(callee, names);
            for arg in args {
                collect_referenced_bindings(arg, names);
            }
        }
        HirExpr::Await(value)
        | HirExpr::ArrayLen(value)
        | HirExpr::JsonAsNumber(value)
        | HirExpr::JsonAsString(value)
        | HirExpr::JsonAsBool(value) => collect_referenced_bindings(value, names),
        HirExpr::Lambda(_, _, _, body) => collect_referenced_bindings(body, names),
        HirExpr::Block(stmts) => collect_stmt_bindings(stmts, names),
        HirExpr::FfiCall(_, args) | HirExpr::DynamicCall(_, args) | HirExpr::ArrayLit(args) => {
            for arg in args {
                collect_referenced_bindings(arg, names);
            }
        }
        HirExpr::IndexAssign(array, index, value) => {
            collect_referenced_bindings(array, names);
            collect_referenced_bindings(index, names);
            collect_referenced_bindings(value, names);
        }
        HirExpr::ObjectLit(fields) => {
            for (_, value) in fields {
                collect_referenced_bindings(value, names);
            }
        }
        HirExpr::PropAccess(object, _, _) | HirExpr::JsonGet(object, _) => {
            collect_referenced_bindings(object, names);
        }
        HirExpr::PropAssign(object, _, _, value) | HirExpr::JsonIndex(object, value) => {
            collect_referenced_bindings(object, names);
            collect_referenced_bindings(value, names);
        }
        HirExpr::Lit(_) | HirExpr::EnvVar(_) => {}
    }
}

fn collect_stmt_bindings(stmts: &[HirStmt], names: &mut BTreeSet<Symbol>) {
    for stmt in stmts {
        match stmt {
            HirStmt::Expr(expr) | HirStmt::Throw(expr) | HirStmt::Let(_, _, expr) => {
                collect_referenced_bindings(expr, names)
            }
            HirStmt::Return(Some(expr)) => collect_referenced_bindings(expr, names),
            HirStmt::If(cond, then_body, else_body) => {
                collect_referenced_bindings(cond, names);
                collect_stmt_bindings(then_body, names);
                collect_stmt_bindings(else_body, names);
            }
            HirStmt::While(cond, body) => {
                collect_referenced_bindings(cond, names);
                collect_stmt_bindings(body, names);
            }
            HirStmt::Try(body, _, catch_body) => {
                collect_stmt_bindings(body, names);
                collect_stmt_bindings(catch_body, names);
            }
            HirStmt::Return(None) | HirStmt::Break | HirStmt::Continue => {}
        }
    }
}

/// Expands an enclosing `finally` before every control-flow exit in `stmts`.
/// A throw in a try body is handled by that try's catch first, so recursive
/// descent into `HirStmt::Try` only instruments its catch body for throws.
/// Returns are always instrumented because they leave every enclosing try.
fn inject_finally_before_exits(
    stmts: Vec<HirStmt>,
    finalizer: &[HirStmt],
    inject_throws: bool,
) -> Vec<HirStmt> {
    let mut out = Vec::new();
    for stmt in stmts {
        match stmt {
            HirStmt::Return(_) => {
                out.extend(finalizer.iter().cloned());
                out.push(stmt);
            }
            HirStmt::Throw(_) if inject_throws => {
                out.extend(finalizer.iter().cloned());
                out.push(stmt);
            }
            HirStmt::If(cond, then_body, else_body) => out.push(HirStmt::If(
                cond,
                inject_finally_before_exits(then_body, finalizer, inject_throws),
                inject_finally_before_exits(else_body, finalizer, inject_throws),
            )),
            HirStmt::While(cond, body) => out.push(HirStmt::While(
                cond,
                inject_finally_before_exits(body, finalizer, inject_throws),
            )),
            HirStmt::Break | HirStmt::Continue => out.push(stmt),
            HirStmt::Try(body, catch_name, catch_body) => out.push(HirStmt::Try(
                inject_finally_before_exits(body, finalizer, false),
                catch_name,
                inject_finally_before_exits(catch_body, finalizer, inject_throws),
            )),
            other => out.push(other),
        }
    }
    out
}

/// A classic `for (...; ...; update)` is represented as a HIR `while` with
/// `update` appended to its body. A source-level `continue` must execute that
/// update before beginning the next condition check. Recurse through branches
/// belonging to this loop, but stop at nested loops whose `continue`s target
/// the nested loop instead.
fn inject_for_update_before_continue(stmts: Vec<HirStmt>, update: &HirExpr) -> Vec<HirStmt> {
    let mut out = Vec::new();
    for stmt in stmts {
        match stmt {
            HirStmt::Continue => {
                out.push(HirStmt::Expr(update.clone()));
                out.push(HirStmt::Continue);
            }
            HirStmt::If(cond, then_body, else_body) => out.push(HirStmt::If(
                cond,
                inject_for_update_before_continue(then_body, update),
                inject_for_update_before_continue(else_body, update),
            )),
            HirStmt::Try(body, catch_name, catch_body) => out.push(HirStmt::Try(
                inject_for_update_before_continue(body, update),
                catch_name,
                inject_for_update_before_continue(catch_body, update),
            )),
            // A continue below this node belongs to the nested loop.
            HirStmt::While(_, _) => out.push(stmt),
            other => out.push(other),
        }
    }
    out
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
    bindings: HashMap<Symbol, Vec<Symbol>>,
    next_binding: usize,
    signatures: &'a HashMap<Symbol, FnSignature>,
    interfaces: &'a HashMap<Symbol, HirType>,
    generic_interfaces: &'a GenericInterfaces<'a>,
    ret_type: HirType,
    call_constraints: Option<&'a RefCell<Vec<CallConstraint>>>,
}

impl<'a> FnLowerer<'a> {
    fn new(
        signatures: &'a HashMap<Symbol, FnSignature>,
        interfaces: &'a HashMap<Symbol, HirType>,
        generic_interfaces: &'a GenericInterfaces<'a>,
        ret_type: HirType,
        call_constraints: Option<&'a RefCell<Vec<CallConstraint>>>,
    ) -> Self {
        Self {
            scope: HashMap::new(),
            bindings: HashMap::new(),
            next_binding: 0,
            signatures,
            interfaces,
            generic_interfaces,
            ret_type,
            call_constraints,
        }
    }

    fn resolve_binding(&self, source_name: &str) -> Symbol {
        self.bindings
            .get(source_name)
            .and_then(|names| names.last())
            .cloned()
            .unwrap_or_else(|| source_name.to_string())
    }

    fn bind_local(&mut self, source_name: &str, ty: HirType) -> Symbol {
        let hir_name = if self.scope.contains_key(source_name) {
            let name = format!("{source_name}__thaw_{}", self.next_binding);
            self.next_binding += 1;
            name
        } else {
            source_name.to_string()
        };
        self.scope.insert(hir_name.clone(), ty);
        self.bindings
            .entry(source_name.to_string())
            .or_default()
            .push(hir_name.clone());
        hir_name
    }

    fn lower_scoped_stmts(&mut self, stmts: &[Stmt]) -> Result<Vec<HirStmt>, String> {
        let saved = self.bindings.clone();
        let lowered = self.lower_stmts(stmts);
        self.bindings = saved;
        lowered
    }

    fn lower_stmts(&mut self, stmts: &[Stmt]) -> Result<Vec<HirStmt>, String> {
        let mut out = Vec::new();
        for stmt in stmts {
            out.extend(self.lower_stmt_seq(stmt)?);
        }
        Ok(out)
    }

    fn infer_return_type(&self, body: &[HirStmt]) -> Result<HirType, String> {
        fn collect<'a>(stmts: &'a [HirStmt], out: &mut Vec<&'a HirExpr>, bare: &mut bool) {
            for stmt in stmts {
                match stmt {
                    HirStmt::Return(Some(value)) => out.push(value),
                    HirStmt::Return(None) => *bare = true,
                    HirStmt::If(_, then_body, else_body) => {
                        collect(then_body, out, bare);
                        collect(else_body, out, bare);
                    }
                    HirStmt::While(_, body) => collect(body, out, bare),
                    HirStmt::Try(body, _, catch_body) => {
                        collect(body, out, bare);
                        collect(catch_body, out, bare);
                    }
                    _ => {}
                }
            }
        }

        let mut values = Vec::new();
        let mut bare = false;
        collect(body, &mut values, &mut bare);
        if values.is_empty() {
            return Ok(HirType::Void);
        }
        if bare {
            return Err("function mixes value-returning and bare `return` statements".into());
        }
        let first = self.infer_expr_type(values[0])?;
        if first == HirType::Dynamic {
            return Ok(HirType::Dynamic);
        }
        for value in &values[1..] {
            let ty = self.infer_expr_type(value)?;
            if ty == HirType::Dynamic {
                return Ok(HirType::Dynamic);
            }
            if ty != first {
                return Err(format!(
                    "function returns incompatible types {first:?} and {ty:?}"
                ));
            }
        }
        Ok(first)
    }

    /// Normalizes a `for`/`while`/`if` body, which SWC represents as a
    /// single `Stmt` (either a `{ ... }` block or one bare statement), into
    /// a flat HIR statement list.
    fn lower_body(&mut self, stmt: &Stmt) -> Result<Vec<HirStmt>, String> {
        match stmt {
            Stmt::Block(block) => self.lower_scoped_stmts(&block.stmts),
            other => {
                let saved = self.bindings.clone();
                let lowered = self.lower_stmt_seq(other);
                self.bindings = saved;
                lowered
            }
        }
    }

    fn lower_stmt_seq(&mut self, stmt: &Stmt) -> Result<Vec<HirStmt>, String> {
        match stmt {
            Stmt::Return(ret) => {
                let value = match &ret.arg {
                    Some(arg) => {
                        let value = self.lower_expr(arg)?;
                        if self.ret_type == HirType::Void {
                            return Err("a void function cannot return a value".into());
                        }
                        Some(self.coerce_to_declared(&self.ret_type.clone(), value)?)
                    }
                    None => {
                        if !matches!(self.ret_type, HirType::Void | HirType::Dynamic) {
                            return Err(format!(
                                "bare `return` is not valid for return type {:?}",
                                self.ret_type
                            ));
                        }
                        None
                    }
                };
                Ok(vec![HirStmt::Return(value)])
            }
            Stmt::Expr(expr_stmt) => Ok(vec![HirStmt::Expr(self.lower_expr(&expr_stmt.expr)?)]),
            Stmt::Block(block) => self.lower_scoped_stmts(&block.stmts),
            Stmt::Decl(Decl::Var(var_decl)) => self.lower_var_decl(var_decl),

            Stmt::If(if_stmt) => {
                let cond = self.lower_expr(&if_stmt.test)?;
                self.expect_type(&HirType::Bool, &cond, "if condition")?;
                let then_branch = self.lower_body(&if_stmt.cons)?;
                let else_branch = match &if_stmt.alt {
                    Some(alt) => self.lower_body(alt)?,
                    None => Vec::new(),
                };
                Ok(vec![HirStmt::If(cond, then_branch, else_branch)])
            }

            Stmt::While(while_stmt) => {
                let cond = self.lower_expr(&while_stmt.test)?;
                self.expect_type(&HirType::Bool, &cond, "while condition")?;
                let body = self.lower_body(&while_stmt.body)?;
                Ok(vec![HirStmt::While(cond, body)])
            }

            Stmt::Break(break_stmt) => {
                if break_stmt.label.is_some() {
                    return Err("labeled `break` is not supported".into());
                }
                Ok(vec![HirStmt::Break])
            }

            Stmt::Continue(continue_stmt) => {
                if continue_stmt.label.is_some() {
                    return Err("labeled `continue` is not supported".into());
                }
                Ok(vec![HirStmt::Continue])
            }

            Stmt::For(for_stmt) => {
                let saved = self.bindings.clone();
                let lowered = (|| -> Result<Vec<HirStmt>, String> {
                    let mut out = Vec::new();
                    if let Some(init) = &for_stmt.init {
                        match init {
                            VarDeclOrExpr::VarDecl(var_decl) => {
                                out.extend(self.lower_var_decl(var_decl)?)
                            }
                            VarDeclOrExpr::Expr(expr) => {
                                out.push(HirStmt::Expr(self.lower_expr(expr)?))
                            }
                        }
                    }

                    let cond = match &for_stmt.test {
                        Some(test) => {
                            let cond = self.lower_expr(test)?;
                            self.expect_type(&HirType::Bool, &cond, "for condition")?;
                            cond
                        }
                        None => HirExpr::Lit(HirLit::Bool(true)),
                    };

                    let mut body = self.lower_body(&for_stmt.body)?;
                    if let Some(update) = &for_stmt.update {
                        let update = self.lower_expr(update)?;
                        body = inject_for_update_before_continue(body, &update);
                        body.push(HirStmt::Expr(update));
                    }

                    out.push(HirStmt::While(cond, body));
                    Ok(out)
                })();
                self.bindings = saved;
                lowered
            }

            Stmt::Throw(throw_stmt) => Ok(vec![HirStmt::Throw(self.lower_expr(&throw_stmt.arg)?)]),

            Stmt::Try(try_stmt) => {
                if try_stmt.handler.is_none() && try_stmt.finalizer.is_none() {
                    return Err("`try` needs a `catch` or `finally` block".into());
                }

                let mut body = self.lower_scoped_stmts(&try_stmt.block.stmts)?;
                let (catch_name, mut catch_body) = match &try_stmt.handler {
                    Some(handler) => {
                        let source_name = match &handler.param {
                            Some(Pat::Ident(binding)) => binding.id.sym.to_string(),
                            Some(_) => {
                                return Err(
                                    "only a simple identifier catch binding is supported".into(),
                                )
                            }
                            None => "_".to_string(),
                        };
                        let saved = self.bindings.clone();
                        let catch_name = self.bind_local(&source_name, HirType::Str);
                        let catch_body = self.lower_stmts(&handler.body.stmts)?;
                        self.bindings = saved;
                        (catch_name, catch_body)
                    }
                    None => {
                        let mut name = "__thaw_finally_exception".to_string();
                        while self.scope.contains_key(&name) {
                            name.push('_');
                        }
                        let name = self.bind_local(&name, HirType::Str);
                        let rethrow = HirStmt::Throw(HirExpr::Var(name.clone()));
                        (name, vec![rethrow])
                    }
                };

                let mut after_try = Vec::new();
                if let Some(finalizer) = &try_stmt.finalizer {
                    let finalizer = self.lower_scoped_stmts(&finalizer.stmts)?;
                    body = inject_finally_before_exits(body, &finalizer, false);
                    catch_body =
                        inject_finally_before_exits(catch_body, &finalizer, true);
                    after_try = finalizer;
                }
                let mut lowered = vec![HirStmt::Try(body, catch_name, catch_body)];
                lowered.extend(after_try);
                Ok(lowered)
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
                    Some(ann) => {
                        lower_ts_type(&ann.type_ann, self.interfaces, self.generic_interfaces)?
                    }
                    None => self.infer_expr_type(&value).map_err(|e| {
                        format!(
                            "cannot infer the type of `{name}`: {e} \
                             (add an explicit type annotation)"
                        )
                    })?,
                };
                let value = self.coerce_to_declared(&ty, value)?;

                let hir_name = self.bind_local(&name, ty.clone());
                Ok(HirStmt::Let(hir_name, ty, value))
            })
            .collect()
    }

    /// If `declared` is an object type and `value` is an object literal,
    /// reorders the literal's fields to match the declared field order and
    /// checks each field's type -- so codegen only ever has to deal with
    /// one canonical field order (the declared one), never the literal's
    /// source order. A no-op for every other combination.
    fn coerce_to_declared(&self, declared: &HirType, value: HirExpr) -> Result<HirExpr, String> {
        if *declared == HirType::Dynamic {
            return Ok(value);
        }
        let (HirType::Object(declared_fields), HirExpr::ObjectLit(lit_fields)) = (declared, &value)
        else {
            self.expect_type(declared, &value, "value")?;
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
                if *expected_ty != HirType::Dynamic && actual_ty != *expected_ty {
                    return Err(format!(
                        "field `{name}` has type {actual_ty:?}, expected {expected_ty:?}"
                    ));
                }
                Ok((name.clone(), field_value.clone()))
            })
            .collect::<Result<Vec<_>, String>>()?;

        Ok(HirExpr::ObjectLit(reordered))
    }

    fn expect_type(
        &self,
        expected: &HirType,
        value: &HirExpr,
        context: &str,
    ) -> Result<(), String> {
        let actual = self.infer_expr_type(value)?;
        if actual == HirType::Dynamic || *expected == HirType::Dynamic || actual == *expected {
            Ok(())
        } else {
            Err(format!(
                "{context} has type {actual:?}, expected {expected:?}"
            ))
        }
    }

    /// Infers the concrete native type of an expression. This is also the
    /// shared checker for assignments, returns, operators, indexes and call
    /// arguments, keeping unresolved/dynamic layouts out of LLVM lowering.
    fn infer_expr_type(&self, expr: &HirExpr) -> Result<HirType, String> {
        match expr {
            HirExpr::Lit(HirLit::F64(_)) => Ok(HirType::F64),
            HirExpr::Lit(HirLit::Str(_)) => Ok(HirType::Str),
            HirExpr::Lit(HirLit::Bool(_)) => Ok(HirType::Bool),
            HirExpr::Var(name) => self
                .scope
                .get(name)
                .cloned()
                .ok_or_else(|| format!("unknown variable `{name}`")),
            HirExpr::Assign(name, value) => {
                let expected = self
                    .scope
                    .get(name)
                    .cloned()
                    .ok_or_else(|| format!("unknown variable `{name}`"))?;
                self.expect_type(&expected, value, &format!("assignment to `{name}`"))?;
                Ok(expected)
            }
            HirExpr::BinOp(op, left, right) => {
                let left_ty = self.infer_expr_type(left)?;
                let right_ty = self.infer_expr_type(right)?;
                match op {
                    BinOp::EqEqEq => {
                        if left_ty != HirType::Dynamic
                            && right_ty != HirType::Dynamic
                            && left_ty != right_ty
                        {
                            return Err(format!(
                                "strict equality compares incompatible types {left_ty:?} and {right_ty:?}"
                            ));
                        }
                        Ok(HirType::Bool)
                    }
                    BinOp::Lt | BinOp::Gt => {
                        if !matches!(left_ty, HirType::F64 | HirType::Dynamic)
                            || !matches!(right_ty, HirType::F64 | HirType::Dynamic)
                        {
                            return Err(format!(
                                "numeric comparison requires F64 operands, got {left_ty:?} and {right_ty:?}"
                            ));
                        }
                        Ok(HirType::Bool)
                    }
                    BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div => {
                        if !matches!(left_ty, HirType::F64 | HirType::Dynamic)
                            || !matches!(right_ty, HirType::F64 | HirType::Dynamic)
                        {
                            return Err(format!(
                                "arithmetic requires F64 operands, got {left_ty:?} and {right_ty:?}"
                            ));
                        }
                        Ok(HirType::F64)
                    }
                }
            }
            HirExpr::Call(callee, args) => {
                let HirExpr::Var(name) = callee.as_ref() else {
                    return Err("cannot infer the type of a call through a non-name callee".into());
                };
                match name.as_str() {
                    "console.log" => return Ok(HirType::F64),
                    "fetch" => return Ok(HirType::Str),
                    "sleep" => return Ok(HirType::Promise(Box::new(HirType::Void))),
                    "JSON.parse" => return Ok(HirType::Json),
                    "JSON.stringify" => return Ok(HirType::Str),
                    // QuickJS-NG fallback path (docs/design/bridge.md
                    // section 7): `loadScript` evaluates JS source into
                    // the global engine context; `callDynamic` calls a
                    // top-level function it defined, by name, with `Json`
                    // args in and a `Json` result out.
                    "loadScript" => return Ok(HirType::Bool),
                    "callDynamic" => return Ok(HirType::Json),
                    "loadNativeAddon" => return Ok(HirType::Bool),
                    "loadNativeAddonEmbedded" => return Ok(HirType::Bool),
                    "callNativeAddon" => return Ok(HirType::Json),
                    _ => {}
                }
                if let Some(HirType::Function(params, ret)) = self.scope.get(name) {
                    if params.len() != args.len() {
                        return Err(format!(
                            "function value `{name}` expects {} argument(s), got {}",
                            params.len(),
                            args.len()
                        ));
                    }
                    return Ok(ret.as_ref().clone());
                }
                let signature = self.signatures.get(name).or_else(|| {
                    name.split_once("__thaw_")
                        .and_then(|(base, _)| self.signatures.get(base))
                        .filter(|signature| !signature.generic_type_params.is_empty())
                });
                match signature {
                    Some(sig) => {
                        if !sig.generic_type_params.is_empty() {
                            let actual = args
                                .iter()
                                .map(|arg| self.infer_expr_type(arg))
                                .collect::<Result<Vec<_>, _>>()?;
                            let types = infer_generic_type_tuple(sig, &actual)?;
                            let substitution = sig
                                .generic_type_params
                                .iter()
                                .cloned()
                                .zip(types)
                                .collect::<HashMap<_, _>>();
                            resolve_ts_type_with_substitution(
                                sig.generic_return_type
                                    .as_ref()
                                    .expect("generic return type"),
                                &substitution,
                                self.interfaces,
                                self.generic_interfaces,
                                &mut Vec::new(),
                            )
                        } else {
                            Ok(sig.ret.clone())
                        }
                    }
                    None => Err(format!("call to unknown function `{name}`")),
                }
            }
            HirExpr::DynamicCall(signature, _) => Ok(signature.ret.clone()),
            HirExpr::ArrayLit(values) => {
                for value in values {
                    self.expect_type(&HirType::F64, value, "array element")?;
                }
                Ok(HirType::Array(Box::new(HirType::F64)))
            }
            HirExpr::Index(arr, index) => {
                self.expect_type(&HirType::F64, index, "array index")?;
                match self.infer_expr_type(arr)? {
                    HirType::Array(elem) => Ok(*elem),
                    other => Err(format!("cannot index into a value of type {other:?}")),
                }
            }
            HirExpr::IndexAssign(arr, index, value) => {
                self.expect_type(&HirType::F64, index, "array index")?;
                let HirType::Array(element) = self.infer_expr_type(arr)? else {
                    return Err("index assignment target is not an array".into());
                };
                self.expect_type(&element, value, "array assignment")?;
                Ok(*element)
            }
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
                other => Err(format!(
                    "cannot access `.{field}` on a value of type {other:?}"
                )),
            },
            HirExpr::PropAssign(_, _, _, value) => self.infer_expr_type(value),
            HirExpr::JsonGet(_, _) | HirExpr::JsonIndex(_, _) => Ok(HirType::Json),
            HirExpr::JsonAsNumber(_) => Ok(HirType::F64),
            HirExpr::JsonAsString(_) => Ok(HirType::Str),
            HirExpr::JsonAsBool(_) => Ok(HirType::Bool),
            HirExpr::FfiCall(sig, _) => Ok(sig.ret.clone()),
            HirExpr::Await(inner) => match self.infer_expr_type(inner)? {
                HirType::Promise(value) => Ok(*value),
                // V1 user-defined async calls still have an already-unwrapped
                // signature until their coroutine frames are generalized.
                other => Ok(other),
            },
            // The Lambda node now preserves typed parameters and its body,
            // but function values do not have a native ABI until the next
            // callback-lowering phase. Keep the enclosing local dynamic
            // instead of discarding or pretending to know that ABI.
            HirExpr::Lambda(_, params, ret, _) => Ok(HirType::Function(
                params.iter().map(|param| param.ty.clone()).collect(),
                Box::new(ret.clone()),
            )),
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
            Expr::Ident(ident) => Ok(HirExpr::Var(
                self.resolve_binding(ident.sym.as_ref()),
            )),
            Expr::Paren(paren) => self.lower_expr(&paren.expr),

            Expr::Bin(bin) => {
                let op = lower_bin_op(bin.op)?;
                let lhs = self.lower_expr(&bin.left)?;
                let rhs = self.lower_expr(&bin.right)?;
                let value = HirExpr::BinOp(op, Box::new(lhs), Box::new(rhs));
                self.infer_expr_type(&value)?;
                Ok(value)
            }

            Expr::Call(call) => self.lower_call(call),

            Expr::Arrow(arrow) => self.lower_arrow(arrow),

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
                let value = HirExpr::ArrayLit(elems);
                self.infer_expr_type(&value)?;
                Ok(value)
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

    fn lower_arrow(&mut self, arrow: &swc_ecma_ast::ArrowExpr) -> Result<HirExpr, String> {
        if arrow.is_async || arrow.is_generator || arrow.type_params.is_some() {
            return Err(
                "async, generator, and generic arrow functions are not supported yet".into(),
            );
        }
        let source_params = arrow
            .params
            .iter()
            .map(|param| {
                lower_param(
                    param,
                    self.interfaces,
                    self.generic_interfaces,
                    false,
                    &HashMap::new(),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let declared_return = arrow
            .return_type
            .as_ref()
            .map(|ann| lower_ts_type(&ann.type_ann, self.interfaces, self.generic_interfaces))
            .transpose()?;

        let saved_scope = self.scope.clone();
        let saved_bindings = self.bindings.clone();
        let saved_return = self.ret_type.clone();
        let result = (|| {
            let mut params = Vec::with_capacity(source_params.len());
            for param in source_params {
                let name = self.bind_local(&param.name, param.ty.clone());
                params.push(HirParam { name, ty: param.ty });
            }
            self.ret_type = declared_return.clone().unwrap_or(HirType::Dynamic);
            let (body, inferred_return) = match arrow.body.as_ref() {
                ArrowFunctionBody::Expr(expr) => {
                    let body = self.lower_expr(expr)?;
                    if let Some(expected) = &declared_return {
                        self.expect_type(expected, &body, "arrow function return value")?;
                    }
                    let inferred = self.infer_expr_type(&body)?;
                    (body, inferred)
                }
                ArrowFunctionBody::FunctionBody(block) => {
                    let stmts = self.lower_stmts(&block.stmts)?;
                    let inferred = self.infer_return_type(&stmts)?;
                    (HirExpr::Block(stmts), inferred)
                }
            };
            let return_type = declared_return.clone().unwrap_or(inferred_return);
            let mut referenced = BTreeSet::new();
            collect_referenced_bindings(&body, &mut referenced);
            let captures = referenced
                .into_iter()
                .filter_map(|name| {
                    saved_scope
                        .get(&name)
                        .cloned()
                        .map(|ty| HirParam { name, ty })
                })
                .collect();
            Ok(HirExpr::Lambda(
                captures,
                params,
                return_type,
                Box::new(body),
            ))
        })();
        self.scope = saved_scope;
        self.bindings = saved_bindings;
        self.ret_type = saved_return;
        result
    }

    fn lower_object_lit(&mut self, obj_lit: &SwcObjectLit) -> Result<HirExpr, String> {
        let fields = obj_lit
            .props
            .iter()
            .map(|prop| {
                let PropOrSpread::Prop(prop) = prop else {
                    return Err(
                        "spread properties are not supported in object literals".to_string()
                    );
                };
                let Prop::KeyValue(KeyValueProp { key, value }) = prop.as_ref() else {
                    return Err(
                        "only `key: value` object literal properties are supported".to_string()
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
                self.expect_type(&HirType::F64, &index, "index expression")?;
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
                            Ok(HirExpr::PropAccess(
                                Box::new(obj),
                                obj_ty.clone(),
                                prop.sym.to_string(),
                            ))
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
            SimpleAssignTarget::Ident(binding) => {
                Ok(Target::Var(self.resolve_binding(binding.id.sym.as_ref())))
            }
            SimpleAssignTarget::Member(member) => match &member.prop {
                MemberProp::Computed(computed) => self.lower_index_target(member, computed),
                MemberProp::Ident(prop) => {
                    let obj = self.lower_expr(&member.obj)?;
                    let obj_ty = self.infer_expr_type(&obj)?;
                    match &obj_ty {
                        HirType::Object(fields)
                            if fields.iter().any(|(n, _)| n == prop.sym.as_str()) =>
                        {
                            Ok(Target::Prop(obj, obj_ty.clone(), prop.sym.to_string()))
                        }
                        other => Err(format!(
                            "cannot assign to `.{}` on a value of type {other:?}",
                            prop.sym
                        )),
                    }
                }
                _ => Err(
                    "only `arr[i] = ...` / `obj.field = ...` member assignment is supported".into(),
                ),
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
            Target::Index(array, index) => {
                self.expect_type(&HirType::F64, index, "array index")?;
                let HirType::Array(element) = self.infer_expr_type(array)? else {
                    return Err("index assignment target is not an array".into());
                };
                self.coerce_to_declared(&element, value)?
            }
            Target::Prop(_, other, field) => {
                return Err(format!(
                    "cannot assign to field `{field}` on value of type {other:?}"
                ));
            }
        };

        Ok(build_assign(target, value))
    }

    fn lower_update(&mut self, update: &swc_ecma_ast::UpdateExpr) -> Result<HirExpr, String> {
        // Phase 1/2 always yield the *new* value (prefix semantics), even
        // for postfix `i++`/`i--`. This only matters when the expression's
        // value is used, which doesn't happen in the
        // `for (...; ...; i++)` / bare `i++;` forms this is meant to support.
        let target = match update.arg.as_ref() {
            Expr::Ident(ident) => Target::Var(self.resolve_binding(ident.sym.as_ref())),
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
            Expr::Ident(ident) => self.resolve_binding(ident.sym.as_ref()),
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
            _ => return Err(
                "unsupported call target (only plain identifiers and console.log are supported)"
                    .into(),
            ),
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
        let local_function = self.scope.get(&callee_name).and_then(|ty| match ty {
            HirType::Function(params, ret) => Some((params.clone(), ret.as_ref().clone())),
            _ => None,
        });
        let param_types = signature
            .as_ref()
            .map(|sig| sig.params.clone())
            .or_else(|| local_function.as_ref().map(|(params, _)| params.clone()));

        if let Some(params) = &param_types {
            if call.args.len() != params.len() {
                return Err(format!(
                    "function `{callee_name}` expects {} argument(s), got {}",
                    params.len(),
                    call.args.len()
                ));
            }
        }

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
                    Some(_)
                        if signature
                            .as_ref()
                            .is_some_and(|sig| !sig.generic_type_params.is_empty()) =>
                    {
                        Ok(value)
                    }
                    Some(declared) => self.coerce_to_declared(declared, value).map_err(|error| {
                        format!("argument {} of `{callee_name}` is invalid: {error}", i + 1)
                    }),
                    None => Ok(value),
                }
            })
            .collect::<Result<Vec<_>, _>>()?;

        let generic_types = if let Some(signature) = signature
            .as_ref()
            .filter(|signature| !signature.generic_type_params.is_empty())
        {
            let actual = args
                .iter()
                .map(|arg| self.infer_expr_type(arg))
                .collect::<Result<Vec<_>, _>>()?;
            let types = infer_generic_type_tuple(signature, &actual)
                .map_err(|error| format!("call to generic function `{callee_name}`: {error}"))?;
            if !types.iter().any(|ty| *ty == HirType::Dynamic) {
                for ty in &types {
                    if !supports_generic_native_layout(ty) {
                        return Err(format!(
                            "generic function `{callee_name}` cannot specialize for native layout {ty:?}"
                        ));
                    }
                }
            }
            Some(types)
        } else {
            None
        };

        if let (Some(signature), Some(constraints)) = (&signature, self.call_constraints) {
            if let Some(types) = &generic_types {
                constraints
                    .borrow_mut()
                    .push(CallConstraint::Generic(callee_name.clone(), types.clone()));
            } else {
                for (index, (declared, value)) in signature.params.iter().zip(&args).enumerate() {
                    if *declared == HirType::Dynamic {
                        let actual = self.infer_expr_type(value)?;
                        constraints.borrow_mut().push(CallConstraint::Parameter(
                            callee_name.clone(),
                            index,
                            actual,
                            (call.span.lo.0, call.span.hi.0),
                        ));
                    }
                }
            }
        }

        if let Some(sig) = signature.clone().filter(|sig| sig.is_extern) {
            if let Some((backend, symbol)) = dynamic_symbol(&callee_name) {
                return Ok(HirExpr::DynamicCall(
                    DynamicSignature {
                        backend,
                        symbol,
                        params: sig.params,
                        ret: sig.ret,
                    },
                    args,
                ));
            }
            let ffi_signature = FfiSignature {
                symbol: callee_name,
                params: sig.params,
                ret: sig.ret,
                error_abi: FfiErrorAbi::Direct,
                return_ownership: FfiOwnership::Borrowed,
                error_ownership: FfiOwnership::Borrowed,
            };
            return Ok(HirExpr::FfiCall(ffi_signature, args));
        }

        let lowered_name = if generic_types
            .as_ref()
            .is_some_and(|types| !types.iter().any(|ty| *ty == HirType::Dynamic))
        {
            let param_types = args
                .iter()
                .map(|arg| self.infer_expr_type(arg))
                .collect::<Result<Vec<_>, _>>()?;
            specialized_generic_name(&callee_name, &param_types)
        } else {
            callee_name
        };
        Ok(HirExpr::Call(Box::new(HirExpr::Var(lowered_name)), args))
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
        let error = lower_module(&module).unwrap_err();
        assert!(error.contains("cannot infer parameter"));
        assert!(error.contains("at bytes"));
    }

    #[test]
    fn infers_unannotated_parameters_from_call_sites() {
        let program = lower(
            "function identity(value) { return value; } function main(): number { return identity(42); }",
        );
        let identity = &program.functions[0];
        assert_eq!(identity.params[0].ty, HirType::F64);
        assert_eq!(identity.ret, HirType::F64);
    }

    #[test]
    fn propagates_parameter_constraints_through_forward_call_chains() {
        let program = lower(
            "function first(value) { return second(value); } function second(value) { return value; } function main(): string { return first(\"ok\"); }",
        );
        assert_eq!(program.functions[0].params[0].ty, HirType::Str);
        assert_eq!(program.functions[0].ret, HirType::Str);
        assert_eq!(program.functions[1].params[0].ty, HirType::Str);
        assert_eq!(program.functions[1].ret, HirType::Str);
    }

    #[test]
    fn rejects_conflicting_call_site_parameter_constraints() {
        let module = thaw_parser::parse_typescript(
            "function identity(value) { return value; } function main(): void { identity(1); identity(\"x\"); }",
        )
        .unwrap();
        let error = lower_module(&module).unwrap_err();
        assert!(
            error.contains("conflicting inferred types"),
            "unexpected error: {error}"
        );
        assert!(error.contains("at bytes"));
    }

    #[test]
    fn structured_diagnostic_resolves_file_line_and_column() {
        let source = "function identity(value) { return value; }\nfunction main(): void { identity(1); identity(\"x\"); }";
        let (module, source_map) = thaw_parser::parse_typescript_with_source_map(source).unwrap();
        let diagnostic =
            lower_module_with_source_map(&module, &source_map, "example.ts").unwrap_err();
        assert!(diagnostic.message.contains("conflicting inferred types"));
        let range = diagnostic
            .range
            .as_ref()
            .expect("diagnostic should carry a range");
        assert_eq!(range.file, "example.ts");
        assert_eq!(range.line, 2);
        assert!(range.column > 1);
        assert!(diagnostic.to_string().starts_with("example.ts:2:"));
    }

    #[test]
    fn monomorphizes_a_generic_function_from_its_call_site() {
        let program = lower(
            "function identity<T>(value: T): T { return value; } function main(): string { return identity(\"ok\"); }",
        );
        let identity = program
            .functions
            .iter()
            .find(|function| function.name == "identity__thaw_str")
            .unwrap();
        assert_eq!(identity.params[0].ty, HirType::Str);
        assert_eq!(identity.ret, HirType::Str);
    }

    #[test]
    fn creates_distinct_native_instantiations_for_polymorphic_uses() {
        let program = lower(
            "function identity<T>(value: T): T { return value; } function main(): void { identity(1); identity(2); identity(\"x\"); }",
        );
        assert_eq!(
            program
                .functions
                .iter()
                .filter(|function| function.name == "identity__thaw_f64")
                .count(),
            1
        );
        assert_eq!(
            program
                .functions
                .iter()
                .filter(|function| function.name == "identity__thaw_str")
                .count(),
            1
        );
    }

    #[test]
    fn specializes_multiple_generic_arguments_as_one_call_tuple() {
        let program = lower(
            r#"
            interface Pair<T, U> { first: T; second: U; }
            function chooseFirst<T, U>(first: T, second: U): T { return first; }
            function makePair<T, U>(first: T, second: U): Pair<T, U> {
                return { first: first, second: second };
            }
            function main(): void {
                console.log(chooseFirst(1, "ignored"));
                const pair = makePair("left", 2);
                console.log(pair.first);
                console.log(pair.second);
            }
            "#,
        );
        let choose = program
            .functions
            .iter()
            .find(|function| function.name == "chooseFirst__thaw_f64__str")
            .expect("chooseFirst<number, string> specialization");
        assert_eq!(choose.params[0].ty, HirType::F64);
        assert_eq!(choose.params[1].ty, HirType::Str);
        assert_eq!(choose.ret, HirType::F64);

        let pair = program
            .functions
            .iter()
            .find(|function| function.name == "makePair__thaw_str__f64")
            .expect("makePair<string, number> specialization");
        assert_eq!(
            pair.ret,
            HirType::Object(vec![
                ("first".into(), HirType::Str),
                ("second".into(), HirType::F64),
            ])
        );
        assert!(
            matches!(&pair.body[0], HirStmt::Return(Some(HirExpr::ObjectLit(fields))) if
            fields[0].0 == "first" && fields[1].0 == "second")
        );
    }

    #[test]
    fn deduplicates_multi_argument_instantiations_and_supports_forward_references() {
        let program = lower(
            r#"
            function main(): void {
                chooseFirst(1, "a");
                chooseFirst(2, "b");
                chooseFirst("x", 3);
            }
            function chooseFirst<T, U>(first: T, second: U): T { return first; }
            "#,
        );
        assert_eq!(
            program
                .functions
                .iter()
                .filter(|function| { function.name == "chooseFirst__thaw_f64__str" })
                .count(),
            1
        );
        assert_eq!(
            program
                .functions
                .iter()
                .filter(|function| { function.name == "chooseFirst__thaw_str__f64" })
                .count(),
            1
        );
    }

    #[test]
    fn enforces_repeated_generic_type_constraints_across_arguments() {
        let module = thaw_parser::parse_typescript(
            r#"
            function same<T>(left: T, right: T): T { return left; }
            function main(): void { same(1, "wrong"); }
            "#,
        )
        .unwrap();
        let error = lower_module(&module).unwrap_err();
        assert!(error.contains("conflicting call-site types"), "{error}");
        assert!(error.contains("F64") && error.contains("Str"), "{error}");
    }

    #[test]
    fn enforces_repeated_generic_constraints_inside_arrays_and_interfaces() {
        let array = thaw_parser::parse_typescript(
            r#"
            function sameArrays<T>(left: T[], right: T[]): T[] { return left; }
            function main(): void { sameArrays([1], [2]); }
            "#,
        )
        .unwrap();
        assert!(lower_module(&array).is_ok());

        let pair = thaw_parser::parse_typescript(
            r#"
            interface Pair<T, U> { first: T; second: U; }
            function diagonal<T>(value: Pair<T, T>): T { return value.first; }
            function main(): void { diagonal({ first: 1, second: "wrong" }); }
            "#,
        )
        .unwrap();
        let error = lower_module(&pair).unwrap_err();
        assert!(error.contains("conflicting call-site types"), "{error}");
    }

    #[test]
    fn diagnoses_uninferable_and_unsupported_generic_layouts() {
        let uninferable = thaw_parser::parse_typescript(
            r#"
            function phantom<T, U>(value: T): T { return value; }
            function main(): void { phantom(1); }
            "#,
        )
        .unwrap();
        let error = lower_module(&uninferable).unwrap_err();
        assert!(
            error.contains("cannot infer generic type parameter `U`"),
            "{error}"
        );

        let unsupported = thaw_parser::parse_typescript(
            r#"
            function identity<T>(value: T): T { return value; }
            function main(): void { identity(JSON.parse("null")); }
            "#,
        )
        .unwrap();
        let error = lower_module(&unsupported).unwrap_err();
        assert!(
            error.contains("cannot specialize for native layout Json"),
            "{error}"
        );
    }

    #[test]
    fn propagates_specializations_through_generic_function_calls() {
        let program = lower(
            r#"
            function forward<T, U>(first: T, second: U): T {
                return chooseFirst(first, second);
            }
            function chooseFirst<T, U>(first: T, second: U): T {
                return first;
            }
            function main(): void { console.log(forward(42, "unused")); }
            "#,
        );
        let forward = program
            .functions
            .iter()
            .find(|function| function.name == "forward__thaw_f64__str")
            .expect("outer specialization");
        assert!(format!("{:?}", forward.body).contains("chooseFirst__thaw_f64__str"));
        assert_eq!(
            program
                .functions
                .iter()
                .filter(|function| function.name == "chooseFirst__thaw_f64__str")
                .count(),
            1
        );
    }

    #[test]
    fn specializes_type_variables_nested_in_arrays_and_objects() {
        let program = lower(
            r#"
            interface Box<T> { value: T; }
            interface Wrapper<T> { boxed: Box<T>; }
            function sameArray<T>(value: T[]): T[] { return value; }
            function sameBox<T>(value: { value: T }): { value: T } { return value; }
            function namedBox<T>(value: Box<T>): Box<T> { return value; }
            function wrapped<T>(value: Wrapper<T>): Wrapper<T> { return value; }
            function main(): void {
                sameArray([1, 2]);
                sameBox({ value: 3 });
                namedBox({ value: 4 });
                wrapped({ boxed: { value: 5 } });
            }
            "#,
        );
        assert!(program
            .functions
            .iter()
            .any(|function| function.name == "sameArray__thaw_array_f64"));
        assert!(program
            .functions
            .iter()
            .any(|function| function.name == "sameBox__thaw_object_value_f64"));
        assert!(program
            .functions
            .iter()
            .any(|function| function.name == "namedBox__thaw_object_value_f64"));
        assert!(program
            .functions
            .iter()
            .any(|function| { function.name == "wrapped__thaw_object_boxed_object_value_f64" }));
    }

    #[test]
    fn specializes_named_structures_with_multiple_type_parameters() {
        let program = lower(
            r#"
            interface Pair<T, U> { first: T; second: U; }
            function samePair<T, U>(value: Pair<T, U>): Pair<T, U> { return value; }
            function main(): void { samePair({ first: 1, second: "two" }); }
            "#,
        );
        let pair = program
            .functions
            .iter()
            .find(|function| function.name == "samePair__thaw_object_first_f64_second_str")
            .expect("Pair<number, string> specialization");
        assert_eq!(
            pair.params[0].ty,
            HirType::Object(vec![
                ("first".into(), HirType::F64),
                ("second".into(), HirType::Str),
            ])
        );
        assert_eq!(pair.ret, pair.params[0].ty);
    }

    #[test]
    fn specializes_generic_property_projections() {
        let program = lower(
            r#"
            interface Pair<T, U> { first: T; second: U; }
            interface Box<T> { value: T; }
            interface Wrapper<T> { boxed: Box<T>; }
            function first<T, U>(value: Pair<T, U>): T { return value.first; }
            function unbox<T>(value: Wrapper<T>): T { return value.boxed.value; }
            function main(): void {
                const n = first({ first: 1, second: "two" });
                const deep = unbox({ boxed: { value: 3 } });
            }
            "#,
        );
        let first = program
            .functions
            .iter()
            .find(|function| function.name == "first__thaw_object_first_f64_second_str")
            .unwrap();
        assert_eq!(first.ret, HirType::F64);
        let unbox = program
            .functions
            .iter()
            .find(|function| function.name == "unbox__thaw_object_boxed_object_value_f64")
            .unwrap();
        assert_eq!(unbox.ret, HirType::F64);
    }

    #[test]
    fn infers_unannotated_function_return_types_through_forward_calls() {
        let program =
            lower("function first() { return second(); } function second() { return 42; }");
        assert_eq!(program.functions[0].ret, HirType::F64);
        assert_eq!(program.functions[1].ret, HirType::F64);
    }

    #[test]
    fn infers_void_for_an_unannotated_function_without_value_returns() {
        let program = lower("function log() { console.log(1); }");
        assert_eq!(program.functions[0].ret, HirType::Void);
    }

    #[test]
    fn rejects_incompatible_return_types() {
        let module = thaw_parser::parse_typescript(
            "function choose(flag: boolean) { if (flag) return 1; return \"no\"; }",
        )
        .unwrap();
        let error = lower_module(&module).unwrap_err();
        assert!(
            error.contains("incompatible types"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn rejects_a_wrong_call_argument_type_and_arity() {
        let wrong_type = thaw_parser::parse_typescript(
            "function square(value: number): number { return value * value; } function main(): number { return square(\"x\"); }",
        )
        .unwrap();
        let error = lower_module(&wrong_type).unwrap_err();
        assert!(error.contains("argument 1"), "unexpected error: {error}");

        let wrong_arity = thaw_parser::parse_typescript(
            "function square(value: number): number { return value * value; } function main(): number { return square(); }",
        )
        .unwrap();
        let error = lower_module(&wrong_arity).unwrap_err();
        assert!(
            error.contains("expects 1 argument"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn rejects_wrong_assignment_and_declared_return_types() {
        let assignment = thaw_parser::parse_typescript(
            "function main(): void { let value = 1; value = \"x\"; }",
        )
        .unwrap();
        assert!(lower_module(&assignment)
            .unwrap_err()
            .contains("expected F64"));

        let returned =
            thaw_parser::parse_typescript("function main(): number { return \"x\"; }").unwrap();
        assert!(lower_module(&returned)
            .unwrap_err()
            .contains("expected F64"));
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
    fn classic_for_continue_runs_the_update_but_nested_loop_continue_does_not() {
        let program = lower(
            r#"function main(): void {
                for (let i = 0; i < 3; i++) {
                    while (i < 1) { continue; }
                    if (i === 1) { continue; }
                    console.log(i);
                }
            }"#,
        );
        let HirStmt::While(_, body) = &program.functions[0].body[1] else {
            panic!("expected desugared for loop");
        };
        let HirStmt::While(_, nested_body) = &body[0] else {
            panic!("expected nested while loop");
        };
        assert_eq!(nested_body, &[HirStmt::Continue]);
        let HirStmt::If(_, then_body, _) = &body[1] else {
            panic!("expected conditional continue");
        };
        assert!(matches!(
            then_body.as_slice(),
            [HirStmt::Expr(HirExpr::Assign(name, _)), HirStmt::Continue] if name == "i"
        ));
        assert!(matches!(
            body.last(),
            Some(HirStmt::Expr(HirExpr::Assign(name, _))) if name == "i"
        ));
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
    fn lowers_finally_onto_normal_return_and_rethrow_paths() {
        let program = lower(
            r#"function f(): string {
                try {
                    return "ok";
                } catch (e) {
                    throw e;
                } finally {
                    console.log("cleanup");
                }
            }
            function main(): void { console.log(f()); }"#,
        );
        let HirStmt::Try(body, _, catch_body) = &program.functions[0].body[0] else {
            panic!("expected lowered try");
        };
        assert!(matches!(body[0], HirStmt::Expr(_)));
        assert!(matches!(body[1], HirStmt::Return(_)));
        assert!(matches!(catch_body[0], HirStmt::Expr(_)));
        assert!(matches!(catch_body[1], HirStmt::Throw(_)));
        assert!(matches!(program.functions[0].body[1], HirStmt::Expr(_)));
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
    fn infers_await_sleep_as_void() {
        let program = lower(
            r#"async function main(): Promise<void> {
                await sleep(1);
            }"#,
        );
        let HirStmt::Expr(HirExpr::Await(inner)) = &program.functions[0].body[0] else {
            panic!("expected await expression");
        };
        assert_eq!(
            FnLowerer::new(
                &HashMap::new(),
                &HashMap::new(),
                &GenericInterfaces::new(),
                HirType::Void,
                None,
            )
            .infer_expr_type(&HirExpr::Await(inner.clone()))
            .unwrap(),
            HirType::Void
        );
    }

    #[test]
    fn rejects_async_function_not_declared_as_returning_promise() {
        let module =
            thaw_parser::parse_typescript("async function f(): number { return 1; }").unwrap();
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
    fn lowers_load_script_and_call_dynamic() {
        let program = lower(
            r#"function main(): void {
                const ok: boolean = loadScript("function add(a,b){return a+b;}");
                const args = JSON.parse("[1,2]");
                const result = callDynamic("add", args);
                console.log(Number(result));
            }"#,
        );
        let f = &program.functions[0];
        assert_eq!(
            f.body[0],
            HirStmt::Let(
                "ok".into(),
                HirType::Bool,
                HirExpr::Call(
                    Box::new(HirExpr::Var("loadScript".into())),
                    vec![HirExpr::Lit(HirLit::Str(
                        "function add(a,b){return a+b;}".into()
                    ))],
                ),
            )
        );
        assert_eq!(
            f.body[2],
            HirStmt::Let(
                "result".into(),
                HirType::Json,
                HirExpr::Call(
                    Box::new(HirExpr::Var("callDynamic".into())),
                    vec![
                        HirExpr::Lit(HirLit::Str("add".into())),
                        HirExpr::Var("args".into()),
                    ],
                ),
            )
        );
    }

    #[test]
    fn lowers_json_type_annotation() {
        let program = lower(
            r#"function wrap(args: Json): Json {
                return args;
            }
            function main(): void {}"#,
        );
        let f = &program.functions[0];
        assert_eq!(
            f.params,
            vec![HirParam {
                name: "args".into(),
                ty: HirType::Json
            }]
        );
        assert_eq!(f.ret, HirType::Json);
    }

    #[test]
    fn rejects_number_conversion_on_a_non_json_value() {
        let module =
            thaw_parser::parse_typescript("function main(): void { const x: number = Number(1); }")
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

        assert_eq!(
            program.functions.len(),
            1,
            "the ambient decl has no body to lower"
        );
        assert_eq!(
            program.extern_functions,
            vec![crate::FfiSignature {
                symbol: "native_add".into(),
                params: vec![HirType::F64, HirType::F64],
                ret: HirType::F64,
                error_abi: crate::FfiErrorAbi::Direct,
                return_ownership: crate::FfiOwnership::Borrowed,
                error_ownership: crate::FfiOwnership::Borrowed,
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
                        error_abi: crate::FfiErrorAbi::Direct,
                        return_ownership: crate::FfiOwnership::Borrowed,
                        error_ownership: crate::FfiOwnership::Borrowed,
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

        let point_ty =
            HirType::Object(vec![("x".into(), HirType::F64), ("y".into(), HirType::F64)]);

        let dist = &program.functions[0];
        assert_eq!(
            dist.params,
            vec![HirParam {
                name: "p".into(),
                ty: point_ty.clone()
            }]
        );

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

        let point_ty =
            HirType::Object(vec![("x".into(), HirType::F64), ("y".into(), HirType::F64)]);
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
        let box_ty = HirType::Object(vec![(
            "items".into(),
            HirType::Array(Box::new(HirType::F64)),
        )]);
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

    #[test]
    fn renames_shadowed_block_locals_and_restores_outer_binding() {
        let program = lower(
            r#"function main(): void {
                let value = 1;
                if (value < 2) {
                    let value = 2;
                    value = value + 1;
                    console.log(value);
                }
                console.log(value);
            }"#,
        );
        let body = &program.functions[0].body;
        assert!(matches!(&body[0], HirStmt::Let(name, _, _) if name == "value"));
        let HirStmt::If(_, then_body, _) = &body[1] else {
            panic!("expected lowered if");
        };
        assert!(matches!(&then_body[0], HirStmt::Let(name, _, _) if name == "value__thaw_0"));
        assert!(
            matches!(&then_body[1], HirStmt::Expr(HirExpr::Assign(name, _)) if name == "value__thaw_0")
        );
        assert!(format!("{:?}", then_body[2]).contains("value__thaw_0"));
        assert!(format!("{:?}", body[2]).contains("Var(\"value\")"));
    }

    #[test]
    fn renames_catch_binding_that_shadows_an_outer_local() {
        let program = lower(
            r#"function main(): void {
                const error = "outer";
                try {
                    throw "inner";
                } catch (error) {
                    console.log(error);
                }
                console.log(error);
            }"#,
        );
        let body = &program.functions[0].body;
        let HirStmt::Try(_, catch_name, catch_body) = &body[1] else {
            panic!("expected lowered try");
        };
        assert_eq!(catch_name, "error__thaw_0");
        assert!(format!("{:?}", catch_body).contains("error__thaw_0"));
        assert!(format!("{:?}", body[2]).contains("Var(\"error\")"));
    }

    #[test]
    fn lowers_typed_arrow_functions_and_restores_the_outer_scope() {
        let program = lower(
            r#"function main(): void {
                const value: number = 10;
                const callback = (value: number): number => value + 1;
                console.log(value);
            }"#,
        );
        let body = &program.functions[0].body;
        let HirStmt::Let(
            _,
            HirType::Function(param_types, return_type),
            HirExpr::Lambda(captures, params, lambda_return, lambda_body),
        ) = &body[1]
        else {
            panic!("expected a lowered arrow function");
        };
        assert!(captures.is_empty());
        assert_eq!(param_types, &[HirType::F64]);
        assert_eq!(return_type.as_ref(), &HirType::F64);
        assert_eq!(lambda_return, &HirType::F64);
        assert_eq!(
            params,
            &[HirParam {
                name: "value__thaw_0".into(),
                ty: HirType::F64
            }]
        );
        assert!(matches!(
            lambda_body.as_ref(),
            HirExpr::BinOp(_, left, _) if matches!(left.as_ref(), HirExpr::Var(name) if name == "value__thaw_0")
        ));
        assert!(format!("{:?}", body[2]).contains("Var(\"value\")"));
    }

    #[test]
    fn lowers_a_typed_arrow_block_body() {
        let program = lower(
            r#"function main(): void {
                const callback = (path: string): string => { return path; };
            }"#,
        );
        let HirStmt::Let(_, _, HirExpr::Lambda(captures, params, return_type, lambda_body)) =
            &program.functions[0].body[0]
        else {
            panic!("expected a lowered arrow function");
        };
        assert!(captures.is_empty());
        assert_eq!(params[0].ty, HirType::Str);
        assert_eq!(return_type, &HirType::Str);
        assert!(matches!(
            lambda_body.as_ref(),
            HirExpr::Block(stmts)
                if matches!(&stmts[0], HirStmt::Return(Some(HirExpr::Var(name))) if name == "path")
        ));
    }

    #[test]
    fn lowers_function_type_annotations_and_calls_through_function_values() {
        let program = lower(
            r#"function main(): void {
                const increment: (value: number) => number =
                    (value: number): number => value + 1;
                console.log(increment(41));
            }"#,
        );
        let function_type = HirType::Function(vec![HirType::F64], Box::new(HirType::F64));
        assert!(matches!(
            &program.functions[0].body[0],
            HirStmt::Let(name, ty, HirExpr::Lambda(_, _, _, _))
                if name == "increment" && ty == &function_type
        ));
        assert!(format!("{:?}", program.functions[0].body[1])
            .contains("Call(Var(\"increment\"), [Lit(F64(41.0))])"));
    }

    #[test]
    fn records_arrow_capture_names_and_types() {
        let program = lower(
            r#"function main(): void {
                const base: number = 40;
                const add = (value: number): number => base + value;
                console.log(add(2));
            }"#,
        );
        let HirStmt::Let(_, _, HirExpr::Lambda(captures, _, _, _)) = &program.functions[0].body[1]
        else {
            panic!("expected captured lambda");
        };
        assert_eq!(
            captures,
            &[HirParam {
                name: "base".into(),
                ty: HirType::F64,
            }]
        );
    }
}
