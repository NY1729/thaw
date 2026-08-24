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
    ArrowFunctionBody, AssignOp, AssignTarget, AwaitExpr, BinaryOp, CallExpr, Callee,
    ComputedPropName, Decl, Expr, FnDecl, ForHead, KeyValueProp, Lit, MemberExpr, MemberProp,
    Module, ModuleItem, ObjectLit as SwcObjectLit, ObjectPatProp, OptChainBase, Pat, Prop,
    PropName, PropOrSpread, SimpleAssignTarget, Stmt, TsFnOrConstructorType, TsFnParam,
    TsInterfaceDecl, TsKeywordTypeKind, TsType, TsTypeElement, UnaryOp, UpdateOp, VarDecl,
    VarDeclOrExpr,
};
use swc_ecma_visit::{Visit, VisitWith};

use crate::{
    BinOp, DynamicBackend, DynamicSignature, FfiAggregateAbi, FfiCallingConvention, FfiErrorAbi,
    FfiOwnership, FfiSignature, FfiStringAbi, HirExpr, HirFunction, HirLit, HirParam, HirProgram,
    HirStmt, HirType, Symbol,
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
    is_async: bool,
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
                        is_async: func.is_async,
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
                    if types.contains(&HirType::Dynamic) {
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
            param_string_abis: vec![FfiStringAbi::NullTerminated; sig.params.len()],
            return_string_abi: FfiStringAbi::NullTerminated,
            calling_convention: FfiCallingConvention::C,
            aggregate_return_abi: FfiAggregateAbi::Internal,
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
                if !nested_types.contains(&HirType::Dynamic) {
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
        HirType::Tuple(elements) => elements.iter().all(supports_generic_native_layout),
        HirType::Object(fields) => fields
            .iter()
            .all(|(_, ty)| supports_generic_native_layout(ty)),
        _ => false,
    }
}

fn promise_settled_result_type(value: HirType) -> HirType {
    HirType::Object(vec![
        ("status".into(), HirType::Str),
        ("value".into(), value),
        ("reason".into(), HirType::Str),
    ])
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
            HirType::Tuple(elements) => format!(
                "tuple_{}",
                elements
                    .iter()
                    .map(fingerprint)
                    .collect::<Vec<_>>()
                    .join("_")
            ),
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
    let mut body = Vec::new();
    for (source, param) in func.params.iter().zip(&params) {
        if !matches!(source.pat, Pat::Ident(_)) {
            lowerer.lower_binding_pattern(
                &source.pat,
                HirExpr::Var(param.name.clone()),
                &param.ty,
                &mut body,
            )?;
        }
    }
    body.extend(lowerer.lower_stmts(&body_block.stmts)?);
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
    let (name, type_ann) = match pat {
        Pat::Ident(binding) => (binding.id.sym.to_string(), binding.type_ann.as_ref()),
        Pat::Object(pattern) => (
            format!("__thaw_param_{}", pattern.span.lo.0),
            pattern.type_ann.as_ref(),
        ),
        Pat::Array(pattern) => (
            format!("__thaw_param_{}", pattern.span.lo.0),
            pattern.type_ann.as_ref(),
        ),
        _ => return Err("unsupported function parameter pattern".into()),
    };
    let ty = match type_ann {
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
        TsType::TsTupleType(tuple) => Ok(HirType::Tuple(
            tuple
                .elem_types
                .iter()
                .map(|element| lower_ts_type(&element.ty, interfaces, generic_interfaces))
                .collect::<Result<Vec<_>, _>>()?,
        )),
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
            if ref_name == Some("JsValue") && ty_ref.type_params.is_none() {
                return Ok(HirType::JsValue);
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
        TsType::TsTupleType(tuple) => Ok(HirType::Tuple(
            tuple
                .elem_types
                .iter()
                .map(|element| {
                    resolve_ts_type_with_substitution(
                        &element.ty,
                        substitution,
                        interfaces,
                        generic_interfaces,
                        in_progress,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?,
        )),
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
        HirExpr::BinOp(_, left, right)
        | HirExpr::Index(left, right)
        | HirExpr::TypedIndex(left, right, _) => {
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
        | HirExpr::AwaitPromise(value, _)
        | HirExpr::PromiseNew(value, _, _)
        | HirExpr::PromiseAllArray(value, _)
        | HirExpr::PromiseRaceArray(value, _)
        | HirExpr::PromiseAnyArray(value, _)
        | HirExpr::PromiseAllSettledArray(value, _)
        | HirExpr::ArrayLen(value)
        | HirExpr::JsonAsNumber(value)
        | HirExpr::JsonAsString(value)
        | HirExpr::JsonAsBool(value) => collect_referenced_bindings(value, names),
        HirExpr::Lambda(captures, _, _, _) => {
            names.extend(captures.iter().map(|capture| capture.name.clone()));
        }
        HirExpr::PromiseThen(source, callback, _, _, _, _) => {
            collect_referenced_bindings(source, names);
            collect_referenced_bindings(callback, names);
        }
        HirExpr::PromiseFinally(source, callback, _, _) => {
            collect_referenced_bindings(source, names);
            collect_referenced_bindings(callback, names);
        }
        HirExpr::Block(stmts) => collect_stmt_bindings(stmts, names),
        HirExpr::FfiCall(_, args)
        | HirExpr::DynamicCall(_, args)
        | HirExpr::ArrayLit(args)
        | HirExpr::ArrayConcat(args, _)
        | HirExpr::PromiseAll(args, _)
        | HirExpr::PromiseAllTuple(args, _)
        | HirExpr::PromiseRace(args, _)
        | HirExpr::PromiseAny(args, _)
        | HirExpr::PromiseAllSettled(args, _) => {
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
        HirExpr::Lit(_) | HirExpr::EnvVar(_) | HirExpr::FunctionRef(..) => {}
    }
}

fn contains_await(expr: &HirExpr) -> bool {
    match expr {
        HirExpr::Await(_) | HirExpr::AwaitPromise(_, _) => true,
        HirExpr::BinOp(_, left, right)
        | HirExpr::Index(left, right)
        | HirExpr::TypedIndex(left, right, _)
        | HirExpr::JsonIndex(left, right) => contains_await(left) || contains_await(right),
        HirExpr::Call(callee, args) => contains_await(callee) || args.iter().any(contains_await),
        HirExpr::PromiseAll(values, _)
        | HirExpr::PromiseAllTuple(values, _)
        | HirExpr::PromiseRace(values, _)
        | HirExpr::PromiseAny(values, _)
        | HirExpr::PromiseAllSettled(values, _)
        | HirExpr::FfiCall(_, values)
        | HirExpr::DynamicCall(_, values)
        | HirExpr::ArrayLit(values)
        | HirExpr::ArrayConcat(values, _) => values.iter().any(contains_await),
        HirExpr::PromiseAllArray(value, _)
        | HirExpr::PromiseRaceArray(value, _)
        | HirExpr::PromiseAnyArray(value, _)
        | HirExpr::PromiseAllSettledArray(value, _)
        | HirExpr::PromiseNew(value, _, _)
        | HirExpr::Assign(_, value)
        | HirExpr::ArrayLen(value)
        | HirExpr::PropAccess(value, _, _)
        | HirExpr::JsonGet(value, _)
        | HirExpr::JsonAsNumber(value)
        | HirExpr::JsonAsString(value)
        | HirExpr::JsonAsBool(value) => contains_await(value),
        HirExpr::PromiseThen(source, callback, _, _, _, _)
        | HirExpr::PromiseFinally(source, callback, _, _) => {
            contains_await(source) || contains_await(callback)
        }
        HirExpr::IndexAssign(array, index, value) => {
            contains_await(array) || contains_await(index) || contains_await(value)
        }
        HirExpr::PropAssign(object, _, _, value) => contains_await(object) || contains_await(value),
        HirExpr::ObjectLit(fields) => fields.iter().any(|(_, value)| contains_await(value)),
        HirExpr::Block(stmts) => stmts.iter().any(stmt_contains_await),
        // A closure body runs only when the closure is invoked, not when the
        // function value is evaluated at this expression boundary.
        HirExpr::Lambda(..)
        | HirExpr::FunctionRef(..)
        | HirExpr::Lit(_)
        | HirExpr::Var(_)
        | HirExpr::EnvVar(_) => false,
    }
}

fn stmt_contains_await(stmt: &HirStmt) -> bool {
    match stmt {
        HirStmt::Expr(value)
        | HirStmt::Return(Some(value))
        | HirStmt::Let(_, _, value)
        | HirStmt::Throw(value) => contains_await(value),
        HirStmt::If(condition, then_body, else_body) => {
            contains_await(condition)
                || then_body.iter().any(stmt_contains_await)
                || else_body.iter().any(stmt_contains_await)
        }
        HirStmt::While(condition, body) => {
            contains_await(condition) || body.iter().any(stmt_contains_await)
        }
        HirStmt::Try(body, _, catch) => {
            body.iter().any(stmt_contains_await) || catch.iter().any(stmt_contains_await)
        }
        HirStmt::Return(None)
        | HirStmt::Break
        | HirStmt::Continue
        | HirStmt::BreakDepth(_)
        | HirStmt::ContinueDepth(_) => false,
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
            HirStmt::Return(None)
            | HirStmt::Break
            | HirStmt::Continue
            | HirStmt::BreakDepth(_)
            | HirStmt::ContinueDepth(_) => {}
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
            HirStmt::Break
            | HirStmt::Continue
            | HirStmt::BreakDepth(_)
            | HirStmt::ContinueDepth(_) => out.push(stmt),
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
    inject_before_target_continue(stmts, 0, &HirStmt::Expr(update.clone()))
}

fn inject_before_target_continue(
    stmts: Vec<HirStmt>,
    nested_depth: usize,
    injected: &HirStmt,
) -> Vec<HirStmt> {
    let mut out = Vec::new();
    for stmt in stmts {
        match stmt {
            HirStmt::Continue => {
                if nested_depth == 0 {
                    out.push(injected.clone());
                }
                out.push(HirStmt::Continue);
            }
            HirStmt::ContinueDepth(depth) => {
                if depth == nested_depth {
                    out.push(injected.clone());
                }
                out.push(HirStmt::ContinueDepth(depth));
            }
            HirStmt::If(cond, then_body, else_body) => out.push(HirStmt::If(
                cond,
                inject_before_target_continue(then_body, nested_depth, injected),
                inject_before_target_continue(else_body, nested_depth, injected),
            )),
            HirStmt::Try(body, catch_name, catch_body) => out.push(HirStmt::Try(
                inject_before_target_continue(body, nested_depth, injected),
                catch_name,
                inject_before_target_continue(catch_body, nested_depth, injected),
            )),
            HirStmt::While(cond, body) => out.push(HirStmt::While(
                cond,
                inject_before_target_continue(body, nested_depth + 1, injected),
            )),
            other => out.push(other),
        }
    }
    out
}

/// A `do { body } while (condition)` is represented as an unconditional HIR
/// loop with a condition guard at the tail. Source-level `continue` also has
/// to execute that guard before starting the next iteration.
fn inject_do_while_guard_before_continue(stmts: Vec<HirStmt>, guard: &HirStmt) -> Vec<HirStmt> {
    inject_before_target_continue(stmts, 0, guard)
}

/// Rewrites breaks that target a source switch into an assignment selecting
/// the synthetic exit state. Breaks inside nested loops retain their loop
/// target; nested switches have already consumed their own breaks while
/// lowering.
fn rewrite_switch_case_stmts(
    mut stmts: Vec<HirStmt>,
    selected: &str,
    case_index: usize,
    exit: &HirStmt,
) -> Vec<HirStmt> {
    if stmts.is_empty() {
        return Vec::new();
    }
    let first = stmts.remove(0);
    let rewritten = match first {
        HirStmt::Break => exit.clone(),
        HirStmt::If(cond, then_body, else_body) => HirStmt::If(
            cond,
            rewrite_switch_case_stmts(then_body, selected, case_index, exit),
            rewrite_switch_case_stmts(else_body, selected, case_index, exit),
        ),
        HirStmt::Try(body, catch_name, catch_body) => HirStmt::Try(
            rewrite_switch_case_stmts(body, selected, case_index, exit),
            catch_name,
            rewrite_switch_case_stmts(catch_body, selected, case_index, exit),
        ),
        other => other,
    };
    let mut out = vec![rewritten];
    let rest = rewrite_switch_case_stmts(stmts, selected, case_index, exit);
    if !rest.is_empty() {
        out.push(HirStmt::If(
            HirExpr::BinOp(
                BinOp::EqEqEq,
                Box::new(HirExpr::Var(selected.to_string())),
                Box::new(HirExpr::Lit(HirLit::F64(case_index as f64))),
            ),
            rest,
            Vec::new(),
        ));
    }
    out
}

fn compound_op(op: AssignOp) -> Option<BinOp> {
    match op {
        AssignOp::AddAssign => Some(BinOp::Add),
        AssignOp::SubAssign => Some(BinOp::Sub),
        AssignOp::MulAssign => Some(BinOp::Mul),
        AssignOp::DivAssign => Some(BinOp::Div),
        AssignOp::ModAssign => Some(BinOp::Mod),
        AssignOp::ExpAssign => Some(BinOp::Exp),
        AssignOp::BitOrAssign => Some(BinOp::BitOr),
        AssignOp::BitXorAssign => Some(BinOp::BitXor),
        AssignOp::BitAndAssign => Some(BinOp::BitAnd),
        AssignOp::LShiftAssign => Some(BinOp::LShift),
        AssignOp::RShiftAssign => Some(BinOp::RShift),
        AssignOp::ZeroFillRShiftAssign => Some(BinOp::ZeroFillRShift),
        _ => None,
    }
}

fn lower_bin_op(op: BinaryOp) -> Result<BinOp, String> {
    match op {
        BinaryOp::Add => Ok(BinOp::Add),
        BinaryOp::Sub => Ok(BinOp::Sub),
        BinaryOp::Mul => Ok(BinOp::Mul),
        BinaryOp::Div => Ok(BinOp::Div),
        BinaryOp::Mod => Ok(BinOp::Mod),
        BinaryOp::Exp => Ok(BinOp::Exp),
        BinaryOp::BitOr => Ok(BinOp::BitOr),
        BinaryOp::BitXor => Ok(BinOp::BitXor),
        BinaryOp::BitAnd => Ok(BinOp::BitAnd),
        BinaryOp::LShift => Ok(BinOp::LShift),
        BinaryOp::RShift => Ok(BinOp::RShift),
        BinaryOp::ZeroFillRShift => Ok(BinOp::ZeroFillRShift),
        BinaryOp::Lt => Ok(BinOp::Lt),
        BinaryOp::Gt => Ok(BinOp::Gt),
        BinaryOp::EqEqEq => Ok(BinOp::EqEqEq),
        other => Err(format!("unsupported binary operator {other:?}")),
    }
}

fn native_typeof_name(ty: &HirType) -> Option<&'static str> {
    match ty {
        HirType::F64 | HirType::I64 => Some("number"),
        HirType::Str => Some("string"),
        HirType::Bool => Some("boolean"),
        HirType::Function(_, _) => Some("function"),
        HirType::Array(_)
        | HirType::Tuple(_)
        | HirType::Object(_)
        | HirType::Json
        | HirType::Promise(_) => Some("object"),
        HirType::Union(elements) => {
            let first = elements.first().and_then(native_typeof_name)?;
            elements
                .iter()
                .all(|element| native_typeof_name(element) == Some(first))
                .then_some(first)
        }
        HirType::Void | HirType::Dynamic | HirType::JsValue => None,
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
    loop_depth: usize,
    labels: Vec<(Symbol, usize, bool)>,
}

impl<'a> FnLowerer<'a> {
    fn stmt_is_iteration(stmt: &Stmt) -> bool {
        match stmt {
            Stmt::While(_) | Stmt::DoWhile(_) | Stmt::For(_) | Stmt::ForIn(_) | Stmt::ForOf(_) => {
                true
            }
            Stmt::Labeled(labeled) => Self::stmt_is_iteration(&labeled.body),
            _ => false,
        }
    }

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
            loop_depth: 0,
            labels: Vec::new(),
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

    fn lower_loop_body(&mut self, stmt: &Stmt) -> Result<Vec<HirStmt>, String> {
        self.loop_depth += 1;
        let result = self.lower_body(stmt);
        self.loop_depth -= 1;
        result
    }

    fn lower_stmt_seq(&mut self, stmt: &Stmt) -> Result<Vec<HirStmt>, String> {
        match stmt {
            // Empty statements have no runtime effect. `debugger` only has an
            // observable effect when a JavaScript debugger is attached; a
            // native Thaw executable therefore treats it as a no-op.
            Stmt::Empty(_) | Stmt::Debugger(_) => Ok(Vec::new()),
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
                let body = self.lower_loop_body(&while_stmt.body)?;
                Ok(vec![HirStmt::While(cond, body)])
            }

            Stmt::DoWhile(do_while) => {
                let cond = self.lower_expr(&do_while.test)?;
                self.expect_type(&HirType::Bool, &cond, "do/while condition")?;
                let guard = HirStmt::If(cond, Vec::new(), vec![HirStmt::Break]);
                let mut body = self.lower_loop_body(&do_while.body)?;
                body = inject_do_while_guard_before_continue(body, &guard);
                body.push(guard);
                Ok(vec![HirStmt::While(
                    HirExpr::Lit(HirLit::Bool(true)),
                    body,
                )])
            }

            Stmt::Break(break_stmt) => {
                if let Some(label) = &break_stmt.label {
                    let name = label.sym.as_ref();
                    let (_, target_depth, _) = self
                        .labels
                        .iter()
                        .rev()
                        .find(|(candidate, _, _)| candidate == name)
                        .ok_or_else(|| format!("unknown break label `{name}`"))?;
                    return Ok(vec![HirStmt::BreakDepth(self.loop_depth - target_depth)]);
                }
                Ok(vec![HirStmt::Break])
            }

            Stmt::Continue(continue_stmt) => {
                if let Some(label) = &continue_stmt.label {
                    let name = label.sym.as_ref();
                    let (_, target_depth, continuable) = self
                        .labels
                        .iter()
                        .rev()
                        .find(|(candidate, _, _)| candidate == name)
                        .ok_or_else(|| format!("unknown continue label `{name}`"))?;
                    if !continuable {
                        return Err(format!("continue label `{name}` does not name a loop"));
                    }
                    return Ok(vec![HirStmt::ContinueDepth(
                        self.loop_depth - target_depth,
                    )]);
                }
                Ok(vec![HirStmt::Continue])
            }

            Stmt::Labeled(labeled) => {
                let name = labeled.label.sym.to_string();
                if self.labels.iter().any(|(candidate, _, _)| candidate == &name) {
                    return Err(format!("duplicate active label `{name}`"));
                }
                let is_loop = Self::stmt_is_iteration(&labeled.body);
                let target_depth = self.loop_depth + 1;
                self.labels.push((name, target_depth, is_loop));
                let lowered = if is_loop {
                    self.lower_stmt_seq(&labeled.body)
                } else {
                    self.loop_depth += 1;
                    let body_result = self.lower_body(&labeled.body);
                    self.loop_depth -= 1;
                    body_result.map(|mut body| {
                        body.push(HirStmt::Break);
                        vec![HirStmt::While(HirExpr::Lit(HirLit::Bool(true)), body)]
                    })
                };
                self.labels.pop();
                lowered
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

                    let mut body = self.lower_loop_body(&for_stmt.body)?;
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

            Stmt::ForOf(for_of) => {
                let saved = self.bindings.clone();
                let saved_scope = self.scope.clone();
                let lowered = (|| -> Result<Vec<HirStmt>, String> {
                    let values = self.lower_expr(&for_of.right)?;
                    let HirType::Array(element) = self.infer_expr_type(&values)? else {
                        return Err("`for...of` currently requires a typed array".into());
                    };
                    let (item_type, await_item) = if for_of.is_await {
                        match element.as_ref() {
                            HirType::Promise(resolved) => (resolved.as_ref().clone(), true),
                            synchronous => (synchronous.clone(), false),
                        }
                    } else {
                        (element.as_ref().clone(), false)
                    };
                    let values_name = format!("__thaw_for_of_values_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(
                        values_name.clone(),
                        HirType::Array(element.clone()),
                    );
                    let index_name = format!("__thaw_for_of_index_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(index_name.clone(), HirType::F64);
                    let item_value = || {
                        let indexed = HirExpr::TypedIndex(
                            Box::new(HirExpr::Var(values_name.clone())),
                            Box::new(HirExpr::Var(index_name.clone())),
                            element.as_ref().clone(),
                        );
                        if await_item {
                            HirExpr::AwaitPromise(Box::new(indexed), item_type.clone())
                        } else {
                            indexed
                        }
                    };
                    let item_stmts = match &for_of.left {
                        ForHead::VarDecl(decl) => {
                            let [declarator] = decl.decls.as_slice() else {
                                return Err("`for...of` requires exactly one loop binding".into());
                            };
                            if declarator.init.is_some() {
                                return Err(
                                    "`for...of` loop bindings cannot have an initializer".into(),
                                );
                            }
                            if let Pat::Ident(binding) = &declarator.name {
                                let item_ty = match &binding.type_ann {
                                    Some(annotation) => {
                                        let declared = lower_ts_type(
                                            &annotation.type_ann,
                                            self.interfaces,
                                            self.generic_interfaces,
                                        )?;
                                        if declared != item_type {
                                            return Err(format!(
                                                "`for...of` binding has type {declared:?}, expected {:?}",
                                                item_type
                                            ));
                                        }
                                        declared
                                    }
                                    None => item_type.clone(),
                                };
                                let source_name = binding.id.sym.to_string();
                                let item_name =
                                    format!("{source_name}__thaw_{}", self.next_binding);
                                self.next_binding += 1;
                                self.scope.insert(item_name.clone(), item_ty.clone());
                                self.bindings
                                    .entry(source_name)
                                    .or_default()
                                    .push(item_name.clone());
                                vec![HirStmt::Let(item_name, item_ty, item_value())]
                            } else if matches!(declarator.name, Pat::Object(_) | Pat::Array(_)) {
                                let temporary =
                                    format!("__thaw_for_of_item_{}", self.next_binding);
                                self.next_binding += 1;
                                self.scope.insert(temporary.clone(), item_type.clone());
                                let mut statements = vec![HirStmt::Let(
                                    temporary.clone(),
                                    item_type.clone(),
                                    item_value(),
                                )];
                                self.lower_binding_pattern(
                                    &declarator.name,
                                    HirExpr::Var(temporary),
                                    &item_type,
                                    &mut statements,
                                )?;
                                statements
                            } else {
                                return Err("unsupported `for...of` binding pattern".into());
                            }
                        }
                        ForHead::Pat(pattern) => {
                            if let Pat::Ident(binding) = pattern.as_ref() {
                                let item_name = self.resolve_binding(binding.id.sym.as_ref());
                                let item_ty = self.scope.get(&item_name).cloned().ok_or_else(|| {
                                    format!("unknown `for...of` assignment target `{item_name}`")
                                })?;
                                if item_ty != item_type {
                                    return Err(format!(
                                        "`for...of` assignment target has type {item_ty:?}, expected {:?}",
                                        item_type
                                    ));
                                }
                                vec![HirStmt::Expr(HirExpr::Assign(
                                    item_name,
                                    Box::new(item_value()),
                                ))]
                            } else if matches!(pattern.as_ref(), Pat::Object(_) | Pat::Array(_)) {
                                let temporary =
                                    format!("__thaw_for_of_item_{}", self.next_binding);
                                self.next_binding += 1;
                                self.scope.insert(temporary.clone(), item_type.clone());
                                let mut statements = vec![HirStmt::Let(
                                    temporary.clone(),
                                    item_type.clone(),
                                    item_value(),
                                )];
                                self.lower_assignment_pattern(
                                    pattern,
                                    HirExpr::Var(temporary),
                                    &item_type,
                                    &mut statements,
                                )?;
                                statements
                            } else {
                                return Err("unsupported `for...of` assignment pattern".into());
                            }
                        }
                        ForHead::UsingDecl(_) => {
                            return Err("`using` bindings in `for...of` are not supported".into())
                        }
                    };
                    let mut body = item_stmts;
                    body.extend(self.lower_loop_body(&for_of.body)?);
                    let update = HirExpr::Assign(
                        index_name.clone(),
                        Box::new(HirExpr::BinOp(
                            BinOp::Add,
                            Box::new(HirExpr::Var(index_name.clone())),
                            Box::new(HirExpr::Lit(HirLit::F64(1.0))),
                        )),
                    );
                    body = inject_for_update_before_continue(body, &update);
                    body.push(HirStmt::Expr(update));
                    Ok(vec![
                        HirStmt::Let(
                            values_name.clone(),
                            HirType::Array(element),
                            values,
                        ),
                        HirStmt::Let(
                            index_name.clone(),
                            HirType::F64,
                            HirExpr::Lit(HirLit::F64(0.0)),
                        ),
                        HirStmt::While(
                            HirExpr::BinOp(
                                BinOp::Lt,
                                Box::new(HirExpr::Var(index_name)),
                                Box::new(HirExpr::ArrayLen(Box::new(HirExpr::Var(values_name)))),
                            ),
                            body,
                        ),
                    ])
                })();
                self.bindings = saved;
                self.scope = saved_scope;
                lowered
            }

            Stmt::ForIn(for_in) => {
                let saved = self.bindings.clone();
                let saved_scope = self.scope.clone();
                let lowered = (|| -> Result<Vec<HirStmt>, String> {
                    let object = self.lower_expr(&for_in.right)?;
                    let object_type = self.infer_expr_type(&object)?;
                    let keys = match &object_type {
                        HirType::Object(fields) => fields
                            .iter()
                            .map(|(name, _)| HirExpr::Lit(HirLit::Str(name.clone())))
                            .collect::<Vec<_>>(),
                        _ => {
                            return Err(
                                "`for...in` currently requires a fixed-shape object".into(),
                            )
                        }
                    };
                    let object_name = format!("__thaw_for_in_object_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope
                        .insert(object_name.clone(), object_type.clone());
                    let keys_name = format!("__thaw_for_in_keys_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(
                        keys_name.clone(),
                        HirType::Array(Box::new(HirType::Str)),
                    );
                    let index_name = format!("__thaw_for_in_index_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(index_name.clone(), HirType::F64);
                    let key_value = || {
                        HirExpr::TypedIndex(
                            Box::new(HirExpr::Var(keys_name.clone())),
                            Box::new(HirExpr::Var(index_name.clone())),
                            HirType::Str,
                        )
                    };
                    let binding_stmt = match &for_in.left {
                        ForHead::VarDecl(decl) => {
                            let [declarator] = decl.decls.as_slice() else {
                                return Err("`for...in` requires exactly one loop binding".into());
                            };
                            if declarator.init.is_some() {
                                return Err(
                                    "`for...in` loop bindings cannot have an initializer".into(),
                                );
                            }
                            let Pat::Ident(binding) = &declarator.name else {
                                return Err(
                                    "`for...in` requires an identifier loop binding".into(),
                                );
                            };
                            if let Some(annotation) = &binding.type_ann {
                                let declared = lower_ts_type(
                                    &annotation.type_ann,
                                    self.interfaces,
                                    self.generic_interfaces,
                                )?;
                                if declared != HirType::Str {
                                    return Err(format!(
                                        "`for...in` binding must be Str, got {declared:?}"
                                    ));
                                }
                            }
                            let source_name = binding.id.sym.to_string();
                            let binding_name =
                                format!("{source_name}__thaw_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(binding_name.clone(), HirType::Str);
                            self.bindings
                                .entry(source_name)
                                .or_default()
                                .push(binding_name.clone());
                            HirStmt::Let(binding_name, HirType::Str, key_value())
                        }
                        ForHead::Pat(pattern) => {
                            let Pat::Ident(binding) = pattern.as_ref() else {
                                return Err(
                                    "`for...in` assignment requires an identifier target".into(),
                                );
                            };
                            let binding_name = self.resolve_binding(binding.id.sym.as_ref());
                            let binding_type = self.scope.get(&binding_name).ok_or_else(|| {
                                format!("unknown `for...in` assignment target `{binding_name}`")
                            })?;
                            if binding_type != &HirType::Str {
                                return Err(format!(
                                    "`for...in` assignment target must be Str, got {binding_type:?}"
                                ));
                            }
                            HirStmt::Expr(HirExpr::Assign(
                                binding_name,
                                Box::new(key_value()),
                            ))
                        }
                        ForHead::UsingDecl(_) => {
                            return Err("`using` bindings in `for...in` are not supported".into())
                        }
                    };
                    let update = HirExpr::Assign(
                        index_name.clone(),
                        Box::new(HirExpr::BinOp(
                            BinOp::Add,
                            Box::new(HirExpr::Var(index_name.clone())),
                            Box::new(HirExpr::Lit(HirLit::F64(1.0))),
                        )),
                    );
                    let mut body = vec![binding_stmt];
                    body.extend(self.lower_loop_body(&for_in.body)?);
                    body = inject_for_update_before_continue(body, &update);
                    body.push(HirStmt::Expr(update));
                    Ok(vec![
                        HirStmt::Let(object_name, object_type, object),
                        HirStmt::Let(
                            keys_name.clone(),
                            HirType::Array(Box::new(HirType::Str)),
                            HirExpr::ArrayLit(keys),
                        ),
                        HirStmt::Let(
                            index_name.clone(),
                            HirType::F64,
                            HirExpr::Lit(HirLit::F64(0.0)),
                        ),
                        HirStmt::While(
                            HirExpr::BinOp(
                                BinOp::Lt,
                                Box::new(HirExpr::Var(index_name)),
                                Box::new(HirExpr::ArrayLen(Box::new(HirExpr::Var(keys_name)))),
                            ),
                            body,
                        ),
                    ])
                })();
                self.bindings = saved;
                self.scope = saved_scope;
                lowered
            }

            Stmt::Switch(switch_stmt) => {
                let saved = self.bindings.clone();
                let saved_scope = self.scope.clone();
                let lowered = (|| -> Result<Vec<HirStmt>, String> {
                    let discriminant = self.lower_expr(&switch_stmt.discriminant)?;
                    let discriminant_type = self.infer_expr_type(&discriminant)?;
                    let value_name = format!("__thaw_switch_value_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope
                        .insert(value_name.clone(), discriminant_type.clone());
                    let selected_name = format!("__thaw_switch_selected_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(selected_name.clone(), HirType::F64);
                    let none = HirExpr::Lit(HirLit::F64(-1.0));
                    let case_count = switch_stmt.cases.len();
                    let default_index = switch_stmt
                        .cases
                        .iter()
                        .position(|case| case.test.is_none())
                        .unwrap_or(case_count);
                    let mut out = vec![
                        HirStmt::Let(value_name.clone(), discriminant_type.clone(), discriminant),
                        HirStmt::Let(selected_name.clone(), HirType::F64, none.clone()),
                    ];

                    for (index, case) in switch_stmt.cases.iter().enumerate() {
                        let Some(test) = &case.test else {
                            continue;
                        };
                        let test = self.lower_expr(test)?;
                        self.expect_type(&discriminant_type, &test, "switch case")?;
                        out.push(HirStmt::If(
                            HirExpr::BinOp(
                                BinOp::EqEqEq,
                                Box::new(HirExpr::Var(selected_name.clone())),
                                Box::new(none.clone()),
                            ),
                            vec![HirStmt::If(
                                HirExpr::BinOp(
                                    BinOp::EqEqEq,
                                    Box::new(HirExpr::Var(value_name.clone())),
                                    Box::new(test),
                                ),
                                vec![HirStmt::Expr(HirExpr::Assign(
                                    selected_name.clone(),
                                    Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                                ))],
                                Vec::new(),
                            )],
                            Vec::new(),
                        ));
                    }
                    out.push(HirStmt::If(
                        HirExpr::BinOp(
                            BinOp::EqEqEq,
                            Box::new(HirExpr::Var(selected_name.clone())),
                            Box::new(none),
                        ),
                        vec![HirStmt::Expr(HirExpr::Assign(
                            selected_name.clone(),
                            Box::new(HirExpr::Lit(HirLit::F64(default_index as f64))),
                        ))],
                        Vec::new(),
                    ));

                    let exit = HirStmt::Expr(HirExpr::Assign(
                        selected_name.clone(),
                        Box::new(HirExpr::Lit(HirLit::F64(case_count as f64))),
                    ));
                    for (index, case) in switch_stmt.cases.iter().enumerate() {
                        let mut body = self.lower_stmts(&case.cons)?;
                        body = rewrite_switch_case_stmts(
                            body,
                            &selected_name,
                            index,
                            &exit,
                        );
                        body.push(HirStmt::If(
                            HirExpr::BinOp(
                                BinOp::EqEqEq,
                                Box::new(HirExpr::Var(selected_name.clone())),
                                Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                            ),
                            vec![HirStmt::Expr(HirExpr::Assign(
                                selected_name.clone(),
                                Box::new(HirExpr::Lit(HirLit::F64((index + 1) as f64))),
                            ))],
                            Vec::new(),
                        ));
                        out.push(HirStmt::If(
                            HirExpr::BinOp(
                                BinOp::EqEqEq,
                                Box::new(HirExpr::Var(selected_name.clone())),
                                Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                            ),
                            body,
                            Vec::new(),
                        ));
                    }
                    Ok(out)
                })();
                self.bindings = saved;
                self.scope = saved_scope;
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
        let mut statements = Vec::new();
        for decl in &var_decl.decls {
            if let Pat::Ident(binding) = &decl.name {
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
                statements.push(HirStmt::Let(hir_name, ty, value));
                continue;
            }

            let init = decl
                .init
                .as_deref()
                .ok_or("destructuring declarations need an initializer")?;
            let mut value = self.lower_expr(init)?;
            let annotation = match &decl.name {
                Pat::Array(pattern) => pattern.type_ann.as_ref(),
                Pat::Object(pattern) => pattern.type_ann.as_ref(),
                _ => None,
            };
            let ty = if let Some(annotation) = annotation {
                let ty = lower_ts_type(
                    &annotation.type_ann,
                    self.interfaces,
                    self.generic_interfaces,
                )?;
                value = self.coerce_to_declared(&ty, value)?;
                ty
            } else if matches!(&decl.name, Pat::Array(_)) {
                if let HirExpr::ArrayLit(elements) = &value {
                    HirType::Tuple(
                        elements
                            .iter()
                            .map(|element| self.infer_expr_type(element))
                            .collect::<Result<Vec<_>, _>>()?,
                    )
                } else {
                    self.infer_expr_type(&value)?
                }
            } else {
                self.infer_expr_type(&value)?
            };
            if !matches!(ty, HirType::Object(_) | HirType::Tuple(_)) {
                return Err(format!(
                    "destructuring requires a fixed-shape object or tuple, got {ty:?}"
                ));
            }
            let temporary = format!("__thaw_destructure_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(temporary.clone(), ty.clone());
            statements.push(HirStmt::Let(temporary.clone(), ty.clone(), value));
            self.lower_binding_pattern(&decl.name, HirExpr::Var(temporary), &ty, &mut statements)?;
        }
        Ok(statements)
    }

    fn lower_binding_pattern(
        &mut self,
        pattern: &Pat,
        value: HirExpr,
        ty: &HirType,
        statements: &mut Vec<HirStmt>,
    ) -> Result<(), String> {
        match pattern {
            Pat::Ident(binding) => {
                let binding_type = if let Some(annotation) = &binding.type_ann {
                    let annotated = lower_ts_type(
                        &annotation.type_ann,
                        self.interfaces,
                        self.generic_interfaces,
                    )?;
                    self.expect_type(&annotated, &value, "destructured binding")?;
                    annotated
                } else {
                    ty.clone()
                };
                let name = self.bind_local(binding.id.sym.as_ref(), binding_type.clone());
                statements.push(HirStmt::Let(name, binding_type, value));
                Ok(())
            }
            Pat::Object(pattern) => {
                let HirType::Object(fields) = ty else {
                    return Err(format!("object pattern cannot destructure {ty:?}"));
                };
                let mut used = BTreeSet::new();
                for property in &pattern.props {
                    match property {
                        ObjectPatProp::Assign(property) => {
                            if property.value.is_some() {
                                return Err(
                                    "destructuring defaults require undefined support".into()
                                );
                            }
                            let key = property.key.id.sym.to_string();
                            let field_type = fields
                                .iter()
                                .find(|(name, _)| name == &key)
                                .map(|(_, ty)| ty.clone())
                                .ok_or_else(|| format!("object has no field `{key}`"))?;
                            used.insert(key.clone());
                            self.lower_binding_pattern(
                                &Pat::Ident(property.key.clone()),
                                HirExpr::PropAccess(Box::new(value.clone()), ty.clone(), key),
                                &field_type,
                                statements,
                            )?;
                        }
                        ObjectPatProp::KeyValue(property) => {
                            let key =
                                match &property.key {
                                    PropName::Ident(key) => key.sym.to_string(),
                                    PropName::Str(key) => key.value.to_string_lossy().into_owned(),
                                    PropName::Computed(computed) => match computed.expr.as_ref() {
                                        Expr::Lit(Lit::Str(key)) => {
                                            key.value.to_string_lossy().into_owned()
                                        }
                                        _ => return Err(
                                            "computed destructuring keys must be string literals"
                                                .into(),
                                        ),
                                    },
                                    _ => return Err("unsupported object destructuring key".into()),
                                };
                            let field_type = fields
                                .iter()
                                .find(|(name, _)| name == &key)
                                .map(|(_, ty)| ty.clone())
                                .ok_or_else(|| format!("object has no field `{key}`"))?;
                            used.insert(key.clone());
                            self.lower_binding_pattern(
                                &property.value,
                                HirExpr::PropAccess(Box::new(value.clone()), ty.clone(), key),
                                &field_type,
                                statements,
                            )?;
                        }
                        ObjectPatProp::Rest(rest) => {
                            let remaining = fields
                                .iter()
                                .filter(|(name, _)| !used.contains(name))
                                .cloned()
                                .collect::<Vec<_>>();
                            let rest_value = HirExpr::ObjectLit(
                                remaining
                                    .iter()
                                    .map(|(name, _)| {
                                        (
                                            name.clone(),
                                            HirExpr::PropAccess(
                                                Box::new(value.clone()),
                                                ty.clone(),
                                                name.clone(),
                                            ),
                                        )
                                    })
                                    .collect(),
                            );
                            self.lower_binding_pattern(
                                &rest.arg,
                                rest_value,
                                &HirType::Object(remaining),
                                statements,
                            )?;
                        }
                    }
                }
                Ok(())
            }
            Pat::Array(pattern) => {
                let HirType::Tuple(elements) = ty else {
                    return Err(format!(
                        "array pattern requires a fixed-length tuple, got {ty:?}"
                    ));
                };
                for (index, element_pattern) in pattern.elems.iter().enumerate() {
                    let Some(element_pattern) = element_pattern else {
                        continue;
                    };
                    if let Pat::Rest(rest) = element_pattern {
                        let remaining = elements[index..].to_vec();
                        let rest_value = HirExpr::ArrayLit(
                            remaining
                                .iter()
                                .enumerate()
                                .map(|(offset, element)| {
                                    HirExpr::TypedIndex(
                                        Box::new(value.clone()),
                                        Box::new(HirExpr::Lit(HirLit::F64(
                                            (index + offset) as f64,
                                        ))),
                                        element.clone(),
                                    )
                                })
                                .collect(),
                        );
                        let rest_type = if remaining
                            .first()
                            .is_some_and(|first| remaining.iter().all(|element| element == first))
                        {
                            HirType::Array(Box::new(
                                remaining.first().cloned().unwrap_or(HirType::F64),
                            ))
                        } else {
                            HirType::Tuple(remaining)
                        };
                        self.lower_binding_pattern(&rest.arg, rest_value, &rest_type, statements)?;
                        break;
                    }
                    let element_type = elements
                        .get(index)
                        .cloned()
                        .ok_or_else(|| format!("tuple pattern index {index} is out of bounds"))?;
                    self.lower_binding_pattern(
                        element_pattern,
                        HirExpr::TypedIndex(
                            Box::new(value.clone()),
                            Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                            element_type.clone(),
                        ),
                        &element_type,
                        statements,
                    )?;
                }
                Ok(())
            }
            Pat::Assign(_) => Err("destructuring defaults require undefined support".into()),
            Pat::Rest(_) => Err("rest patterns are only valid inside object/array patterns".into()),
            _ => Err("unsupported destructuring binding pattern".into()),
        }
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
        if let (HirType::Tuple(expected), HirExpr::ArrayLit(values)) = (declared, &value) {
            if expected.len() != values.len() {
                return Err(format!(
                    "tuple literal has {} element(s), expected {}",
                    values.len(),
                    expected.len()
                ));
            }
            for (index, (expected, value)) in expected.iter().zip(values).enumerate() {
                self.expect_type(expected, value, &format!("tuple element {index}"))?;
            }
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
            HirExpr::FunctionRef(_, params, ret) => {
                Ok(HirType::Function(params.clone(), Box::new(ret.clone())))
            }
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
                    BinOp::Lt | BinOp::Gt | BinOp::LtEq | BinOp::GtEq => {
                        if !matches!(left_ty, HirType::F64 | HirType::Dynamic)
                            || !matches!(right_ty, HirType::F64 | HirType::Dynamic)
                        {
                            return Err(format!(
                                "numeric comparison requires F64 operands, got {left_ty:?} and {right_ty:?}"
                            ));
                        }
                        Ok(HirType::Bool)
                    }
                    BinOp::Add
                    | BinOp::Sub
                    | BinOp::Mul
                    | BinOp::Div
                    | BinOp::Mod
                    | BinOp::Exp
                    | BinOp::BitOr
                    | BinOp::BitXor
                    | BinOp::BitAnd
                    | BinOp::LShift
                    | BinOp::RShift
                    | BinOp::ZeroFillRShift => {
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
                    let HirType::Function(params, ret) = self.infer_expr_type(callee)? else {
                        return Err("call target is not a function value".into());
                    };
                    if params.len() != args.len() {
                        return Err(format!(
                            "function value expects {} argument(s), got {}",
                            params.len(),
                            args.len()
                        ));
                    }
                    return Ok(*ret);
                };
                match name.as_str() {
                    "console.log" => return Ok(HirType::Void),
                    "__thaw_string_concat" => {
                        if args.len() != 2 {
                            return Err("string concatenation expects two operands".into());
                        }
                        for argument in args {
                            self.expect_type(&HirType::Str, argument, "string concatenation")?;
                        }
                        return Ok(HirType::Str);
                    }
                    "__thaw_bool_to_string" => {
                        let [argument] = args.as_slice() else {
                            return Err("boolean string conversion expects one operand".into());
                        };
                        self.expect_type(&HirType::Bool, argument, "boolean string conversion")?;
                        return Ok(HirType::Str);
                    }
                    "__thaw_number_to_string" => {
                        let [argument] = args.as_slice() else {
                            return Err("number string conversion expects one operand".into());
                        };
                        self.expect_type(&HirType::F64, argument, "number string conversion")?;
                        return Ok(HirType::Str);
                    }
                    "__thaw_bool_to_number" => {
                        let [argument] = args.as_slice() else {
                            return Err("boolean number conversion expects one operand".into());
                        };
                        self.expect_type(&HirType::Bool, argument, "boolean number conversion")?;
                        return Ok(HirType::F64);
                    }
                    "__thaw_string_to_number" => {
                        let [argument] = args.as_slice() else {
                            return Err("string number conversion expects one operand".into());
                        };
                        self.expect_type(&HirType::Str, argument, "string number conversion")?;
                        return Ok(HirType::F64);
                    }
                    "__thaw_parse_float" => {
                        let [argument] = args.as_slice() else {
                            return Err("parseFloat expects one operand".into());
                        };
                        self.expect_type(&HirType::Str, argument, "parseFloat operand")?;
                        return Ok(HirType::F64);
                    }
                    "__thaw_parse_int" => {
                        let [text, radix] = args.as_slice() else {
                            return Err("parseInt expects text and radix operands".into());
                        };
                        self.expect_type(&HirType::Str, text, "parseInt text")?;
                        self.expect_type(&HirType::F64, radix, "parseInt radix")?;
                        return Ok(HirType::F64);
                    }
                    "__thaw_string_lt" | "__thaw_string_gt" | "__thaw_string_lte"
                    | "__thaw_string_gte" => {
                        if args.len() != 2 {
                            return Err("string comparison expects two operands".into());
                        }
                        for argument in args {
                            self.expect_type(&HirType::Str, argument, "string comparison")?;
                        }
                        return Ok(HirType::Bool);
                    }
                    "__thaw_number_array_to_string"
                    | "__thaw_string_array_to_string"
                    | "__thaw_bool_array_to_string"
                    | "__thaw_object_array_to_string"
                    | "__thaw_object_to_string" => return Ok(HirType::Str),
                    "__thaw_number_array_join"
                    | "__thaw_string_array_join"
                    | "__thaw_bool_array_join"
                    | "__thaw_object_array_join" => {
                        if args.len() != 2 {
                            return Err("array join expects two operands".into());
                        }
                        self.expect_type(&HirType::Str, &args[1], "array join separator")?;
                        return Ok(HirType::Str);
                    }
                    "__thaw_array_reverse" => {
                        let [array] = args.as_slice() else {
                            return Err("array reverse expects one operand".into());
                        };
                        let ty = self.infer_expr_type(array)?;
                        if !matches!(ty, HirType::Array(_)) {
                            return Err("array reverse requires a homogeneous array".into());
                        }
                        return Ok(ty);
                    }
                    "__thaw_array_copy_within" => {
                        if args.len() != 4 {
                            return Err("array copyWithin expects four operands".into());
                        }
                        let ty = self.infer_expr_type(&args[0])?;
                        if !matches!(ty, HirType::Array(_)) {
                            return Err("array copyWithin requires a homogeneous array".into());
                        }
                        for argument in &args[1..] {
                            self.expect_type(&HirType::F64, argument, "copyWithin index")?;
                        }
                        return Ok(ty);
                    }
                    "__thaw_number_array_fill"
                    | "__thaw_pointer_array_fill"
                    | "__thaw_bool_array_fill" => {
                        if args.len() != 4 {
                            return Err("array fill expects four operands".into());
                        }
                        let ty = self.infer_expr_type(&args[0])?;
                        let HirType::Array(element) = &ty else {
                            return Err("array fill requires a homogeneous array".into());
                        };
                        self.expect_type(element, &args[1], "fill value")?;
                        for argument in &args[2..] {
                            self.expect_type(&HirType::F64, argument, "fill index")?;
                        }
                        return Ok(ty);
                    }
                    "__thaw_array_slice" => {
                        if args.len() != 3 {
                            return Err("array slice expects three operands".into());
                        }
                        let ty = self.infer_expr_type(&args[0])?;
                        if !matches!(ty, HirType::Array(_)) {
                            return Err("array slice requires a homogeneous array".into());
                        }
                        for argument in &args[1..] {
                            self.expect_type(&HirType::F64, argument, "slice index")?;
                        }
                        return Ok(ty);
                    }
                    "__thaw_array_to_reversed" => {
                        let [array] = args.as_slice() else {
                            return Err("array toReversed expects one operand".into());
                        };
                        let ty = self.infer_expr_type(array)?;
                        if !matches!(ty, HirType::Array(_)) {
                            return Err("array toReversed requires a homogeneous array".into());
                        }
                        return Ok(ty);
                    }
                    "__thaw_number_array_index_of"
                    | "__thaw_string_array_index_of"
                    | "__thaw_bool_array_index_of"
                    | "__thaw_object_array_index_of"
                    | "__thaw_number_array_last_index_of"
                    | "__thaw_string_array_last_index_of"
                    | "__thaw_bool_array_last_index_of"
                    | "__thaw_object_array_last_index_of"
                    | "__thaw_number_array_includes"
                    | "__thaw_string_array_includes"
                    | "__thaw_bool_array_includes"
                    | "__thaw_object_array_includes" => {
                        if args.len() != 3 {
                            return Err("array search expects three operands".into());
                        }
                        self.expect_type(&HirType::F64, &args[2], "array search start")?;
                        return Ok(if name.ends_with("_includes") {
                            HirType::Bool
                        } else {
                            HirType::F64
                        });
                    }
                    "__thaw_string_index_of"
                    | "__thaw_string_last_index_of"
                    | "__thaw_string_includes"
                    | "__thaw_string_starts_with"
                    | "__thaw_string_ends_with" => {
                        if args.len() != 3 {
                            return Err("string search expects three operands".into());
                        }
                        self.expect_type(&HirType::Str, &args[0], "string search receiver")?;
                        self.expect_type(&HirType::Str, &args[1], "string search needle")?;
                        self.expect_type(&HirType::F64, &args[2], "string search position")?;
                        return Ok(
                            if matches!(
                                name.as_str(),
                                "__thaw_string_index_of" | "__thaw_string_last_index_of"
                            ) {
                                HirType::F64
                            } else {
                                HirType::Bool
                            },
                        );
                    }
                    "__thaw_string_trim"
                    | "__thaw_string_trim_start"
                    | "__thaw_string_trim_end" => {
                        let [argument] = args.as_slice() else {
                            return Err("string trim expects one operand".into());
                        };
                        self.expect_type(&HirType::Str, argument, "string trim receiver")?;
                        return Ok(HirType::Str);
                    }
                    "__thaw_string_length" => {
                        let [argument] = args.as_slice() else {
                            return Err("string length expects one operand".into());
                        };
                        self.expect_type(&HirType::Str, argument, "string length receiver")?;
                        return Ok(HirType::F64);
                    }
                    "__thaw_string_char_code_at" => {
                        let [value, index] = args.as_slice() else {
                            return Err("string charCodeAt expects two operands".into());
                        };
                        self.expect_type(&HirType::Str, value, "charCodeAt receiver")?;
                        self.expect_type(&HirType::F64, index, "charCodeAt index")?;
                        return Ok(HirType::F64);
                    }
                    "__thaw_json_is_array" => {
                        let [value] = args.as_slice() else {
                            return Err("Array.isArray expects one operand".into());
                        };
                        self.expect_type(&HirType::Json, value, "Array.isArray JSON operand")?;
                        return Ok(HirType::Bool);
                    }
                    "__thaw_number_is_nan"
                    | "__thaw_number_is_finite"
                    | "__thaw_number_is_integer"
                    | "__thaw_number_is_safe_integer" => {
                        let [argument] = args.as_slice() else {
                            return Err("number predicate expects one operand".into());
                        };
                        self.expect_type(&HirType::F64, argument, "number predicate")?;
                        return Ok(HirType::Bool);
                    }
                    "__thaw_number_neg" | "__thaw_math_abs" | "__thaw_math_floor"
                    | "__thaw_math_ceil" | "__thaw_math_trunc" | "__thaw_math_sqrt"
                    | "__thaw_math_sign" | "__thaw_math_round" | "__thaw_math_exp"
                    | "__thaw_math_log" | "__thaw_math_log2" | "__thaw_math_log10"
                    | "__thaw_math_sin" | "__thaw_math_cos" | "__thaw_math_tan"
                    | "__thaw_math_asin" | "__thaw_math_acos" | "__thaw_math_atan"
                    | "__thaw_math_sinh" | "__thaw_math_cosh" | "__thaw_math_tanh"
                    | "__thaw_math_cbrt" | "__thaw_math_acosh" | "__thaw_math_asinh"
                    | "__thaw_math_atanh" | "__thaw_math_expm1" | "__thaw_math_log1p" => {
                        let [argument] = args.as_slice() else {
                            return Err("unary Math function expects one operand".into());
                        };
                        self.expect_type(&HirType::F64, argument, "Math operand")?;
                        return Ok(HirType::F64);
                    }
                    "__thaw_math_fround" | "__thaw_math_clz32" => {
                        let [argument] = args.as_slice() else {
                            return Err("unary Math function expects one operand".into());
                        };
                        self.expect_type(&HirType::F64, argument, "Math operand")?;
                        return Ok(HirType::F64);
                    }
                    "__thaw_math_random" => {
                        if !args.is_empty() {
                            return Err("Math.random expects no operands".into());
                        }
                        return Ok(HirType::F64);
                    }
                    "__thaw_math_pow" => {
                        if args.len() != 2 {
                            return Err("Math.pow expects two operands".into());
                        }
                        for argument in args {
                            self.expect_type(&HirType::F64, argument, "Math.pow operand")?;
                        }
                        return Ok(HirType::F64);
                    }
                    "__thaw_math_atan2" => {
                        if args.len() != 2 {
                            return Err("Math.atan2 expects two operands".into());
                        }
                        for argument in args {
                            self.expect_type(&HirType::F64, argument, "Math.atan2 operand")?;
                        }
                        return Ok(HirType::F64);
                    }
                    "__thaw_math_imul" => {
                        if args.len() != 2 {
                            return Err("Math.imul expects two operands".into());
                        }
                        for argument in args {
                            self.expect_type(&HirType::F64, argument, "Math.imul operand")?;
                        }
                        return Ok(HirType::F64);
                    }
                    "__thaw_math_min" | "__thaw_math_max" | "__thaw_math_hypot" => {
                        for argument in args {
                            self.expect_type(&HirType::F64, argument, "Math extrema operand")?;
                        }
                        return Ok(HirType::F64);
                    }
                    "fetch" => return Ok(HirType::Str),
                    "sleep" => return Ok(HirType::Promise(Box::new(HirType::Void))),
                    "Promise.all" => {
                        for (index, arg) in args.iter().enumerate() {
                            match self.infer_expr_type(arg)? {
                                HirType::Promise(value) if *value == HirType::F64 => {}
                                HirType::F64
                                    if matches!(arg, HirExpr::Call(callee, _)
                                        if matches!(callee.as_ref(), HirExpr::Var(name)
                                            if self.signatures.get(name).is_some_and(|signature| signature.is_async && signature.ret == HirType::F64))) => {}
                                other => {
                                    return Err(format!(
                                        "Promise.all element {index} must be Promise<number>, got {other:?}"
                                    ))
                                }
                            }
                        }
                        return Ok(HirType::Promise(Box::new(HirType::Array(Box::new(
                            HirType::F64,
                        )))));
                    }
                    "JSON.parse" => return Ok(HirType::Json),
                    "JSON.stringify" => return Ok(HirType::Str),
                    // QuickJS-NG fallback path (docs/design/bridge.md
                    // section 7): `loadScript` evaluates JS source into
                    // the global engine context; `callDynamic` calls a
                    // top-level function it defined, by name, with `Json`
                    // args in and a `Json` result out.
                    "loadScript" => return Ok(HirType::Bool),
                    "callDynamic" => return Ok(HirType::Json),
                    "getDynamicValue" => return Ok(HirType::JsValue),
                    "callDynamicValue" => return Ok(HirType::Json),
                    "callDynamicValueHandle" => return Ok(HirType::JsValue),
                    "callDynamicValueWithValue" => return Ok(HirType::Json),
                    "releaseDynamicValue" => return Ok(HirType::Bool),
                    "getDynamicProperty" => return Ok(HirType::JsValue),
                    "setDynamicProperty" => return Ok(HirType::Bool),
                    "callDynamicMethod" => return Ok(HirType::Json),
                    "readDynamicValue" => return Ok(HirType::Json),
                    "callDynamicValueMixed" => return Ok(HirType::Json),
                    "constructDynamicValue" => return Ok(HirType::JsValue),
                    "loadNativeAddon" => return Ok(HirType::Bool),
                    "loadNativeAddonEmbedded" => return Ok(HirType::Bool),
                    "callNativeAddon" => return Ok(HirType::Json),
                    "callNativeAddonWithCallback" => return Ok(HirType::Json),
                    "pollNativeAddonEvents" => return Ok(HirType::F64),
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
                        } else if sig.is_async {
                            Ok(HirType::Promise(Box::new(sig.ret.clone())))
                        } else {
                            Ok(sig.ret.clone())
                        }
                    }
                    None => Err(format!("call to unknown function `{name}`")),
                }
            }
            HirExpr::PromiseAll(_, element) => Ok(HirType::Promise(Box::new(HirType::Array(
                Box::new(element.clone()),
            )))),
            HirExpr::PromiseAllArray(_, element) => Ok(HirType::Promise(Box::new(HirType::Array(
                Box::new(element.clone()),
            )))),
            HirExpr::PromiseAllTuple(_, elements) => {
                Ok(HirType::Promise(Box::new(HirType::Tuple(elements.clone()))))
            }
            HirExpr::PromiseRace(_, element)
            | HirExpr::PromiseRaceArray(_, element)
            | HirExpr::PromiseAny(_, element)
            | HirExpr::PromiseAnyArray(_, element) => {
                Ok(HirType::Promise(Box::new(element.clone())))
            }
            HirExpr::PromiseAllSettled(_, element)
            | HirExpr::PromiseAllSettledArray(_, element) => Ok(HirType::Promise(Box::new(
                HirType::Array(Box::new(promise_settled_result_type(element.clone()))),
            ))),
            HirExpr::PromiseNew(_, resolved, _) => Ok(HirType::Promise(Box::new(resolved.clone()))),
            HirExpr::PromiseThen(_, _, _, output, _, _) => {
                Ok(HirType::Promise(Box::new(output.clone())))
            }
            HirExpr::PromiseFinally(_, _, input, _) => {
                Ok(HirType::Promise(Box::new(input.clone())))
            }
            HirExpr::DynamicCall(signature, _) => Ok(signature.ret.clone()),
            HirExpr::ArrayLit(values) => {
                if values.is_empty() {
                    return Ok(HirType::Array(Box::new(HirType::F64)));
                }
                let array_element_type =
                    |value: &HirExpr| -> Result<HirType, String> { self.infer_expr_type(value) };
                let elements = values
                    .iter()
                    .map(array_element_type)
                    .collect::<Result<Vec<_>, _>>()?;
                if elements.iter().all(|element| element == &elements[0]) {
                    Ok(HirType::Array(Box::new(elements[0].clone())))
                } else {
                    Ok(HirType::Tuple(elements))
                }
            }
            HirExpr::ArrayConcat(_, element) => Ok(HirType::Array(Box::new(element.clone()))),
            HirExpr::Index(arr, index) => {
                self.expect_type(&HirType::F64, index, "array index")?;
                match self.infer_expr_type(arr)? {
                    HirType::Array(elem) => Ok(*elem),
                    other => Err(format!("cannot index into a value of type {other:?}")),
                }
            }
            HirExpr::TypedIndex(_, _, element) => Ok(element.clone()),
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
            HirExpr::Await(inner) | HirExpr::AwaitPromise(inner, _) => {
                match self.infer_expr_type(inner)? {
                    HirType::Promise(value) => Ok(*value),
                    // Legacy/direct await sources can already expose their
                    // resolved type to the surrounding expression.
                    other => Ok(other),
                }
            }
            // The Lambda node now preserves typed parameters and its body,
            // but function values do not have a native ABI until the next
            // callback-lowering phase. Keep the enclosing local dynamic
            // instead of discarding or pretending to know that ABI.
            HirExpr::Lambda(_, params, ret, _) => Ok(HirType::Function(
                params.iter().map(|param| param.ty.clone()).collect(),
                Box::new(ret.clone()),
            )),
            HirExpr::Block(stmts) => self.infer_return_type(stmts),
        }
    }

    fn truthiness_expr(&self, value: HirExpr, ty: &HirType) -> Result<HirExpr, String> {
        let false_lit = || HirExpr::Lit(HirLit::Bool(false));
        match ty {
            HirType::Bool => Ok(value),
            HirType::F64 => {
                let is_zero = HirExpr::BinOp(
                    BinOp::EqEqEq,
                    Box::new(value.clone()),
                    Box::new(HirExpr::Lit(HirLit::F64(0.0))),
                );
                let not_nan =
                    HirExpr::BinOp(BinOp::EqEqEq, Box::new(value.clone()), Box::new(value));
                Ok(HirExpr::BinOp(
                    BinOp::EqEqEq,
                    Box::new(HirExpr::BinOp(
                        BinOp::EqEqEq,
                        Box::new(is_zero),
                        Box::new(not_nan),
                    )),
                    Box::new(false_lit()),
                ))
            }
            HirType::Str => Ok(HirExpr::BinOp(
                BinOp::EqEqEq,
                Box::new(HirExpr::BinOp(
                    BinOp::EqEqEq,
                    Box::new(value),
                    Box::new(HirExpr::Lit(HirLit::Str(String::new()))),
                )),
                Box::new(false_lit()),
            )),
            HirType::Json => Ok(HirExpr::JsonAsBool(Box::new(value))),
            HirType::Array(_)
            | HirType::Tuple(_)
            | HirType::Object(_)
            | HirType::Promise(_)
            | HirType::Function(_, _) => Ok(HirExpr::Lit(HirLit::Bool(true))),
            other => Err(format!(
                "logical truthiness is not defined for native type {other:?}"
            )),
        }
    }

    fn lower_logical_expr(
        &mut self,
        lhs: HirExpr,
        rhs: HirExpr,
        is_and: bool,
    ) -> Result<HirExpr, String> {
        let lhs_type = self.infer_expr_type(&lhs)?;
        let rhs_type = self.infer_expr_type(&rhs)?;
        if lhs_type != rhs_type {
            return Err(format!(
                "logical operands have incompatible types {lhs_type:?} and {rhs_type:?}"
            ));
        }
        let name = format!("__thaw_logical_left_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(name.clone(), lhs_type.clone());
        let left = HirExpr::Var(name.clone());
        let condition = self.truthiness_expr(left.clone(), &lhs_type)?;
        let (then_value, else_value) = if is_and { (rhs, left) } else { (left, rhs) };
        let result = HirExpr::Block(vec![HirStmt::If(
            condition,
            vec![HirStmt::Return(Some(then_value))],
            vec![HirStmt::Return(Some(else_value))],
        )]);
        self.wrap_call_argument_bindings(result, &[(name, lhs_type, lhs)])
    }

    fn coerce_primitive_to_string(&mut self, value: HirExpr) -> Result<HirExpr, String> {
        match self.infer_expr_type(&value)? {
            HirType::Str => Ok(value),
            HirType::Bool => Ok(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_bool_to_string".to_string())),
                vec![value],
            )),
            HirType::F64 => Ok(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_number_to_string".to_string())),
                vec![value],
            )),
            HirType::Object(_) => Ok(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_object_to_string".to_string())),
                vec![value],
            )),
            HirType::Array(element) => {
                let builtin = match element.as_ref() {
                    HirType::F64 => "__thaw_number_array_to_string",
                    HirType::Str => "__thaw_string_array_to_string",
                    HirType::Bool => "__thaw_bool_array_to_string",
                    HirType::Object(_) => "__thaw_object_array_to_string",
                    other => {
                        return Err(format!(
                            "array string conversion does not support element type {other:?}"
                        ))
                    }
                };
                Ok(HirExpr::Call(
                    Box::new(HirExpr::Var(builtin.to_string())),
                    vec![value],
                ))
            }
            HirType::Tuple(elements) => {
                let tuple_type = HirType::Tuple(elements.clone());
                let (tuple, binding) = if matches!(value, HirExpr::Var(_) | HirExpr::TypedIndex(..))
                {
                    (value, None)
                } else {
                    let name = format!("__thaw_string_tuple_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(name.clone(), tuple_type.clone());
                    (
                        HirExpr::Var(name.clone()),
                        Some((name, tuple_type.clone(), value)),
                    )
                };
                let mut result = HirExpr::Lit(HirLit::Str(String::new()));
                for (index, element) in elements.iter().enumerate() {
                    if index != 0 {
                        result = HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_string_concat".to_string())),
                            vec![result, HirExpr::Lit(HirLit::Str(",".to_string()))],
                        );
                    }
                    let part = self.coerce_primitive_to_string(HirExpr::TypedIndex(
                        Box::new(tuple.clone()),
                        Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                        element.clone(),
                    ))?;
                    result = HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_string_concat".to_string())),
                        vec![result, part],
                    );
                }
                match binding {
                    Some(binding) => self.wrap_call_argument_bindings(result, &[binding]),
                    None => Ok(result),
                }
            }
            other => Err(format!(
                "string concatenation cannot convert native type {other:?}"
            )),
        }
    }

    fn join_tuple(
        &mut self,
        value: HirExpr,
        elements: Vec<HirType>,
        separator: HirExpr,
    ) -> Result<HirExpr, String> {
        let tuple_type = HirType::Tuple(elements.clone());
        let tuple_name = format!("__thaw_join_tuple_{}", self.next_binding);
        self.next_binding += 1;
        let separator_name = format!("__thaw_join_separator_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(tuple_name.clone(), tuple_type.clone());
        self.scope.insert(separator_name.clone(), HirType::Str);
        let tuple = HirExpr::Var(tuple_name.clone());
        let separator_var = HirExpr::Var(separator_name.clone());
        let mut result = HirExpr::Lit(HirLit::Str(String::new()));
        for (index, element) in elements.into_iter().enumerate() {
            if index != 0 {
                result = HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_string_concat".to_string())),
                    vec![result, separator_var.clone()],
                );
            }
            let part = self.coerce_primitive_to_string(HirExpr::TypedIndex(
                Box::new(tuple.clone()),
                Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                element,
            ))?;
            result = HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_string_concat".to_string())),
                vec![result, part],
            );
        }
        self.wrap_call_argument_bindings(
            result,
            &[
                (tuple_name, tuple_type, value),
                (separator_name, HirType::Str, separator),
            ],
        )
    }

    fn lower_loose_equality(
        &mut self,
        mut lhs: HirExpr,
        mut rhs: HirExpr,
    ) -> Result<HirExpr, String> {
        let lhs_type = self.infer_expr_type(&lhs)?;
        let rhs_type = self.infer_expr_type(&rhs)?;
        if lhs_type == rhs_type {
            return Ok(HirExpr::BinOp(BinOp::EqEqEq, Box::new(lhs), Box::new(rhs)));
        }
        lhs = self.coerce_primitive_to_number(lhs)?;
        rhs = self.coerce_primitive_to_number(rhs)?;
        Ok(HirExpr::BinOp(BinOp::EqEqEq, Box::new(lhs), Box::new(rhs)))
    }

    fn coerce_primitive_to_number(&mut self, value: HirExpr) -> Result<HirExpr, String> {
        match self.infer_expr_type(&value)? {
            HirType::F64 => Ok(value),
            HirType::Bool => Ok(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_bool_to_number".to_string())),
                vec![value],
            )),
            HirType::Str => Ok(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_string_to_number".to_string())),
                vec![value],
            )),
            HirType::Array(_) | HirType::Tuple(_) | HirType::Object(_) => {
                let string = self.coerce_primitive_to_string(value)?;
                Ok(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_string_to_number".to_string())),
                    vec![string],
                ))
            }
            other => Err(format!(
                "numeric conversion is not defined for native type {other:?}"
            )),
        }
    }

    fn lower_relational(
        &mut self,
        lhs: HirExpr,
        rhs: HirExpr,
        op: BinOp,
    ) -> Result<HirExpr, String> {
        if self.infer_expr_type(&lhs)? == HirType::Str
            && self.infer_expr_type(&rhs)? == HirType::Str
        {
            return Ok(HirExpr::Call(
                Box::new(HirExpr::Var(
                    match op {
                        BinOp::Lt => "__thaw_string_lt",
                        BinOp::Gt => "__thaw_string_gt",
                        BinOp::LtEq => "__thaw_string_lte",
                        BinOp::GtEq => "__thaw_string_gte",
                        _ => unreachable!(),
                    }
                    .to_string(),
                )),
                vec![lhs, rhs],
            ));
        }
        Ok(HirExpr::BinOp(
            op,
            Box::new(self.coerce_primitive_to_number(lhs)?),
            Box::new(self.coerce_primitive_to_number(rhs)?),
        ))
    }

    fn lower_expr(&mut self, expr: &Expr) -> Result<HirExpr, String> {
        match expr {
            Expr::Lit(Lit::Num(n)) => Ok(HirExpr::Lit(HirLit::F64(n.value))),
            Expr::Lit(Lit::Str(s)) => Ok(HirExpr::Lit(HirLit::Str(
                s.value.to_string_lossy().into_owned(),
            ))),
            Expr::Lit(Lit::Bool(b)) => Ok(HirExpr::Lit(HirLit::Bool(b.value))),
            Expr::Ident(ident) => {
                let name = self.resolve_binding(ident.sym.as_ref());
                if !self.scope.contains_key(&name) && !self.signatures.contains_key(&name) {
                    match ident.sym.as_ref() {
                        "NaN" => return Ok(HirExpr::Lit(HirLit::F64(f64::NAN))),
                        "Infinity" => return Ok(HirExpr::Lit(HirLit::F64(f64::INFINITY))),
                        _ => {}
                    }
                }
                Ok(HirExpr::Var(name))
            }
            Expr::Paren(paren) => self.lower_expr(&paren.expr),

            Expr::Seq(sequence) => {
                let mut values = sequence
                    .exprs
                    .iter()
                    .map(|expr| self.lower_expr(expr))
                    .collect::<Result<Vec<_>, _>>()?;
                let last = values
                    .pop()
                    .ok_or("sequence expression must contain at least one value")?;
                let result_type = self.infer_expr_type(&last)?;
                let mut statements = values
                    .into_iter()
                    .map(HirStmt::Expr)
                    .collect::<Vec<_>>();
                if result_type == HirType::Void {
                    statements.push(HirStmt::Expr(last));
                } else {
                    statements.push(HirStmt::Return(Some(last)));
                }
                let body = HirExpr::Block(statements);
                let mut referenced = BTreeSet::new();
                collect_referenced_bindings(&body, &mut referenced);
                let captures = referenced
                    .into_iter()
                    .filter_map(|name| {
                        self.scope
                            .get(&name)
                            .cloned()
                            .map(|ty| HirParam { name, ty })
                    })
                    .collect();
                Ok(HirExpr::Call(
                    Box::new(HirExpr::Lambda(
                        captures,
                        Vec::new(),
                        result_type,
                        Box::new(body),
                    )),
                    Vec::new(),
                ))
            }

            Expr::Tpl(template) => {
                let mut parts = Vec::with_capacity(template.quasis.len() + template.exprs.len());
                for (index, quasi) in template.quasis.iter().enumerate() {
                    let text = quasi
                        .cooked
                        .as_ref()
                        .map(|cooked| cooked.to_string_lossy().into_owned())
                        .unwrap_or_else(|| quasi.raw.to_string());
                    if !text.is_empty() {
                        parts.push(HirExpr::Lit(HirLit::Str(text)));
                    }
                    if let Some(expression) = template.exprs.get(index) {
                        let mut value = self.lower_expr(expression)?;
                        value = self.coerce_primitive_to_string(value)?;
                        parts.push(value);
                    }
                }
                let mut parts = parts.into_iter();
                let Some(mut result) = parts.next() else {
                    return Ok(HirExpr::Lit(HirLit::Str(String::new())));
                };
                for part in parts {
                    result = HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_string_concat".to_string())),
                        vec![result, part],
                    );
                }
                Ok(result)
            }

            Expr::Bin(bin) => {
                let mut lhs = self.lower_expr(&bin.left)?;
                let mut rhs = self.lower_expr(&bin.right)?;
                let mut bindings = Vec::new();
                if !matches!(
                    bin.op,
                    BinaryOp::In | BinaryOp::LogicalAnd | BinaryOp::LogicalOr
                ) && contains_await(&rhs)
                {
                    let lhs_type = self.infer_expr_type(&lhs)?;
                    let lhs_name = format!("__thaw_binary_left_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(lhs_name.clone(), lhs_type.clone());
                    bindings.push((lhs_name.clone(), lhs_type, lhs));
                    lhs = HirExpr::Var(lhs_name);

                    let rhs_type = self.infer_expr_type(&rhs)?;
                    let rhs_name = format!("__thaw_binary_right_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(rhs_name.clone(), rhs_type.clone());
                    bindings.push((rhs_name.clone(), rhs_type, rhs));
                    rhs = HirExpr::Var(rhs_name);
                }
                let value = match bin.op {
                    BinaryOp::In => {
                        let HirExpr::Lit(HirLit::Str(key)) = &lhs else {
                            return Err("fixed object `in` keys must be string literals".into());
                        };
                        let HirType::Object(fields) = self.infer_expr_type(&rhs)? else {
                            return Err("`in` currently requires a fixed-shape object".into());
                        };
                        let exists = fields.iter().any(|(name, _)| name == key);
                        let left_name = format!("__thaw_in_key_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(left_name.clone(), HirType::Str);
                        let right_type = HirType::Object(fields);
                        let right_name = format!("__thaw_in_object_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(right_name.clone(), right_type.clone());
                        self.wrap_call_argument_bindings(
                            HirExpr::Lit(HirLit::Bool(exists)),
                            &[
                                (left_name, HirType::Str, lhs),
                                (right_name, right_type, rhs),
                            ],
                        )?
                    }
                    BinaryOp::LogicalAnd | BinaryOp::LogicalOr => {
                        self.lower_logical_expr(
                            lhs,
                            rhs,
                            bin.op == BinaryOp::LogicalAnd,
                        )?
                    }
                    BinaryOp::Lt => self.lower_relational(lhs, rhs, BinOp::Lt)?,
                    BinaryOp::Gt => self.lower_relational(lhs, rhs, BinOp::Gt)?,
                    BinaryOp::LtEq => self.lower_relational(lhs, rhs, BinOp::LtEq)?,
                    BinaryOp::GtEq => self.lower_relational(lhs, rhs, BinOp::GtEq)?,
                    BinaryOp::NotEqEq => HirExpr::BinOp(
                        BinOp::EqEqEq,
                        Box::new(HirExpr::BinOp(
                            BinOp::EqEqEq,
                            Box::new(lhs),
                            Box::new(rhs),
                        )),
                        Box::new(HirExpr::Lit(HirLit::Bool(false))),
                    ),
                    BinaryOp::EqEq => self.lower_loose_equality(lhs, rhs)?,
                    BinaryOp::NotEq => HirExpr::BinOp(
                        BinOp::EqEqEq,
                        Box::new(self.lower_loose_equality(lhs, rhs)?),
                        Box::new(HirExpr::Lit(HirLit::Bool(false))),
                    ),
                    BinaryOp::Add
                        if self.infer_expr_type(&lhs)? == HirType::Str
                            || self.infer_expr_type(&rhs)? == HirType::Str =>
                    {
                        HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_string_concat".to_string())),
                            vec![
                                self.coerce_primitive_to_string(lhs)?,
                                self.coerce_primitive_to_string(rhs)?,
                            ],
                        )
                    }
                    other => HirExpr::BinOp(
                        lower_bin_op(other)?,
                        Box::new(lhs),
                        Box::new(rhs),
                    ),
                };
                self.infer_expr_type(&value)?;
                self.wrap_call_argument_bindings(value, &bindings)
            }

            Expr::Unary(unary) => {
                let value = self.lower_expr(&unary.arg)?;
                let lowered = match unary.op {
                    UnaryOp::Minus => {
                        self.expect_type(&HirType::F64, &value, "unary minus")?;
                        HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_number_neg".to_string())),
                            vec![value],
                        )
                    }
                    UnaryOp::Plus => {
                        self.expect_type(&HirType::F64, &value, "unary plus")?;
                        value
                    }
                    UnaryOp::Bang => {
                        self.expect_type(&HirType::Bool, &value, "logical not")?;
                        HirExpr::BinOp(
                            BinOp::EqEqEq,
                            Box::new(value),
                            Box::new(HirExpr::Lit(HirLit::Bool(false))),
                        )
                    }
                    UnaryOp::Tilde => {
                        self.expect_type(&HirType::F64, &value, "bitwise not")?;
                        HirExpr::BinOp(
                            BinOp::BitXor,
                            Box::new(value),
                            Box::new(HirExpr::Lit(HirLit::F64(-1.0))),
                        )
                    }
                    UnaryOp::TypeOf => {
                        let operand_type = if let HirExpr::Var(name) = &value {
                            self.signatures.get(name).map(|signature| {
                                let ret = if signature.is_async {
                                    HirType::Promise(Box::new(signature.ret.clone()))
                                } else {
                                    signature.ret.clone()
                                };
                                HirType::Function(signature.params.clone(), Box::new(ret))
                            })
                        } else {
                            None
                        }
                        .map(Ok)
                        .unwrap_or_else(|| self.infer_expr_type(&value))?;
                        let Some(type_name) = native_typeof_name(&operand_type) else {
                            return Err(format!(
                                "`typeof` requires one statically known runtime category, got {operand_type:?}"
                            ));
                        };
                        if matches!(&value, HirExpr::Var(_))
                            && matches!(operand_type, HirType::Function(_, _))
                        {
                            HirExpr::Lit(HirLit::Str(type_name.into()))
                        } else {
                        let parameter = format!("__thaw_typeof_{}", self.next_binding);
                        self.next_binding += 1;
                        HirExpr::Call(
                            Box::new(HirExpr::Lambda(
                                Vec::new(),
                                vec![HirParam {
                                    name: parameter,
                                    ty: operand_type,
                                }],
                                HirType::Str,
                                Box::new(HirExpr::Lit(HirLit::Str(type_name.into()))),
                            )),
                            vec![value],
                            )
                        }
                    }
                    UnaryOp::Void => {
                        let body = HirExpr::Block(vec![HirStmt::Expr(value)]);
                        let mut referenced = BTreeSet::new();
                        collect_referenced_bindings(&body, &mut referenced);
                        let captures = referenced
                            .into_iter()
                            .filter_map(|name| {
                                self.scope
                                    .get(&name)
                                    .cloned()
                                    .map(|ty| HirParam { name, ty })
                            })
                            .collect();
                        HirExpr::Call(
                            Box::new(HirExpr::Lambda(
                                captures,
                                Vec::new(),
                                HirType::Void,
                                Box::new(body),
                            )),
                            Vec::new(),
                        )
                    }
                    other => return Err(format!("unsupported unary operator {other:?}")),
                };
                self.infer_expr_type(&lowered)?;
                Ok(lowered)
            }

            Expr::Cond(conditional) => {
                let test = self.lower_expr(&conditional.test)?;
                self.expect_type(&HirType::Bool, &test, "conditional expression test")?;
                let consequent = self.lower_expr(&conditional.cons)?;
                let alternate = self.lower_expr(&conditional.alt)?;
                let consequent_type = self.infer_expr_type(&consequent)?;
                let alternate_type = self.infer_expr_type(&alternate)?;
                if consequent_type != alternate_type {
                    return Err(format!(
                        "conditional expression branches have incompatible types {consequent_type:?} and {alternate_type:?}"
                    ));
                }
                let body = HirExpr::Block(vec![HirStmt::If(
                    test,
                    vec![HirStmt::Return(Some(consequent))],
                    vec![HirStmt::Return(Some(alternate))],
                )]);
                let mut referenced = BTreeSet::new();
                collect_referenced_bindings(&body, &mut referenced);
                let captures = referenced
                    .into_iter()
                    .filter_map(|name| {
                        self.scope
                            .get(&name)
                            .cloned()
                            .map(|ty| HirParam { name, ty })
                    })
                    .collect();
                Ok(HirExpr::Call(
                    Box::new(HirExpr::Lambda(
                        captures,
                        Vec::new(),
                        consequent_type,
                        Box::new(body),
                    )),
                    Vec::new(),
                ))
            }

            Expr::Call(call) => self.lower_call(call),

            Expr::OptChain(chain) => match chain.base.as_ref() {
                OptChainBase::Member(member) => self.lower_member_read(member),
                OptChainBase::Call(call) => {
                    let call = CallExpr::from(call.clone());
                    self.lower_call(&call)
                }
            },

            Expr::Arrow(arrow) => self.lower_arrow(arrow),

            Expr::Array(array_lit) => {
                if array_lit
                    .elems
                    .iter()
                    .all(|element| element.as_ref().is_some_and(|element| element.spread.is_none()))
                {
                    let values = array_lit
                        .elems
                        .iter()
                        .map(|element| {
                            self.lower_expr(
                                &element.as_ref().expect("checked array element").expr,
                            )
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    let preserve_order = values.iter().any(contains_await);
                    let mut bindings = Vec::new();
                    let values = if preserve_order {
                        values
                            .into_iter()
                            .enumerate()
                            .map(|(position, value)| {
                                let ty = self.infer_expr_type(&value)?;
                                let name = format!(
                                    "__thaw_array_element_{}_{}",
                                    position, self.next_binding
                                );
                                self.next_binding += 1;
                                self.scope.insert(name.clone(), ty.clone());
                                bindings.push((name.clone(), ty, value));
                                Ok(HirExpr::Var(name))
                            })
                            .collect::<Result<Vec<_>, String>>()?
                    } else {
                        values
                    };
                    let value = HirExpr::ArrayLit(values);
                    self.infer_expr_type(&value)?;
                    return self.wrap_call_argument_bindings(value, &bindings);
                }
                let mut parts = Vec::new();
                let mut pending = Vec::new();
                let mut element_type: Option<HirType> = None;
                for element in &array_lit.elems {
                    let Some(element) = element else {
                        return Err("elisions are not supported in array literals".into());
                    };
                    let value = self.lower_expr(&element.expr)?;
                    if element.spread.is_some() {
                        if !pending.is_empty() {
                            parts.push(HirExpr::ArrayLit(std::mem::take(&mut pending)));
                        }
                        let HirType::Array(spread_element) = self.infer_expr_type(&value)? else {
                            return Err("array spread source must be a typed array".into());
                        };
                        if let Some(expected) = &element_type {
                            if expected != spread_element.as_ref() {
                                return Err(format!(
                                    "array spread element type {:?} does not match {expected:?}",
                                    spread_element
                                ));
                            }
                        } else {
                            element_type = Some(spread_element.as_ref().clone());
                        }
                        parts.push(value);
                    } else {
                        let actual = self.infer_expr_type(&value)?;
                        if let Some(expected) = &element_type {
                            if expected != &actual {
                                return Err(format!(
                                    "array element type {actual:?} does not match {expected:?}"
                                ));
                            }
                        } else {
                            element_type = Some(actual);
                        }
                        pending.push(value);
                    }
                }
                if !pending.is_empty() {
                    parts.push(HirExpr::ArrayLit(pending));
                }
                let element_type = element_type.unwrap_or(HirType::F64);
                if !parts.iter().any(contains_await) {
                    return Ok(HirExpr::ArrayConcat(parts, element_type));
                }
                let parts = parts
                    .into_iter()
                    .flat_map(|part| match part {
                        HirExpr::ArrayLit(values) => values
                            .into_iter()
                            .map(|value| HirExpr::ArrayLit(vec![value]))
                            .collect(),
                        other => vec![other],
                    })
                    .collect::<Vec<_>>();
                let mut bindings = Vec::with_capacity(parts.len());
                let mut ordered = Vec::with_capacity(parts.len());
                for (position, part) in parts.into_iter().enumerate() {
                    let ty = self.infer_expr_type(&part)?;
                    let name = format!("__thaw_array_part_{}_{}", position, self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(name.clone(), ty.clone());
                    bindings.push((name.clone(), ty, part));
                    ordered.push(HirExpr::Var(name));
                }
                self.wrap_call_argument_bindings(
                    HirExpr::ArrayConcat(ordered, element_type),
                    &bindings,
                )
            }

            Expr::Object(obj_lit) => self.lower_object_lit(obj_lit),

            Expr::Member(member) => self.lower_member_read(member),

            Expr::Assign(assign) => self.lower_assign(assign),

            Expr::Update(update) => self.lower_update(update),

            Expr::Await(await_expr) => {
                let value = self.lower_expr(&await_expr.arg)?;
                if let HirType::Promise(resolved) = self.infer_expr_type(&value)? {
                    if *resolved == HirType::Void {
                        Ok(HirExpr::Await(Box::new(value)))
                    } else {
                        Ok(HirExpr::AwaitPromise(Box::new(value), *resolved))
                    }
                } else {
                    Ok(HirExpr::Await(Box::new(value)))
                }
            }

            Expr::New(new_expr) => self.lower_promise_new(new_expr),

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
            let mut destructuring = Vec::new();
            for (pattern, param) in arrow.params.iter().zip(source_params) {
                let name = self.bind_local(&param.name, param.ty.clone());
                if !matches!(pattern, Pat::Ident(_)) {
                    destructuring.push((pattern, name.clone(), param.ty.clone()));
                }
                params.push(HirParam { name, ty: param.ty });
            }
            let mut prefix = Vec::new();
            for (pattern, name, ty) in destructuring {
                self.lower_binding_pattern(pattern, HirExpr::Var(name), &ty, &mut prefix)?;
            }
            self.ret_type = declared_return.clone().unwrap_or(HirType::Dynamic);
            let (body, inferred_return) = match arrow.body.as_ref() {
                ArrowFunctionBody::Expr(expr) => {
                    let expression = self.lower_expr(expr)?;
                    if let Some(expected) = &declared_return {
                        self.expect_type(expected, &expression, "arrow function return value")?;
                    }
                    let inferred = self.infer_expr_type(&expression)?;
                    let body = if prefix.is_empty() {
                        expression
                    } else {
                        prefix.push(HirStmt::Return(Some(expression)));
                        HirExpr::Block(prefix)
                    };
                    (body, inferred)
                }
                ArrowFunctionBody::FunctionBody(block) => {
                    let mut stmts = prefix;
                    stmts.extend(self.lower_stmts(&block.stmts)?);
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

    fn lower_contextual_arrow(
        &mut self,
        arrow: &swc_ecma_ast::ArrowExpr,
        parameter_types: &[HirType],
        expected_return: Option<&HirType>,
    ) -> Result<HirExpr, String> {
        if arrow.is_async || arrow.is_generator || arrow.type_params.is_some() {
            return Err("async, generator, and generic Promise callbacks are not supported".into());
        }
        if arrow.params.len() != parameter_types.len() {
            return Err(format!(
                "Promise callback expects {} parameter(s), got {}",
                parameter_types.len(),
                arrow.params.len()
            ));
        }
        let saved_scope = self.scope.clone();
        let saved_bindings = self.bindings.clone();
        let saved_return = self.ret_type.clone();
        let result = (|| {
            let mut params = Vec::with_capacity(parameter_types.len());
            let mut destructuring = Vec::new();
            for (pat, ty) in arrow.params.iter().zip(parameter_types) {
                let source_name = match pat {
                    Pat::Ident(binding) => binding.id.sym.to_string(),
                    Pat::Object(pattern) => format!("__thaw_param_{}", pattern.span.lo.0),
                    Pat::Array(pattern) => format!("__thaw_param_{}", pattern.span.lo.0),
                    _ => return Err("unsupported Promise callback parameter pattern".into()),
                };
                let name = self.bind_local(&source_name, ty.clone());
                if !matches!(pat, Pat::Ident(_)) {
                    destructuring.push((pat, name.clone(), ty.clone()));
                }
                params.push(HirParam {
                    name,
                    ty: ty.clone(),
                });
            }
            let mut prefix = Vec::new();
            for (pattern, name, ty) in destructuring {
                self.lower_binding_pattern(pattern, HirExpr::Var(name), &ty, &mut prefix)?;
            }
            self.ret_type = expected_return.cloned().unwrap_or(HirType::Dynamic);
            let (body, inferred) = match arrow.body.as_ref() {
                ArrowFunctionBody::Expr(expr) => {
                    let expression = self.lower_expr(expr)?;
                    let inferred = self.infer_expr_type(&expression)?;
                    let body = if prefix.is_empty() {
                        expression
                    } else {
                        prefix.push(HirStmt::Return(Some(expression)));
                        HirExpr::Block(prefix)
                    };
                    (body, inferred)
                }
                ArrowFunctionBody::FunctionBody(block) => {
                    let mut stmts = prefix;
                    stmts.extend(self.lower_stmts(&block.stmts)?);
                    let inferred = self.infer_return_type(&stmts)?;
                    (HirExpr::Block(stmts), inferred)
                }
            };
            if let Some(expected) = expected_return {
                if inferred != *expected && inferred != HirType::Dynamic {
                    return Err(format!(
                        "Promise callback returns {inferred:?}, expected {expected:?}"
                    ));
                }
            }
            let ret = expected_return.cloned().unwrap_or(inferred);
            if let Some(expected) = expected_return {
                debug_assert_eq!(&ret, expected);
            }
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
            Ok(HirExpr::Lambda(captures, params, ret, Box::new(body)))
        })();
        self.scope = saved_scope;
        self.bindings = saved_bindings;
        self.ret_type = saved_return;
        result
    }

    fn lower_promise_callback(
        &mut self,
        expr: &Expr,
        parameter_types: &[HirType],
        expected_return: Option<&HirType>,
    ) -> Result<HirExpr, String> {
        let callback = match expr {
            Expr::Arrow(arrow) => {
                return self.lower_contextual_arrow(arrow, parameter_types, expected_return)
            }
            Expr::Ident(ident) => {
                let name = self.resolve_binding(ident.sym.as_ref());
                if self.scope.contains_key(&name) {
                    self.lower_expr(expr)?
                } else {
                    let signature = self
                        .signatures
                        .get(&name)
                        .ok_or_else(|| format!("unknown Promise callback `{name}`"))?;
                    if !signature.generic_type_params.is_empty() {
                        return Err("generic Promise callbacks are not supported".into());
                    }
                    let ret = if signature.is_async {
                        HirType::Promise(Box::new(signature.ret.clone()))
                    } else {
                        signature.ret.clone()
                    };
                    HirExpr::FunctionRef(name, signature.params.clone(), ret)
                }
            }
            _ => return Err("Promise callback must be an arrow or function value".into()),
        };
        let HirType::Function(params, ret) = self.infer_expr_type(&callback)? else {
            return Err("Promise callback is not a function value".into());
        };
        if params != parameter_types {
            return Err(format!(
                "Promise callback has parameters {params:?}, expected {parameter_types:?}"
            ));
        }
        if let Some(expected) = expected_return {
            if *ret != *expected {
                return Err(format!(
                    "Promise callback returns {:?}, expected {expected:?}",
                    ret
                ));
            }
        }
        Ok(callback)
    }

    fn lower_promise_new(&mut self, new_expr: &swc_ecma_ast::NewExpr) -> Result<HirExpr, String> {
        let Expr::Ident(callee) = new_expr.callee.as_ref() else {
            return Err("only `new Promise<T>(...)` is supported".into());
        };
        if callee.sym != *"Promise" {
            return Err("only `new Promise<T>(...)` is supported".into());
        }
        let args = new_expr.args.as_deref().unwrap_or_default();
        let [executor] = args else {
            return Err("`new Promise<T>` expects exactly one executor".into());
        };
        if executor.spread.is_some() {
            return Err("Promise executor spread is not supported".into());
        }
        let inferred_resolve = self.infer_promise_constructor_type(&executor.expr);
        let (resolved, assimilates) = if let Some(type_args) = &new_expr.type_args {
            let [resolved] = type_args.params.as_slice() else {
                return Err("`new Promise` requires exactly one type argument".into());
            };
            let resolved = lower_ts_type(resolved, self.interfaces, self.generic_interfaces)?;
            let assimilates = matches!(
                inferred_resolve.as_ref().ok(),
                Some(HirType::Promise(inner)) if inner.as_ref() == &resolved
            );
            (resolved, assimilates)
        } else {
            match inferred_resolve? {
                HirType::Promise(inner) => (*inner, true),
                resolved => (resolved, false),
            }
        };
        let resolve_value = if assimilates {
            vec![HirType::Promise(Box::new(resolved.clone()))]
        } else if resolved == HirType::Void {
            Vec::new()
        } else {
            vec![resolved.clone()]
        };
        let resolve = HirType::Function(resolve_value, Box::new(HirType::Void));
        let reject = HirType::Function(vec![HirType::Str], Box::new(HirType::Void));
        let executor =
            self.lower_promise_callback(&executor.expr, &[resolve, reject], Some(&HirType::Void))?;
        Ok(HirExpr::PromiseNew(
            Box::new(executor),
            resolved,
            assimilates,
        ))
    }

    fn infer_promise_constructor_type(&mut self, executor: &Expr) -> Result<HirType, String> {
        use swc_ecma_visit::{Visit, VisitMut, VisitMutWith, VisitWith};

        if let Expr::Ident(ident) = executor {
            let name = self.resolve_binding(ident.sym.as_ref());
            let callback_type = self.scope.get(&name).cloned().or_else(|| {
                self.signatures.get(&name).map(|signature| {
                    HirType::Function(
                        signature.params.clone(),
                        Box::new(if signature.is_async {
                            HirType::Promise(Box::new(signature.ret.clone()))
                        } else {
                            signature.ret.clone()
                        }),
                    )
                })
            });
            let Some(HirType::Function(params, _)) = callback_type else {
                return Err("cannot infer Promise type from executor function".into());
            };
            let Some(HirType::Function(resolve_params, _)) = params.first() else {
                return Err("cannot infer Promise type from executor resolve parameter".into());
            };
            let [resolved] = resolve_params.as_slice() else {
                return Err("Promise resolve callback must take exactly one value".into());
            };
            return Ok(resolved.clone());
        }

        let Expr::Arrow(arrow) = executor else {
            return Err("cannot infer Promise type from this executor".into());
        };
        let Some(Pat::Ident(resolve_binding)) = arrow.params.first() else {
            return Err("cannot infer Promise type without a resolve parameter".into());
        };
        struct ResolveCalls {
            name: Symbol,
            values: Vec<Expr>,
            locals: HashMap<Symbol, Expr>,
        }
        impl Visit for ResolveCalls {
            fn visit_var_declarator(&mut self, declarator: &swc_ecma_ast::VarDeclarator) {
                if let (Pat::Ident(binding), Some(initializer)) =
                    (&declarator.name, &declarator.init)
                {
                    self.locals
                        .insert(binding.id.sym.to_string(), initializer.as_ref().clone());
                }
                declarator.visit_children_with(self);
            }

            fn visit_call_expr(&mut self, call: &CallExpr) {
                if let Callee::Expr(callee) = &call.callee {
                    if matches!(callee.as_ref(), Expr::Ident(ident) if ident.sym == self.name) {
                        if let [arg] = call.args.as_slice() {
                            if arg.spread.is_none() {
                                self.values.push(arg.expr.as_ref().clone());
                            }
                        }
                    }
                }
                call.visit_children_with(self);
            }
        }
        let mut calls = ResolveCalls {
            name: resolve_binding.id.sym.to_string(),
            values: Vec::new(),
            locals: HashMap::new(),
        };
        arrow.body.visit_with(&mut calls);

        struct ExpandExecutorLocals<'a> {
            locals: &'a HashMap<Symbol, Expr>,
            expanding: BTreeSet<Symbol>,
        }
        impl VisitMut for ExpandExecutorLocals<'_> {
            fn visit_mut_expr(&mut self, expr: &mut Expr) {
                if let Expr::Ident(ident) = expr {
                    let name = ident.sym.to_string();
                    if let Some(initializer) = self.locals.get(&name) {
                        if self.expanding.insert(name.clone()) {
                            let mut replacement = initializer.clone();
                            replacement.visit_mut_with(self);
                            self.expanding.remove(&name);
                            *expr = replacement;
                        }
                        return;
                    }
                }
                expr.visit_mut_children_with(self);
            }
        }
        let mut inferred = None;
        for mut value in calls.values {
            value.visit_mut_with(&mut ExpandExecutorLocals {
                locals: &calls.locals,
                expanding: BTreeSet::new(),
            });
            let value = self.lower_expr(&value).map_err(|error| {
                format!("cannot infer Promise type from resolve argument: {error}")
            })?;
            let actual = self.infer_expr_type(&value)?;
            if let Some(expected) = &inferred {
                if expected != &actual {
                    return Err(format!(
                        "conflicting Promise resolve types: {expected:?} and {actual:?}"
                    ));
                }
            } else {
                inferred = Some(actual);
            }
        }
        inferred.ok_or_else(|| {
            "cannot infer Promise type because the executor has no resolvable `resolve(value)` call"
                .into()
        })
    }

    fn lower_object_lit(&mut self, obj_lit: &SwcObjectLit) -> Result<HirExpr, String> {
        struct AwaitFinder(bool);
        impl Visit for AwaitFinder {
            fn visit_await_expr(&mut self, _: &AwaitExpr) {
                self.0 = true;
            }
        }
        let mut finder = AwaitFinder(false);
        obj_lit.visit_with(&mut finder);
        if finder.0 {
            return self.lower_ordered_await_object_lit(obj_lit);
        }
        let mut fields = Vec::new();
        let mut evaluated_spreads: Vec<(Symbol, HirType, HirExpr)> = Vec::new();
        for property in &obj_lit.props {
            let additions = match property {
                PropOrSpread::Spread(spread) => {
                    let source = self.lower_expr(&spread.expr)?;
                    let source_type = self.infer_expr_type(&source)?;
                    let HirType::Object(source_fields) = &source_type else {
                        return Err(format!(
                            "cannot spread a value of type {source_type:?} into an object literal"
                        ));
                    };
                    if let HirExpr::ObjectLit(source_values) = &source {
                        source_values.clone()
                    } else if matches!(spread.expr.as_ref(), Expr::Ident(_)) {
                        source_fields
                            .iter()
                            .map(|(name, _)| {
                                (
                                    name.clone(),
                                    HirExpr::PropAccess(
                                        Box::new(source.clone()),
                                        source_type.clone(),
                                        name.clone(),
                                    ),
                                )
                            })
                            .collect::<Vec<_>>()
                    } else {
                        let temporary = format!("__thaw_object_spread_{}", self.next_binding);
                        self.next_binding += 1;
                        let additions = source_fields
                            .iter()
                            .map(|(name, _)| {
                                (
                                    name.clone(),
                                    HirExpr::PropAccess(
                                        Box::new(HirExpr::Var(temporary.clone())),
                                        source_type.clone(),
                                        name.clone(),
                                    ),
                                )
                            })
                            .collect();
                        evaluated_spreads.push((temporary, source_type, source));
                        additions
                    }
                }
                PropOrSpread::Prop(prop) => match prop.as_ref() {
                    Prop::KeyValue(KeyValueProp { key, value }) => {
                        let name = match key {
                            PropName::Ident(ident) => ident.sym.to_string(),
                            PropName::Str(s) => s.value.to_string_lossy().into_owned(),
                            PropName::Computed(computed) => match computed.expr.as_ref() {
                                Expr::Lit(Lit::Str(value)) => {
                                    value.value.to_string_lossy().into_owned()
                                }
                                _ => {
                                    return Err(
                                        "computed object literal keys must be string literals"
                                            .to_string(),
                                    )
                                }
                            },
                            _ => return Err("unsupported object literal key".to_string()),
                        };
                        vec![(name, self.lower_expr(value)?)]
                    }
                    Prop::Shorthand(ident) => vec![(
                        ident.sym.to_string(),
                        self.lower_expr(&Expr::Ident(ident.clone()))?,
                    )],
                    _ => {
                        return Err(
                            "only data properties are supported in object literals".to_string()
                        )
                    }
                },
            };
            for (name, value) in additions {
                if let Some((_, existing)) =
                    fields.iter_mut().find(|(existing, _)| existing == &name)
                {
                    *existing = value;
                } else {
                    fields.push((name, value));
                }
            }
        }
        let mut property_bindings = Vec::new();
        if evaluated_spreads.is_empty() && fields.iter().any(|(_, value)| contains_await(value)) {
            for (position, (_, value)) in fields.iter_mut().enumerate() {
                let source = value.clone();
                let ty = self.infer_expr_type(&source)?;
                let name = format!("__thaw_object_field_{}_{}", position, self.next_binding);
                self.next_binding += 1;
                self.scope.insert(name.clone(), ty.clone());
                *value = HirExpr::Var(name.clone());
                property_bindings.push((name, ty, source));
            }
        }
        let mut result = HirExpr::ObjectLit(fields);
        let result_type = self.infer_expr_type(&result)?;
        for index in (0..evaluated_spreads.len()).rev() {
            let (name, ty, source) = &evaluated_spreads[index];
            let captures = evaluated_spreads[..index]
                .iter()
                .map(|(name, ty, _)| HirParam {
                    name: name.clone(),
                    ty: ty.clone(),
                })
                .collect();
            result = HirExpr::Call(
                Box::new(HirExpr::Lambda(
                    captures,
                    vec![HirParam {
                        name: name.clone(),
                        ty: ty.clone(),
                    }],
                    result_type.clone(),
                    Box::new(result),
                )),
                vec![source.clone()],
            );
        }
        self.wrap_call_argument_bindings(result, &property_bindings)
    }

    fn lower_ordered_await_object_lit(
        &mut self,
        obj_lit: &SwcObjectLit,
    ) -> Result<HirExpr, String> {
        let mut fields = Vec::new();
        let mut bindings = Vec::new();
        for (position, property) in obj_lit.props.iter().enumerate() {
            let additions = match property {
                PropOrSpread::Spread(spread) => {
                    let source = self.lower_expr(&spread.expr)?;
                    let source_type = self.infer_expr_type(&source)?;
                    let HirType::Object(source_fields) = &source_type else {
                        return Err(format!(
                            "cannot spread a value of type {source_type:?} into an object literal"
                        ));
                    };
                    let name = format!("__thaw_object_source_{}_{}", position, self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(name.clone(), source_type.clone());
                    bindings.push((name.clone(), source_type.clone(), source));
                    source_fields
                        .iter()
                        .map(|(field, _)| {
                            (
                                field.clone(),
                                HirExpr::PropAccess(
                                    Box::new(HirExpr::Var(name.clone())),
                                    source_type.clone(),
                                    field.clone(),
                                ),
                            )
                        })
                        .collect::<Vec<_>>()
                }
                PropOrSpread::Prop(prop) => {
                    let (field, source) = match prop.as_ref() {
                        Prop::KeyValue(KeyValueProp { key, value }) => {
                            let field = match key {
                                PropName::Ident(ident) => ident.sym.to_string(),
                                PropName::Str(value) => value.value.to_string_lossy().into_owned(),
                                PropName::Computed(computed) => {
                                    match computed.expr.as_ref() {
                                        Expr::Lit(Lit::Str(value)) => {
                                            value.value.to_string_lossy().into_owned()
                                        }
                                        _ => return Err(
                                            "computed object literal keys must be string literals"
                                                .into(),
                                        ),
                                    }
                                }
                                _ => return Err("unsupported object literal key".into()),
                            };
                            (field, self.lower_expr(value)?)
                        }
                        Prop::Shorthand(ident) => (
                            ident.sym.to_string(),
                            self.lower_expr(&Expr::Ident(ident.clone()))?,
                        ),
                        _ => {
                            return Err(
                                "only data properties are supported in object literals".into()
                            )
                        }
                    };
                    let ty = self.infer_expr_type(&source)?;
                    let name = format!("__thaw_object_value_{}_{}", position, self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(name.clone(), ty.clone());
                    bindings.push((name.clone(), ty, source));
                    vec![(field, HirExpr::Var(name))]
                }
            };
            for (name, value) in additions {
                if let Some((_, existing)) =
                    fields.iter_mut().find(|(existing, _)| existing == &name)
                {
                    *existing = value;
                } else {
                    fields.push((name, value));
                }
            }
        }
        self.wrap_call_argument_bindings(HirExpr::ObjectLit(fields), &bindings)
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
                match obj_ty {
                    HirType::Array(element) => {
                        let index = self.lower_expr(&computed.expr)?;
                        self.expect_type(&HirType::F64, &index, "index expression")?;
                        Ok(HirExpr::TypedIndex(
                            Box::new(obj),
                            Box::new(index),
                            *element,
                        ))
                    }
                    HirType::Tuple(elements) => {
                        let index = self.lower_expr(&computed.expr)?;
                        self.expect_type(&HirType::F64, &index, "index expression")?;
                        let HirExpr::Lit(HirLit::F64(position)) = index else {
                            return Err("tuple index must be a numeric literal".into());
                        };
                        let position = position as usize;
                        let element = elements
                            .get(position)
                            .cloned()
                            .ok_or_else(|| format!("tuple index {position} is out of bounds"))?;
                        Ok(HirExpr::TypedIndex(
                            Box::new(obj),
                            Box::new(HirExpr::Lit(HirLit::F64(position as f64))),
                            element,
                        ))
                    }
                    HirType::Object(fields) => {
                        let Expr::Lit(Lit::Str(key)) = computed.expr.as_ref() else {
                            return Err("computed object key must be a string literal".into());
                        };
                        let key = key.value.to_string_lossy().into_owned();
                        if fields.iter().any(|(name, _)| name == &key) {
                            Ok(HirExpr::PropAccess(
                                Box::new(obj),
                                HirType::Object(fields),
                                key,
                            ))
                        } else {
                            Err(format!("object has no field `{key}`"))
                        }
                    }
                    HirType::Json => match computed.expr.as_ref() {
                        Expr::Lit(Lit::Str(key)) => Ok(HirExpr::JsonGet(
                            Box::new(obj),
                            key.value.to_string_lossy().into_owned(),
                        )),
                        _ => {
                            let index = self.lower_expr(&computed.expr)?;
                            self.expect_type(&HirType::F64, &index, "JSON index expression")?;
                            Ok(HirExpr::JsonIndex(Box::new(obj), Box::new(index)))
                        }
                    },
                    other => Err(format!("cannot index into a value of type {other:?}")),
                }
            }
            MemberProp::Ident(prop) => {
                if matches!(member.obj.as_ref(), Expr::Ident(object) if object.sym == *"Math") {
                    let value = match prop.sym.as_ref() {
                        "E" => std::f64::consts::E,
                        "PI" => std::f64::consts::PI,
                        "LN2" => std::f64::consts::LN_2,
                        "LN10" => std::f64::consts::LN_10,
                        "LOG2E" => std::f64::consts::LOG2_E,
                        "LOG10E" => std::f64::consts::LOG10_E,
                        "SQRT1_2" => std::f64::consts::FRAC_1_SQRT_2,
                        "SQRT2" => std::f64::consts::SQRT_2,
                        _ => {
                            return Err(format!("unsupported Math property `{}`", prop.sym));
                        }
                    };
                    return Ok(HirExpr::Lit(HirLit::F64(value)));
                }
                if matches!(member.obj.as_ref(), Expr::Ident(object) if object.sym == *"Number") {
                    let value = match prop.sym.as_ref() {
                        "NaN" => f64::NAN,
                        "POSITIVE_INFINITY" => f64::INFINITY,
                        "NEGATIVE_INFINITY" => f64::NEG_INFINITY,
                        "MAX_VALUE" => f64::MAX,
                        "MIN_VALUE" => f64::from_bits(1),
                        "MAX_SAFE_INTEGER" => 9_007_199_254_740_991.0,
                        "MIN_SAFE_INTEGER" => -9_007_199_254_740_991.0,
                        "EPSILON" => f64::EPSILON,
                        _ => {
                            return Err(format!("unsupported Number property `{}`", prop.sym));
                        }
                    };
                    return Ok(HirExpr::Lit(HirLit::F64(value)));
                }
                let obj = self.lower_expr(&member.obj)?;
                let obj_ty = self.infer_expr_type(&obj)?;
                match &obj_ty {
                    HirType::Array(_) | HirType::Tuple(_) if prop.sym == *"length" => {
                        Ok(HirExpr::ArrayLen(Box::new(obj)))
                    }
                    HirType::Str if prop.sym == *"length" => Ok(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_string_length".to_string())),
                        vec![obj],
                    )),
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

    /// Resolves a computed assignment/update target without confusing the
    /// pointer-compatible array, object, string and JSON layouts.
    fn lower_computed_target(
        &mut self,
        member: &MemberExpr,
        computed: &ComputedPropName,
    ) -> Result<Target, String> {
        let object = self.lower_expr(&member.obj)?;
        let object_type = self.infer_expr_type(&object)?;
        match &object_type {
            HirType::Array(element) if element.as_ref() == &HirType::F64 => {
                let index = self.lower_expr(&computed.expr)?;
                self.expect_type(&HirType::F64, &index, "index expression")?;
                Ok(Target::Index(object, index))
            }
            HirType::Object(fields) => {
                let Expr::Lit(Lit::Str(key)) = computed.expr.as_ref() else {
                    return Err("computed object assignment key must be a string literal".into());
                };
                let key = key.value.to_string_lossy().into_owned();
                if fields.iter().any(|(name, _)| name == &key) {
                    Ok(Target::Prop(object, object_type, key))
                } else {
                    Err(format!("object has no field `{key}`"))
                }
            }
            _ => Err(format!(
                "cannot assign through a computed key on a value of type {object_type:?}"
            )),
        }
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
                MemberProp::Computed(computed) => self.lower_computed_target(member, computed),
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
        if let AssignTarget::Pat(pattern) = &assign.left {
            if assign.op != AssignOp::Assign {
                return Err("destructuring only supports simple `=` assignment".into());
            }
            let pattern = match pattern {
                swc_ecma_ast::AssignTargetPat::Array(pattern) => Pat::Array(pattern.clone()),
                swc_ecma_ast::AssignTargetPat::Object(pattern) => Pat::Object(pattern.clone()),
                swc_ecma_ast::AssignTargetPat::Invalid(_) => {
                    return Err("invalid destructuring assignment target".into())
                }
            };
            let value = self.lower_expr(&assign.right)?;
            let ty = if matches!(pattern, Pat::Array(_)) {
                if let HirExpr::ArrayLit(elements) = &value {
                    HirType::Tuple(
                        elements
                            .iter()
                            .map(|element| self.infer_expr_type(element))
                            .collect::<Result<Vec<_>, _>>()?,
                    )
                } else {
                    self.infer_expr_type(&value)?
                }
            } else {
                self.infer_expr_type(&value)?
            };
            if !matches!(ty, HirType::Object(_) | HirType::Tuple(_)) {
                return Err(format!(
                    "destructuring assignment requires a fixed-shape object or tuple, got {ty:?}"
                ));
            }
            let temporary = format!("__thaw_destructure_assign_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(temporary.clone(), ty.clone());
            let mut statements = Vec::new();
            self.lower_assignment_pattern(
                &pattern,
                HirExpr::Var(temporary.clone()),
                &ty,
                &mut statements,
            )?;
            statements.push(HirStmt::Return(Some(HirExpr::Var(temporary.clone()))));
            return self.wrap_call_argument_bindings(
                HirExpr::Block(statements),
                &[(temporary, ty, value)],
            );
        }

        let mut target = self.lower_assign_target(&assign.left)?;
        let rhs = self.lower_expr(&assign.right)?;
        let mut bindings = Vec::new();

        if assign.op != AssignOp::Assign {
            target = match target {
                Target::Var(name) => Target::Var(name),
                Target::Index(array, index) => {
                    let array_name = format!("__thaw_assign_array_{}", self.next_binding);
                    self.next_binding += 1;
                    let array_type = HirType::Array(Box::new(HirType::F64));
                    self.scope.insert(array_name.clone(), array_type.clone());
                    bindings.push((array_name.clone(), array_type, array));

                    let index_name = format!("__thaw_assign_index_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(index_name.clone(), HirType::F64);
                    bindings.push((index_name.clone(), HirType::F64, index));
                    Target::Index(HirExpr::Var(array_name), HirExpr::Var(index_name))
                }
                Target::Prop(object, object_type, field) => {
                    let object_name = format!("__thaw_assign_object_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(object_name.clone(), object_type.clone());
                    bindings.push((object_name.clone(), object_type.clone(), object));
                    Target::Prop(HirExpr::Var(object_name), object_type, field)
                }
            };
        }

        let value = if assign.op == AssignOp::Assign {
            rhs
        } else if let Some(op) = compound_op(assign.op) {
            let current = target_to_read_expr(&target);
            if assign.op == AssignOp::AddAssign
                && (self.infer_expr_type(&current)? == HirType::Str
                    || self.infer_expr_type(&rhs)? == HirType::Str)
            {
                HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_string_concat".to_string())),
                    vec![
                        self.coerce_primitive_to_string(current)?,
                        self.coerce_primitive_to_string(rhs)?,
                    ],
                )
            } else {
                HirExpr::BinOp(op, Box::new(current), Box::new(rhs))
            }
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

        let result = build_assign(target, value);
        self.wrap_call_argument_bindings(result, &bindings)
    }

    fn lower_assignment_pattern(
        &mut self,
        pattern: &Pat,
        value: HirExpr,
        ty: &HirType,
        statements: &mut Vec<HirStmt>,
    ) -> Result<(), String> {
        match pattern {
            Pat::Ident(binding) => {
                let name = self.resolve_binding(binding.id.sym.as_ref());
                let expected = self
                    .scope
                    .get(&name)
                    .cloned()
                    .ok_or_else(|| format!("assignment to unknown binding `{name}`"))?;
                let value = self.coerce_to_declared(&expected, value)?;
                statements.push(HirStmt::Expr(HirExpr::Assign(name, Box::new(value))));
                Ok(())
            }
            Pat::Object(pattern) => {
                let HirType::Object(fields) = ty else {
                    return Err(format!("object pattern cannot destructure {ty:?}"));
                };
                let mut used = BTreeSet::new();
                for property in &pattern.props {
                    match property {
                        ObjectPatProp::Assign(property) => {
                            if property.value.is_some() {
                                return Err(
                                    "destructuring defaults require undefined support".into()
                                );
                            }
                            let key = property.key.id.sym.to_string();
                            let field_type = fields
                                .iter()
                                .find(|(name, _)| name == &key)
                                .map(|(_, ty)| ty.clone())
                                .ok_or_else(|| format!("object has no field `{key}`"))?;
                            used.insert(key.clone());
                            self.lower_assignment_pattern(
                                &Pat::Ident(property.key.clone()),
                                HirExpr::PropAccess(Box::new(value.clone()), ty.clone(), key),
                                &field_type,
                                statements,
                            )?;
                        }
                        ObjectPatProp::KeyValue(property) => {
                            let key =
                                match &property.key {
                                    PropName::Ident(key) => key.sym.to_string(),
                                    PropName::Str(key) => key.value.to_string_lossy().into_owned(),
                                    PropName::Computed(computed) => match computed.expr.as_ref() {
                                        Expr::Lit(Lit::Str(key)) => {
                                            key.value.to_string_lossy().into_owned()
                                        }
                                        _ => return Err(
                                            "computed destructuring keys must be string literals"
                                                .into(),
                                        ),
                                    },
                                    _ => return Err("unsupported object destructuring key".into()),
                                };
                            let field_type = fields
                                .iter()
                                .find(|(name, _)| name == &key)
                                .map(|(_, ty)| ty.clone())
                                .ok_or_else(|| format!("object has no field `{key}`"))?;
                            used.insert(key.clone());
                            self.lower_assignment_pattern(
                                &property.value,
                                HirExpr::PropAccess(Box::new(value.clone()), ty.clone(), key),
                                &field_type,
                                statements,
                            )?;
                        }
                        ObjectPatProp::Rest(rest) => {
                            let remaining = fields
                                .iter()
                                .filter(|(name, _)| !used.contains(name))
                                .cloned()
                                .collect::<Vec<_>>();
                            let rest_value = HirExpr::ObjectLit(
                                remaining
                                    .iter()
                                    .map(|(name, _)| {
                                        (
                                            name.clone(),
                                            HirExpr::PropAccess(
                                                Box::new(value.clone()),
                                                ty.clone(),
                                                name.clone(),
                                            ),
                                        )
                                    })
                                    .collect(),
                            );
                            self.lower_assignment_pattern(
                                &rest.arg,
                                rest_value,
                                &HirType::Object(remaining),
                                statements,
                            )?;
                        }
                    }
                }
                Ok(())
            }
            Pat::Array(pattern) => {
                let HirType::Tuple(elements) = ty else {
                    return Err(format!(
                        "array pattern requires a fixed-length tuple, got {ty:?}"
                    ));
                };
                for (index, element_pattern) in pattern.elems.iter().enumerate() {
                    let Some(element_pattern) = element_pattern else {
                        continue;
                    };
                    if let Pat::Rest(rest) = element_pattern {
                        let remaining = elements[index..].to_vec();
                        let rest_value = HirExpr::ArrayLit(
                            remaining
                                .iter()
                                .enumerate()
                                .map(|(offset, element)| {
                                    HirExpr::TypedIndex(
                                        Box::new(value.clone()),
                                        Box::new(HirExpr::Lit(HirLit::F64(
                                            (index + offset) as f64,
                                        ))),
                                        element.clone(),
                                    )
                                })
                                .collect(),
                        );
                        let rest_type = if remaining
                            .first()
                            .is_some_and(|first| remaining.iter().all(|element| element == first))
                        {
                            HirType::Array(Box::new(
                                remaining.first().cloned().unwrap_or(HirType::F64),
                            ))
                        } else {
                            HirType::Tuple(remaining)
                        };
                        self.lower_assignment_pattern(
                            &rest.arg, rest_value, &rest_type, statements,
                        )?;
                        break;
                    }
                    let element_type = elements
                        .get(index)
                        .cloned()
                        .ok_or_else(|| format!("tuple pattern index {index} is out of bounds"))?;
                    self.lower_assignment_pattern(
                        element_pattern,
                        HirExpr::TypedIndex(
                            Box::new(value.clone()),
                            Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                            element_type.clone(),
                        ),
                        &element_type,
                        statements,
                    )?;
                }
                Ok(())
            }
            Pat::Assign(_) => Err("destructuring defaults require undefined support".into()),
            Pat::Rest(_) => Err("rest patterns are only valid inside object/array patterns".into()),
            _ => Err("unsupported destructuring assignment target".into()),
        }
    }

    fn lower_update(&mut self, update: &swc_ecma_ast::UpdateExpr) -> Result<HirExpr, String> {
        let target = match update.arg.as_ref() {
            Expr::Ident(ident) => Target::Var(self.resolve_binding(ident.sym.as_ref())),
            Expr::Member(member) => match &member.prop {
                MemberProp::Computed(computed) => self.lower_computed_target(member, computed)?,
                MemberProp::Ident(prop) => {
                    let object = self.lower_expr(&member.obj)?;
                    let object_type = self.infer_expr_type(&object)?;
                    match &object_type {
                        HirType::Object(fields)
                            if fields.iter().any(|(name, ty)| {
                                name == prop.sym.as_str() && *ty == HirType::F64
                            }) =>
                        {
                            Target::Prop(object, object_type, prop.sym.to_string())
                        }
                        _ => {
                            return Err(format!(
                                "cannot apply ++/-- to non-number field `.{}` on {object_type:?}",
                                prop.sym
                            ))
                        }
                    }
                }
                _ => return Err("unsupported ++/-- target".into()),
            },
            _ => return Err("unsupported ++/-- target".into()),
        };

        let op = match update.op {
            UpdateOp::PlusPlus => BinOp::Add,
            UpdateOp::MinusMinus => BinOp::Sub,
        };
        let one = HirExpr::Lit(HirLit::F64(1.0));
        let current = target_to_read_expr(&target);
        self.expect_type(&HirType::F64, &current, "update operand")?;
        if update.prefix {
            let value = HirExpr::BinOp(op, Box::new(current), Box::new(one));
            return Ok(build_assign(target, value));
        }

        let mut bindings = Vec::new();
        let target = match target {
            Target::Var(name) => Target::Var(name),
            Target::Index(array, index) => {
                let array_name = format!("__thaw_update_array_{}", self.next_binding);
                self.next_binding += 1;
                let array_type = HirType::Array(Box::new(HirType::F64));
                self.scope.insert(array_name.clone(), array_type.clone());
                bindings.push((array_name.clone(), array_type, array));

                let index_name = format!("__thaw_update_index_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(index_name.clone(), HirType::F64);
                bindings.push((index_name.clone(), HirType::F64, index));
                Target::Index(HirExpr::Var(array_name), HirExpr::Var(index_name))
            }
            Target::Prop(object, object_type, field) => {
                let object_name = format!("__thaw_update_object_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(object_name.clone(), object_type.clone());
                bindings.push((object_name.clone(), object_type.clone(), object));
                Target::Prop(HirExpr::Var(object_name), object_type, field)
            }
        };
        let old_name = format!("__thaw_update_old_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(old_name.clone(), HirType::F64);
        bindings.push((old_name.clone(), HirType::F64, target_to_read_expr(&target)));
        let old = HirExpr::Var(old_name);
        let updated = HirExpr::BinOp(op, Box::new(old.clone()), Box::new(one));
        let result = HirExpr::Block(vec![
            HirStmt::Expr(build_assign(target, updated)),
            HirStmt::Return(Some(old)),
        ]);
        self.wrap_call_argument_bindings(result, &bindings)
    }

    fn wrap_call_argument_bindings(
        &mut self,
        mut result: HirExpr,
        bindings: &[(Symbol, HirType, HirExpr)],
    ) -> Result<HirExpr, String> {
        if bindings.is_empty() {
            return Ok(result);
        }
        let result_type = self.infer_expr_type(&result)?;
        for index in (0..bindings.len()).rev() {
            let (name, ty, source) = &bindings[index];
            let body = if result_type == HirType::Void {
                HirExpr::Block(vec![HirStmt::Expr(result)])
            } else {
                result
            };
            let mut referenced = BTreeSet::new();
            collect_referenced_bindings(&body, &mut referenced);
            let captures = referenced
                .into_iter()
                .filter(|referenced| referenced != name)
                .filter(|referenced| {
                    bindings
                        .iter()
                        .position(|(binding, _, _)| binding == referenced)
                        .is_none_or(|position| position < index)
                })
                .filter_map(|referenced| {
                    self.scope.get(&referenced).cloned().map(|ty| HirParam {
                        name: referenced,
                        ty,
                    })
                })
                .collect();
            result = HirExpr::Call(
                Box::new(HirExpr::Lambda(
                    captures,
                    vec![HirParam {
                        name: name.clone(),
                        ty: ty.clone(),
                    }],
                    result_type.clone(),
                    Box::new(body),
                )),
                vec![source.clone()],
            );
        }
        Ok(result)
    }

    fn lower_promise_array_value(
        &mut self,
        expr: &Expr,
        combinator: &str,
    ) -> Result<(HirExpr, HirType), String> {
        let values = self.lower_expr(expr)?;
        let HirType::Array(element) = self.infer_expr_type(&values)? else {
            return Err(format!(
                "`Promise.{combinator}` expects an array of promises"
            ));
        };
        let HirType::Promise(element) = *element else {
            return Err(format!(
                "`Promise.{combinator}` expects an array of promises"
            ));
        };
        if *element == HirType::Void {
            return Err(format!(
                "`Promise.{combinator}` elements must not resolve to void"
            ));
        }
        Ok((values, *element))
    }

    fn lower_parse_call(&mut self, call: &CallExpr, parse_int: bool) -> Result<HirExpr, String> {
        let label = if parse_int { "parseInt" } else { "parseFloat" };
        let expected = if parse_int { 1..=2 } else { 1..=1 };
        if !expected.contains(&call.args.len()) {
            return Err(format!(
                "`{label}` expects one{} argument",
                if parse_int { " or two" } else { "" }
            ));
        }
        if call.args.iter().any(|argument| argument.spread.is_some()) {
            return Err("parse function spread is not supported".into());
        }
        let text = self.lower_expr(&call.args[0].expr)?;
        let text = self.coerce_primitive_to_string(text)?;
        let mut arguments = vec![text];
        if parse_int {
            let radix = if let Some(argument) = call.args.get(1) {
                let value = self.lower_expr(&argument.expr)?;
                self.coerce_primitive_to_number(value)?
            } else {
                HirExpr::Lit(HirLit::F64(0.0))
            };
            arguments.push(radix);
        }
        Ok(HirExpr::Call(
            Box::new(HirExpr::Var(
                if parse_int {
                    "__thaw_parse_int"
                } else {
                    "__thaw_parse_float"
                }
                .to_string(),
            )),
            arguments,
        ))
    }

    fn lower_call(&mut self, call: &CallExpr) -> Result<HirExpr, String> {
        let Callee::Expr(callee_expr) = &call.callee else {
            return Err("unsupported callee (super/import calls not supported)".into());
        };

        if let Expr::Member(member) = callee_expr.as_ref() {
            if let MemberProp::Ident(property) = &member.prop {
                if let Expr::Ident(object) = member.obj.as_ref() {
                    if object.sym == *"Array" && property.sym == *"isArray" {
                        let [argument] = call.args.as_slice() else {
                            return Err("`Array.isArray` expects exactly one argument".into());
                        };
                        if argument.spread.is_some() {
                            return Err("Array.isArray spread is not supported".into());
                        }
                        let value = self.lower_expr(&argument.expr)?;
                        let ty = self.infer_expr_type(&value)?;
                        if ty == HirType::Json {
                            return Ok(HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_json_is_array".to_string())),
                                vec![value],
                            ));
                        }
                        let name = format!("__thaw_is_array_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(name.clone(), ty.clone());
                        return self.wrap_call_argument_bindings(
                            HirExpr::Lit(HirLit::Bool(matches!(
                                ty,
                                HirType::Array(_) | HirType::Tuple(_)
                            ))),
                            &[(name, ty, value)],
                        );
                    }
                    if object.sym == *"Object" && property.sym == *"keys" {
                        let [argument] = call.args.as_slice() else {
                            return Err("`Object.keys` expects exactly one argument".into());
                        };
                        if argument.spread.is_some() {
                            return Err("Object.keys spread is not supported".into());
                        }
                        let value = self.lower_expr(&argument.expr)?;
                        let ty = self.infer_expr_type(&value)?;
                        let HirType::Object(fields) = &ty else {
                            return Err(format!(
                                "`Object.keys` currently requires a fixed object, got {ty:?}"
                            ));
                        };
                        let keys = HirExpr::ArrayLit(
                            fields
                                .iter()
                                .map(|(name, _)| HirExpr::Lit(HirLit::Str(name.clone())))
                                .collect(),
                        );
                        let name = format!("__thaw_object_keys_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(name.clone(), ty.clone());
                        return self.wrap_call_argument_bindings(keys, &[(name, ty, value)]);
                    }
                    if object.sym == *"Object" && property.sym == *"hasOwn" {
                        let [object, key] = call.args.as_slice() else {
                            return Err("`Object.hasOwn` expects exactly two arguments".into());
                        };
                        if object.spread.is_some() || key.spread.is_some() {
                            return Err("Object.hasOwn spread is not supported".into());
                        }
                        let object_value = self.lower_expr(&object.expr)?;
                        let object_type = self.infer_expr_type(&object_value)?;
                        let HirType::Object(fields) = &object_type else {
                            return Err(format!(
                                "`Object.hasOwn` currently requires a fixed object, got {object_type:?}"
                            ));
                        };
                        let field_names = fields
                            .iter()
                            .map(|(name, _)| name.clone())
                            .collect::<Vec<_>>();
                        let key_value = self.lower_expr(&key.expr)?;
                        let key_value = self.coerce_primitive_to_string(key_value)?;
                        let object_name = format!("__thaw_has_own_object_{}", self.next_binding);
                        self.next_binding += 1;
                        let key_name = format!("__thaw_has_own_key_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(object_name.clone(), object_type.clone());
                        self.scope.insert(key_name.clone(), HirType::Str);
                        let mut comparisons = field_names.into_iter().map(|field| {
                            HirExpr::BinOp(
                                BinOp::EqEqEq,
                                Box::new(HirExpr::Var(key_name.clone())),
                                Box::new(HirExpr::Lit(HirLit::Str(field))),
                            )
                        });
                        let mut result = comparisons
                            .next()
                            .unwrap_or(HirExpr::Lit(HirLit::Bool(false)));
                        for comparison in comparisons {
                            result = self.lower_logical_expr(result, comparison, false)?;
                        }
                        return self.wrap_call_argument_bindings(
                            result,
                            &[
                                (object_name, object_type, object_value),
                                (key_name, HirType::Str, key_value),
                            ],
                        );
                    }
                    if object.sym == *"Number"
                        && matches!(property.sym.as_ref(), "parseFloat" | "parseInt")
                    {
                        return self.lower_parse_call(call, property.sym == *"parseInt");
                    }
                    if object.sym == *"Math" && property.sym == *"random" {
                        if !call.args.is_empty() {
                            return Err("`Math.random` expects no arguments".into());
                        }
                        return Ok(HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_math_random".to_string())),
                            Vec::new(),
                        ));
                    }
                    if object.sym == *"Math"
                        && matches!(
                            property.sym.as_ref(),
                            "abs"
                                | "floor"
                                | "ceil"
                                | "trunc"
                                | "sqrt"
                                | "exp"
                                | "log"
                                | "log2"
                                | "log10"
                                | "sin"
                                | "cos"
                                | "tan"
                                | "asin"
                                | "acos"
                                | "atan"
                                | "sinh"
                                | "cosh"
                                | "tanh"
                                | "cbrt"
                                | "acosh"
                                | "asinh"
                                | "atanh"
                                | "expm1"
                                | "log1p"
                                | "fround"
                                | "clz32"
                        )
                    {
                        let [argument] = call.args.as_slice() else {
                            return Err(format!(
                                "`Math.{}` expects exactly one argument",
                                property.sym
                            ));
                        };
                        if argument.spread.is_some() {
                            return Err("Math function spread is not supported".into());
                        }
                        let value = self.lower_expr(&argument.expr)?;
                        let value = self.coerce_primitive_to_number(value)?;
                        return Ok(HirExpr::Call(
                            Box::new(HirExpr::Var(format!("__thaw_math_{}", property.sym))),
                            vec![value],
                        ));
                    }
                    if object.sym == *"Math"
                        && matches!(
                            property.sym.as_ref(),
                            "pow" | "min" | "max" | "sign" | "round" | "atan2" | "hypot" | "imul"
                        )
                    {
                        let expected = match property.sym.as_ref() {
                            "pow" | "atan2" | "imul" => Some(2),
                            "sign" | "round" => Some(1),
                            _ => None,
                        };
                        if expected.is_some_and(|expected| call.args.len() != expected) {
                            return Err(format!(
                                "`Math.{}` expects {} argument(s)",
                                property.sym,
                                expected.unwrap()
                            ));
                        }
                        let mut arguments = Vec::with_capacity(call.args.len());
                        for argument in &call.args {
                            if argument.spread.is_some() {
                                return Err("Math function spread is not supported".into());
                            }
                            let value = self.lower_expr(&argument.expr)?;
                            arguments.push(self.coerce_primitive_to_number(value)?);
                        }
                        return Ok(HirExpr::Call(
                            Box::new(HirExpr::Var(format!("__thaw_math_{}", property.sym))),
                            arguments,
                        ));
                    }
                    if object.sym == *"Number"
                        && matches!(
                            property.sym.as_ref(),
                            "isNaN" | "isFinite" | "isInteger" | "isSafeInteger"
                        )
                    {
                        let [argument] = call.args.as_slice() else {
                            return Err(format!(
                                "`Number.{}` expects exactly one argument",
                                property.sym
                            ));
                        };
                        if argument.spread.is_some() {
                            return Err("number predicate spread is not supported".into());
                        }
                        let value = self.lower_expr(&argument.expr)?;
                        let ty = self.infer_expr_type(&value)?;
                        if ty == HirType::F64 {
                            return Ok(HirExpr::Call(
                                Box::new(HirExpr::Var(
                                    match property.sym.as_ref() {
                                        "isNaN" => "__thaw_number_is_nan",
                                        "isFinite" => "__thaw_number_is_finite",
                                        "isInteger" => "__thaw_number_is_integer",
                                        "isSafeInteger" => "__thaw_number_is_safe_integer",
                                        _ => unreachable!(),
                                    }
                                    .to_string(),
                                )),
                                vec![value],
                            ));
                        }
                        let name = format!("__thaw_number_predicate_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(name.clone(), ty.clone());
                        return self.wrap_call_argument_bindings(
                            HirExpr::Lit(HirLit::Bool(false)),
                            &[(name, ty, value)],
                        );
                    }
                }
                if property.sym == *"charCodeAt" {
                    if call.args.len() > 1 {
                        return Err("native `.charCodeAt()` expects zero or one argument".into());
                    }
                    if call.args.iter().any(|argument| argument.spread.is_some()) {
                        return Err("charCodeAt spread is not supported".into());
                    }
                    let receiver = self.lower_expr(&member.obj)?;
                    self.expect_type(&HirType::Str, &receiver, "charCodeAt receiver")?;
                    let index = if let Some(argument) = call.args.first() {
                        let index = self.lower_expr(&argument.expr)?;
                        self.coerce_primitive_to_number(index)?
                    } else {
                        HirExpr::Lit(HirLit::F64(0.0))
                    };
                    let receiver_name = format!("__thaw_char_code_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    let index_name = format!("__thaw_char_code_index_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(receiver_name.clone(), HirType::Str);
                    self.scope.insert(index_name.clone(), HirType::F64);
                    let result = HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_string_char_code_at".to_string())),
                        vec![
                            HirExpr::Var(receiver_name.clone()),
                            HirExpr::Var(index_name.clone()),
                        ],
                    );
                    return self.wrap_call_argument_bindings(
                        result,
                        &[
                            (receiver_name, HirType::Str, receiver),
                            (index_name, HirType::F64, index),
                        ],
                    );
                }
                if property.sym == *"concat" {
                    if call.args.iter().any(|argument| argument.spread.is_some()) {
                        return Err("native concat spread is not supported".into());
                    }
                    let receiver = self.lower_expr(&member.obj)?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    if let HirType::Array(element) = receiver_type {
                        let element = element.as_ref().clone();
                        let mut parts = vec![receiver];
                        for argument in &call.args {
                            let value = self.lower_expr(&argument.expr)?;
                            let actual = self.infer_expr_type(&value)?;
                            if actual == HirType::Array(Box::new(element.clone())) {
                                parts.push(value);
                            } else if actual == element {
                                parts.push(HirExpr::ArrayLit(vec![value]));
                            } else {
                                return Err(format!(
                                    "array concat argument has type {actual:?}, expected {element:?} or an array of it"
                                ));
                            }
                        }
                        let mut bindings = Vec::with_capacity(parts.len());
                        let mut ordered = Vec::with_capacity(parts.len());
                        for (position, part) in parts.into_iter().enumerate() {
                            let ty = self.infer_expr_type(&part)?;
                            let name =
                                format!("__thaw_concat_part_{}_{}", position, self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(name.clone(), ty.clone());
                            bindings.push((name.clone(), ty, part));
                            ordered.push(HirExpr::Var(name));
                        }
                        return self.wrap_call_argument_bindings(
                            HirExpr::ArrayConcat(ordered, element),
                            &bindings,
                        );
                    }
                    self.expect_type(&HirType::Str, &receiver, "string concat receiver")?;
                    let mut sources = vec![receiver];
                    for argument in &call.args {
                        let value = self.lower_expr(&argument.expr)?;
                        let value = self.coerce_primitive_to_string(value)?;
                        sources.push(value);
                    }
                    let mut bindings = Vec::with_capacity(sources.len());
                    let mut values = Vec::with_capacity(sources.len());
                    for (position, source) in sources.into_iter().enumerate() {
                        let name = format!(
                            "__thaw_string_concat_part_{}_{}",
                            position, self.next_binding
                        );
                        self.next_binding += 1;
                        self.scope.insert(name.clone(), HirType::Str);
                        bindings.push((name.clone(), HirType::Str, source));
                        values.push(HirExpr::Var(name));
                    }
                    let mut values = values.into_iter();
                    let mut result = values.next().expect("concat always has a receiver");
                    for value in values {
                        result = HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_string_concat".to_string())),
                            vec![result, value],
                        );
                    }
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if matches!(property.sym.as_ref(), "trim" | "trimStart" | "trimEnd") {
                    if !call.args.is_empty() {
                        return Err(format!("native `.{}()` expects no arguments", property.sym));
                    }
                    let receiver = self.lower_expr(&member.obj)?;
                    self.expect_type(&HirType::Str, &receiver, "string trim receiver")?;
                    let suffix = match property.sym.as_ref() {
                        "trim" => "trim",
                        "trimStart" => "trim_start",
                        "trimEnd" => "trim_end",
                        _ => unreachable!(),
                    };
                    return Ok(HirExpr::Call(
                        Box::new(HirExpr::Var(format!("__thaw_string_{suffix}"))),
                        vec![receiver],
                    ));
                }
                if property.sym == *"toReversed" {
                    if !call.args.is_empty() {
                        return Err("native `.toReversed()` expects no arguments".into());
                    }
                    let receiver = self.lower_expr(&member.obj)?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    if !matches!(receiver_type, HirType::Array(_)) {
                        return Err(format!(
                            "`.toReversed()` requires a homogeneous array, got {receiver_type:?}"
                        ));
                    }
                    return Ok(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_array_to_reversed".to_string())),
                        vec![receiver],
                    ));
                }
                if property.sym == *"slice" {
                    if call.args.len() > 2 {
                        return Err("native `.slice()` expects zero to two arguments".into());
                    }
                    if call.args.iter().any(|argument| argument.spread.is_some()) {
                        return Err("array slice spread is not supported".into());
                    }
                    let receiver = self.lower_expr(&member.obj)?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    if !matches!(receiver_type, HirType::Array(_)) {
                        return Err(format!(
                            "`.slice()` requires a homogeneous array, got {receiver_type:?}"
                        ));
                    }
                    let mut indices = Vec::with_capacity(2);
                    for argument in &call.args {
                        let value = self.lower_expr(&argument.expr)?;
                        indices.push(self.coerce_primitive_to_number(value)?);
                    }
                    if indices.is_empty() {
                        indices.push(HirExpr::Lit(HirLit::F64(0.0)));
                    }
                    if indices.len() == 1 {
                        indices.push(HirExpr::Lit(HirLit::F64(f64::INFINITY)));
                    }
                    let receiver_name = format!("__thaw_slice_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope
                        .insert(receiver_name.clone(), receiver_type.clone());
                    let mut bindings = vec![(receiver_name.clone(), receiver_type, receiver)];
                    let mut arguments = vec![HirExpr::Var(receiver_name)];
                    for (position, index) in indices.into_iter().enumerate() {
                        let name = format!("__thaw_slice_index_{}_{}", position, self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(name.clone(), HirType::F64);
                        arguments.push(HirExpr::Var(name.clone()));
                        bindings.push((name, HirType::F64, index));
                    }
                    let result = HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_array_slice".to_string())),
                        arguments,
                    );
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if property.sym == *"copyWithin" {
                    if !(2..=3).contains(&call.args.len()) {
                        return Err("native `.copyWithin()` expects two or three arguments".into());
                    }
                    if call.args.iter().any(|argument| argument.spread.is_some()) {
                        return Err("copyWithin spread is not supported".into());
                    }
                    let receiver = self.lower_expr(&member.obj)?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    if !matches!(receiver_type, HirType::Array(_)) {
                        return Err(format!(
                            "`.copyWithin()` requires a homogeneous array, got {receiver_type:?}"
                        ));
                    }
                    let mut indices = Vec::with_capacity(3);
                    for argument in &call.args {
                        let value = self.lower_expr(&argument.expr)?;
                        indices.push(self.coerce_primitive_to_number(value)?);
                    }
                    if indices.len() == 2 {
                        indices.push(HirExpr::Lit(HirLit::F64(f64::INFINITY)));
                    }
                    let receiver_name = format!("__thaw_copy_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope
                        .insert(receiver_name.clone(), receiver_type.clone());
                    let mut bindings = vec![(receiver_name.clone(), receiver_type, receiver)];
                    let mut arguments = vec![HirExpr::Var(receiver_name)];
                    for (position, index) in indices.into_iter().enumerate() {
                        let name = format!("__thaw_copy_index_{}_{}", position, self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(name.clone(), HirType::F64);
                        arguments.push(HirExpr::Var(name.clone()));
                        bindings.push((name, HirType::F64, index));
                    }
                    let result = HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_array_copy_within".to_string())),
                        arguments,
                    );
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if property.sym == *"fill" {
                    if !(1..=3).contains(&call.args.len()) {
                        return Err("native `.fill()` expects one to three arguments".into());
                    }
                    if call.args.iter().any(|argument| argument.spread.is_some()) {
                        return Err("array fill spread is not supported".into());
                    }
                    let receiver = self.lower_expr(&member.obj)?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    let HirType::Array(element) = &receiver_type else {
                        return Err(format!(
                            "`.fill()` requires a homogeneous array, got {receiver_type:?}"
                        ));
                    };
                    let element = element.as_ref().clone();
                    let value = self.lower_expr(&call.args[0].expr)?;
                    self.expect_type(&element, &value, "fill value")?;
                    let mut indices = Vec::with_capacity(2);
                    for argument in &call.args[1..] {
                        let value = self.lower_expr(&argument.expr)?;
                        indices.push(self.coerce_primitive_to_number(value)?);
                    }
                    if indices.is_empty() {
                        indices.push(HirExpr::Lit(HirLit::F64(0.0)));
                    }
                    if indices.len() == 1 {
                        indices.push(HirExpr::Lit(HirLit::F64(f64::INFINITY)));
                    }
                    let runtime = match &element {
                        HirType::F64 => "__thaw_number_array_fill",
                        HirType::Bool => "__thaw_bool_array_fill",
                        _ => "__thaw_pointer_array_fill",
                    };
                    let receiver_name = format!("__thaw_fill_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope
                        .insert(receiver_name.clone(), receiver_type.clone());
                    let value_name = format!("__thaw_fill_value_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(value_name.clone(), element.clone());
                    let mut bindings = vec![
                        (receiver_name.clone(), receiver_type, receiver),
                        (value_name.clone(), element, value),
                    ];
                    let mut arguments = vec![HirExpr::Var(receiver_name), HirExpr::Var(value_name)];
                    for (position, index) in indices.into_iter().enumerate() {
                        let name = format!("__thaw_fill_index_{}_{}", position, self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(name.clone(), HirType::F64);
                        arguments.push(HirExpr::Var(name.clone()));
                        bindings.push((name, HirType::F64, index));
                    }
                    let result = HirExpr::Call(Box::new(HirExpr::Var(runtime.into())), arguments);
                    return self.wrap_call_argument_bindings(result, &bindings);
                }
                if property.sym == *"reverse" {
                    if !call.args.is_empty() {
                        return Err("native `.reverse()` expects no arguments".into());
                    }
                    let receiver = self.lower_expr(&member.obj)?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    if !matches!(receiver_type, HirType::Array(_)) {
                        return Err(format!(
                            "`.reverse()` requires a homogeneous array, got {receiver_type:?}"
                        ));
                    }
                    return Ok(HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_array_reverse".to_string())),
                        vec![receiver],
                    ));
                }
                if property.sym == *"join" {
                    if call.args.len() > 1 {
                        return Err("native `.join()` expects zero or one argument".into());
                    }
                    if call.args.iter().any(|argument| argument.spread.is_some()) {
                        return Err("array join spread is not supported".into());
                    }
                    let receiver = self.lower_expr(&member.obj)?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    let separator = if let Some(argument) = call.args.first() {
                        let value = self.lower_expr(&argument.expr)?;
                        self.coerce_primitive_to_string(value)?
                    } else {
                        HirExpr::Lit(HirLit::Str(",".to_string()))
                    };
                    return match receiver_type {
                        HirType::Array(element) => {
                            let array_type = HirType::Array(element.clone());
                            let builtin = match element.as_ref() {
                                HirType::F64 => "__thaw_number_array_join",
                                HirType::Str => "__thaw_string_array_join",
                                HirType::Bool => "__thaw_bool_array_join",
                                HirType::Object(_) => "__thaw_object_array_join",
                                other => {
                                    return Err(format!(
                                        "array join does not support element type {other:?}"
                                    ))
                                }
                            };
                            let receiver_name =
                                format!("__thaw_join_receiver_{}", self.next_binding);
                            self.next_binding += 1;
                            let separator_name =
                                format!("__thaw_join_separator_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(receiver_name.clone(), array_type.clone());
                            self.scope.insert(separator_name.clone(), HirType::Str);
                            let result = HirExpr::Call(
                                Box::new(HirExpr::Var(builtin.to_string())),
                                vec![
                                    HirExpr::Var(receiver_name.clone()),
                                    HirExpr::Var(separator_name.clone()),
                                ],
                            );
                            self.wrap_call_argument_bindings(
                                result,
                                &[
                                    (receiver_name, array_type, receiver),
                                    (separator_name, HirType::Str, separator),
                                ],
                            )
                        }
                        HirType::Tuple(elements) => self.join_tuple(receiver, elements, separator),
                        other => Err(format!(
                            "`.join()` requires an array receiver, got {other:?}"
                        )),
                    };
                }
                if matches!(
                    property.sym.as_ref(),
                    "indexOf" | "lastIndexOf" | "includes" | "startsWith" | "endsWith"
                ) {
                    if !(1..=2).contains(&call.args.len()) {
                        return Err(format!(
                            "native `.{}` expects one or two arguments",
                            property.sym
                        ));
                    }
                    if call.args.iter().any(|argument| argument.spread.is_some()) {
                        return Err("native search spread is not supported".into());
                    }
                    let receiver = self.lower_expr(&member.obj)?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    if receiver_type == HirType::Str {
                        let needle = self.lower_expr(&call.args[0].expr)?;
                        let needle = self.coerce_primitive_to_string(needle)?;
                        let position = if let Some(argument) = call.args.get(1) {
                            let value = self.lower_expr(&argument.expr)?;
                            self.coerce_primitive_to_number(value)?
                        } else if property.sym == *"endsWith" || property.sym == *"lastIndexOf" {
                            HirExpr::Lit(HirLit::F64(f64::INFINITY))
                        } else {
                            HirExpr::Lit(HirLit::F64(0.0))
                        };
                        let suffix = match property.sym.as_ref() {
                            "indexOf" => "index_of",
                            "lastIndexOf" => "last_index_of",
                            "includes" => "includes",
                            "startsWith" => "starts_with",
                            "endsWith" => "ends_with",
                            _ => unreachable!(),
                        };
                        let receiver_name =
                            format!("__thaw_string_search_receiver_{}", self.next_binding);
                        self.next_binding += 1;
                        let needle_name =
                            format!("__thaw_string_search_needle_{}", self.next_binding);
                        self.next_binding += 1;
                        let position_name =
                            format!("__thaw_string_search_position_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(receiver_name.clone(), HirType::Str);
                        self.scope.insert(needle_name.clone(), HirType::Str);
                        self.scope.insert(position_name.clone(), HirType::F64);
                        let result = HirExpr::Call(
                            Box::new(HirExpr::Var(format!("__thaw_string_{suffix}"))),
                            vec![
                                HirExpr::Var(receiver_name.clone()),
                                HirExpr::Var(needle_name.clone()),
                                HirExpr::Var(position_name.clone()),
                            ],
                        );
                        return self.wrap_call_argument_bindings(
                            result,
                            &[
                                (receiver_name, HirType::Str, receiver),
                                (needle_name, HirType::Str, needle),
                                (position_name, HirType::F64, position),
                            ],
                        );
                    }
                    if matches!(property.sym.as_ref(), "startsWith" | "endsWith") {
                        return Err(format!(
                            "`.{}` requires a string receiver, got {receiver_type:?}",
                            property.sym
                        ));
                    }
                    let HirType::Array(element) = receiver_type.clone() else {
                        return Err(format!(
                            "`.{}` requires a homogeneous array receiver, got {receiver_type:?}",
                            property.sym
                        ));
                    };
                    let needle = self.lower_expr(&call.args[0].expr)?;
                    let needle_type = self.infer_expr_type(&needle)?;
                    let from_index = if let Some(argument) = call.args.get(1) {
                        let value = self.lower_expr(&argument.expr)?;
                        self.coerce_primitive_to_number(value)?
                    } else if property.sym == *"lastIndexOf" {
                        HirExpr::Lit(HirLit::F64(f64::INFINITY))
                    } else {
                        HirExpr::Lit(HirLit::F64(0.0))
                    };
                    if needle_type != *element {
                        let receiver_name = format!("__thaw_search_array_{}", self.next_binding);
                        self.next_binding += 1;
                        let needle_name = format!("__thaw_search_needle_{}", self.next_binding);
                        self.next_binding += 1;
                        let start_name = format!("__thaw_search_start_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope
                            .insert(receiver_name.clone(), receiver_type.clone());
                        self.scope.insert(needle_name.clone(), needle_type.clone());
                        self.scope.insert(start_name.clone(), HirType::F64);
                        let result = if property.sym == *"includes" {
                            HirExpr::Lit(HirLit::Bool(false))
                        } else {
                            HirExpr::Lit(HirLit::F64(-1.0))
                        };
                        return self.wrap_call_argument_bindings(
                            result,
                            &[
                                (receiver_name, receiver_type, receiver),
                                (needle_name, needle_type, needle),
                                (start_name, HirType::F64, from_index),
                            ],
                        );
                    }
                    let prefix = match element.as_ref() {
                        HirType::F64 => "number",
                        HirType::Str => "string",
                        HirType::Bool => "bool",
                        HirType::Object(_) => "object",
                        other => {
                            return Err(format!(
                                "array search does not support element type {other:?}"
                            ))
                        }
                    };
                    let suffix = match property.sym.as_ref() {
                        "includes" => "includes",
                        "lastIndexOf" => "last_index_of",
                        _ => "index_of",
                    };
                    let receiver_name = format!("__thaw_search_array_{}", self.next_binding);
                    self.next_binding += 1;
                    let needle_name = format!("__thaw_search_needle_{}", self.next_binding);
                    self.next_binding += 1;
                    let start_name = format!("__thaw_search_start_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope
                        .insert(receiver_name.clone(), receiver_type.clone());
                    self.scope.insert(needle_name.clone(), needle_type.clone());
                    self.scope.insert(start_name.clone(), HirType::F64);
                    let result = HirExpr::Call(
                        Box::new(HirExpr::Var(format!("__thaw_{prefix}_array_{suffix}"))),
                        vec![
                            HirExpr::Var(receiver_name.clone()),
                            HirExpr::Var(needle_name.clone()),
                            HirExpr::Var(start_name.clone()),
                        ],
                    );
                    return self.wrap_call_argument_bindings(
                        result,
                        &[
                            (receiver_name, receiver_type, receiver),
                            (needle_name, needle_type, needle),
                            (start_name, HirType::F64, from_index),
                        ],
                    );
                }
                if property.sym == *"toString" {
                    if !call.args.is_empty() {
                        return Err("native `.toString()` does not accept arguments yet".into());
                    }
                    let receiver = self.lower_expr(&member.obj)?;
                    return self.coerce_primitive_to_string(receiver);
                }
                if property.sym == *"finally" {
                    let source = self.lower_expr(&member.obj)?;
                    let HirType::Promise(input) = self.infer_expr_type(&source)? else {
                        return Err("`.finally` requires a Promise receiver".into());
                    };
                    let [callback] = call.args.as_slice() else {
                        return Err("`.finally` expects exactly one callback".into());
                    };
                    if callback.spread.is_some() {
                        return Err("Promise callback spread is not supported".into());
                    }
                    let callback = self.lower_promise_callback(&callback.expr, &[], None)?;
                    let HirType::Function(_, callback_return) = self.infer_expr_type(&callback)?
                    else {
                        unreachable!()
                    };
                    return Ok(HirExpr::PromiseFinally(
                        Box::new(source),
                        Box::new(callback),
                        *input,
                        *callback_return,
                    ));
                }
                if property.sym == *"then" || property.sym == *"catch" {
                    let source = self.lower_expr(&member.obj)?;
                    let HirType::Promise(input) = self.infer_expr_type(&source)? else {
                        return Err(format!("`.{}` requires a Promise receiver", property.sym));
                    };
                    let [callback] = call.args.as_slice() else {
                        return Err(format!("`.{}` expects exactly one callback", property.sym));
                    };
                    if callback.spread.is_some() {
                        return Err("Promise callback spread is not supported".into());
                    }
                    let on_rejected = property.sym == *"catch";
                    let callback_input = if on_rejected {
                        HirType::Str
                    } else {
                        input.as_ref().clone()
                    };
                    let callback_params = if !on_rejected && callback_input == HirType::Void {
                        Vec::new()
                    } else {
                        vec![callback_input]
                    };
                    let callback =
                        self.lower_promise_callback(&callback.expr, &callback_params, None)?;
                    let HirType::Function(_, callback_output) = self.infer_expr_type(&callback)?
                    else {
                        unreachable!()
                    };
                    let (output, flatten) = match callback_output.as_ref() {
                        HirType::Promise(inner) => (inner.as_ref().clone(), true),
                        output => (output.clone(), false),
                    };
                    if on_rejected && output != *input {
                        return Err(format!(
                            "`.catch` callback resolves to {output:?}, expected {:?}",
                            input
                        ));
                    }
                    return Ok(HirExpr::PromiseThen(
                        Box::new(source),
                        Box::new(callback),
                        input.as_ref().clone(),
                        output,
                        on_rejected,
                        flatten,
                    ));
                }
            }
        }

        // An object field with a function type is a callable value. Preserve
        // it as `Call(PropAccess(...), args)` instead of flattening it into a
        // synthetic `object.method` global symbol (the latter is reserved for
        // builtins such as `console.log` and `JSON.parse`).
        if let Expr::Member(member) = callee_expr.as_ref() {
            if let (Expr::Ident(object), MemberProp::Ident(property)) =
                (member.obj.as_ref(), &member.prop)
            {
                let object_name = self.resolve_binding(object.sym.as_ref());
                let requested_property = property.sym.as_str();
                let resolved_property = match (requested_property, call.args.len()) {
                    ("listen", 2) => "__listenWithCallback",
                    ("close", 1) => "__closeWithCallback",
                    ("on", 2)
                        if matches!(
                            call.args.first().map(|arg| arg.expr.as_ref()),
                            Some(Expr::Lit(Lit::Str(event))) if event.value == *"error"
                        ) =>
                    {
                        "__onError"
                    }
                    _ => requested_property,
                };
                let object_ty = self.scope.get(&object_name).cloned();
                let callable = object_ty.as_ref().and_then(|ty| match ty {
                    HirType::Object(fields) => fields
                        .iter()
                        .find(|(name, _)| name == resolved_property)
                        .and_then(|(_, ty)| match ty {
                            HirType::Function(params, ret) => {
                                Some((params.clone(), ret.as_ref().clone()))
                            }
                            _ => None,
                        }),
                    _ => None,
                });
                if let Some((params, _)) = callable {
                    if params.len() != call.args.len() {
                        return Err(format!(
                            "method `{}.{}` expects {} argument(s), got {}",
                            object.sym,
                            property.sym,
                            params.len(),
                            call.args.len()
                        ));
                    }
                    let object_expr = self.lower_expr(&member.obj)?;
                    let callee = HirExpr::PropAccess(
                        Box::new(object_expr),
                        object_ty.unwrap(),
                        resolved_property.to_string(),
                    );
                    let args = call
                        .args
                        .iter()
                        .zip(&params)
                        .enumerate()
                        .map(|(index, (arg, expected))| {
                            if arg.spread.is_some() {
                                return Err("spread arguments are not supported".to_string());
                            }
                            let value = self.lower_expr(&arg.expr)?;
                            self.coerce_to_declared(expected, value).map_err(|error| {
                                format!(
                                    "argument {} of `{}.{}` is invalid: {error}",
                                    index + 1,
                                    object.sym,
                                    property.sym
                                )
                            })
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    return Ok(HirExpr::Call(Box::new(callee), args));
                }
            }
        }

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

        if matches!(callee_name.as_str(), "isNaN" | "isFinite") {
            let [argument] = call.args.as_slice() else {
                return Err(format!("`{callee_name}` expects exactly one argument"));
            };
            if argument.spread.is_some() {
                return Err("number predicate spread is not supported".into());
            }
            let value = self.lower_expr(&argument.expr)?;
            let value = self.coerce_primitive_to_number(value)?;
            return Ok(HirExpr::Call(
                Box::new(HirExpr::Var(
                    if callee_name == "isNaN" {
                        "__thaw_number_is_nan"
                    } else {
                        "__thaw_number_is_finite"
                    }
                    .to_string(),
                )),
                vec![value],
            ));
        }

        if callee_name == "Promise.all" {
            let [arg] = call.args.as_slice() else {
                return Err("`Promise.all` expects exactly one array argument".into());
            };
            if arg.spread.is_some() {
                return Err("spread arguments are not supported in `Promise.all`".into());
            }
            let Expr::Array(array) = arg.expr.as_ref() else {
                let (values, element) = self.lower_promise_array_value(&arg.expr, "all")?;
                return Ok(HirExpr::PromiseAllArray(Box::new(values), element));
            };
            if array
                .elems
                .iter()
                .flatten()
                .any(|element| element.spread.is_some())
            {
                let (values, element) = self.lower_promise_array_value(&arg.expr, "all")?;
                return Ok(HirExpr::PromiseAllArray(Box::new(values), element));
            }
            let mut element_types = Vec::new();
            let promises = array
                .elems
                .iter()
                .enumerate()
                .map(|(index, element)| {
                    let Some(element) = element else {
                        return Err(format!("Promise.all element {index} is missing"));
                    };
                    if element.spread.is_some() {
                        return Err("spread elements are not supported in `Promise.all`".into());
                    }
                    let value = self.lower_expr(&element.expr)?;
                    let resolved = match self.infer_expr_type(&value)? {
                        HirType::Promise(inner) => *inner,
                        returned
                            if matches!(&value, HirExpr::Call(callee, _)
                                if matches!(callee.as_ref(), HirExpr::Var(name)
                                    if self.signatures.get(name).is_some_and(|signature| signature.is_async))) => returned,
                        other => Err(format!(
                            "Promise.all element {index} must be a Promise, got {other:?}"
                        ))?,
                    };
                    if resolved == HirType::Void {
                        return Err(format!("Promise.all element {index} resolves to void"));
                    }
                    element_types.push(resolved);
                    Ok(value)
                })
                .collect::<Result<Vec<_>, String>>()?;
            let first = element_types.first().cloned().unwrap_or(HirType::F64);
            if element_types.iter().all(|element| element == &first) {
                return Ok(HirExpr::PromiseAll(promises, first));
            }
            return Ok(HirExpr::PromiseAllTuple(promises, element_types));
        }

        if callee_name == "Promise.allSettled" {
            let [arg] = call.args.as_slice() else {
                return Err("`Promise.allSettled` expects exactly one array argument".into());
            };
            if arg.spread.is_some() {
                return Err("spread arguments are not supported in `Promise.allSettled`".into());
            }
            let Expr::Array(array) = arg.expr.as_ref() else {
                let (values, element) = self.lower_promise_array_value(&arg.expr, "allSettled")?;
                return Ok(HirExpr::PromiseAllSettledArray(Box::new(values), element));
            };
            if array
                .elems
                .iter()
                .flatten()
                .any(|element| element.spread.is_some())
            {
                let (values, element) = self.lower_promise_array_value(&arg.expr, "allSettled")?;
                return Ok(HirExpr::PromiseAllSettledArray(Box::new(values), element));
            }
            let mut element_type = None;
            let promises = array
                .elems
                .iter()
                .enumerate()
                .map(|(index, element)| {
                    let Some(element) = element else {
                        return Err(format!("Promise.allSettled element {index} is missing"));
                    };
                    if element.spread.is_some() {
                        return Err(
                            "spread elements are not supported in `Promise.allSettled`".into(),
                        );
                    }
                    let value = self.lower_expr(&element.expr)?;
                    let resolved = match self.infer_expr_type(&value)? {
                        HirType::Promise(inner) => *inner,
                        returned
                            if matches!(&value, HirExpr::Call(callee, _)
                                if matches!(callee.as_ref(), HirExpr::Var(name)
                                    if self.signatures.get(name).is_some_and(|signature| signature.is_async))) => returned,
                        other => Err(format!(
                            "Promise.allSettled element {index} must be a Promise, got {other:?}"
                        ))?,
                    };
                    if resolved == HirType::Void {
                        return Err(format!(
                            "Promise.allSettled element {index} resolves to void"
                        ));
                    }
                    if let Some(expected) = &element_type {
                        if expected != &resolved {
                            return Err(format!(
                                "Promise.allSettled element {index} resolves to {resolved:?}, expected {expected:?}"
                            ));
                        }
                    } else {
                        element_type = Some(resolved);
                    }
                    Ok(value)
                })
                .collect::<Result<Vec<_>, String>>()?;
            return Ok(HirExpr::PromiseAllSettled(
                promises,
                element_type.unwrap_or(HirType::F64),
            ));
        }

        if callee_name == "Promise.race" {
            let [arg] = call.args.as_slice() else {
                return Err("`Promise.race` expects exactly one array argument".into());
            };
            if arg.spread.is_some() {
                return Err("spread arguments are not supported in `Promise.race`".into());
            }
            let Expr::Array(array) = arg.expr.as_ref() else {
                let (values, element) = self.lower_promise_array_value(&arg.expr, "race")?;
                return Ok(HirExpr::PromiseRaceArray(Box::new(values), element));
            };
            if array
                .elems
                .iter()
                .flatten()
                .any(|element| element.spread.is_some())
            {
                let (values, element) = self.lower_promise_array_value(&arg.expr, "race")?;
                return Ok(HirExpr::PromiseRaceArray(Box::new(values), element));
            }
            if array.elems.is_empty() {
                return Err("`Promise.race` requires at least one promise".into());
            }
            let mut element_type = None;
            let promises = array
                .elems
                .iter()
                .enumerate()
                .map(|(index, element)| {
                    let Some(element) = element else {
                        return Err(format!("Promise.race element {index} is missing"));
                    };
                    if element.spread.is_some() {
                        return Err("spread elements are not supported in `Promise.race`".into());
                    }
                    let value = self.lower_expr(&element.expr)?;
                    let resolved = match self.infer_expr_type(&value)? {
                        HirType::Promise(inner) => *inner,
                        returned
                            if matches!(&value, HirExpr::Call(callee, _)
                                if matches!(callee.as_ref(), HirExpr::Var(name)
                                    if self.signatures.get(name).is_some_and(|signature| signature.is_async))) => returned,
                        other => Err(format!(
                            "Promise.race element {index} must be a Promise, got {other:?}"
                        ))?,
                    };
                    if resolved == HirType::Void {
                        return Err(format!("Promise.race element {index} resolves to void"));
                    }
                    if let Some(expected) = &element_type {
                        if expected != &resolved {
                            return Err(format!(
                                "Promise.race element {index} resolves to {resolved:?}, expected {expected:?}"
                            ));
                        }
                    } else {
                        element_type = Some(resolved);
                    }
                    Ok(value)
                })
                .collect::<Result<Vec<_>, String>>()?;
            return Ok(HirExpr::PromiseRace(
                promises,
                element_type.expect("non-empty Promise.race"),
            ));
        }

        if callee_name == "Promise.any" {
            let [arg] = call.args.as_slice() else {
                return Err("`Promise.any` expects exactly one array argument".into());
            };
            if arg.spread.is_some() {
                return Err("spread arguments are not supported in `Promise.any`".into());
            }
            let Expr::Array(array) = arg.expr.as_ref() else {
                let (values, element) = self.lower_promise_array_value(&arg.expr, "any")?;
                return Ok(HirExpr::PromiseAnyArray(Box::new(values), element));
            };
            if array
                .elems
                .iter()
                .flatten()
                .any(|element| element.spread.is_some())
            {
                let (values, element) = self.lower_promise_array_value(&arg.expr, "any")?;
                return Ok(HirExpr::PromiseAnyArray(Box::new(values), element));
            }
            if array.elems.is_empty() {
                return Err("`Promise.any` requires at least one promise".into());
            }
            let mut element_type = None;
            let promises = array
                .elems
                .iter()
                .enumerate()
                .map(|(index, element)| {
                    let Some(element) = element else {
                        return Err(format!("Promise.any element {index} is missing"));
                    };
                    if element.spread.is_some() {
                        return Err("spread elements are not supported in `Promise.any`".into());
                    }
                    let value = self.lower_expr(&element.expr)?;
                    let resolved = match self.infer_expr_type(&value)? {
                        HirType::Promise(inner) => *inner,
                        returned
                            if matches!(&value, HirExpr::Call(callee, _)
                                if matches!(callee.as_ref(), HirExpr::Var(name)
                                    if self.signatures.get(name).is_some_and(|signature| signature.is_async))) => returned,
                        other => Err(format!(
                            "Promise.any element {index} must be a Promise, got {other:?}"
                        ))?,
                    };
                    if resolved == HirType::Void {
                        return Err(format!("Promise.any element {index} resolves to void"));
                    }
                    if let Some(expected) = &element_type {
                        if expected != &resolved {
                            return Err(format!(
                                "Promise.any element {index} resolves to {resolved:?}, expected {expected:?}"
                            ));
                        }
                    } else {
                        element_type = Some(resolved);
                    }
                    Ok(value)
                })
                .collect::<Result<Vec<_>, String>>()?;
            return Ok(HirExpr::PromiseAny(
                promises,
                element_type.expect("non-empty Promise.any"),
            ));
        }

        // `Number`/`String`/`Boolean` convert a `Json` leaf to a concrete
        // value. Unlike `console.log` (whose codegen can disambiguate its
        // argument by LLVM value shape -- f64 vs. pointer), `Str`/`Array`/
        if matches!(callee_name.as_str(), "parseFloat" | "parseInt") {
            return self.lower_parse_call(call, callee_name == "parseInt");
        }

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
            if callee_name == "String" && ty == HirType::Str {
                return Ok(value);
            }
            if callee_name == "String" && ty == HirType::Bool {
                return Ok(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_bool_to_string".to_string())),
                    vec![value],
                ));
            }
            if callee_name == "String" && ty == HirType::F64 {
                return Ok(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_number_to_string".to_string())),
                    vec![value],
                ));
            }
            if callee_name == "String"
                && matches!(
                    ty,
                    HirType::Array(_) | HirType::Tuple(_) | HirType::Object(_)
                )
            {
                return self.coerce_primitive_to_string(value);
            }
            if callee_name == "Boolean" && ty != HirType::Json {
                let name = format!("__thaw_boolean_value_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(name.clone(), ty.clone());
                let converted = self.truthiness_expr(HirExpr::Var(name.clone()), &ty)?;
                return self.wrap_call_argument_bindings(converted, &[(name, ty, value)]);
            }
            if callee_name == "Number" && ty == HirType::F64 {
                return Ok(value);
            }
            if callee_name == "Number" && ty == HirType::Bool {
                return Ok(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_bool_to_number".to_string())),
                    vec![value],
                ));
            }
            if callee_name == "Number" && ty == HirType::Str {
                return Ok(HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_string_to_number".to_string())),
                    vec![value],
                ));
            }
            if callee_name == "Number"
                && matches!(
                    ty,
                    HirType::Array(_) | HirType::Tuple(_) | HirType::Object(_)
                )
            {
                return self.coerce_primitive_to_number(value);
            }
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

        let mut argument_bindings = Vec::new();
        let mut lowered_arguments = Vec::new();
        let lowered = call
            .args
            .iter()
            .map(|arg| self.lower_expr(&arg.expr))
            .collect::<Result<Vec<_>, _>>()?;
        let preserve_argument_order =
            call.args.iter().any(|arg| arg.spread.is_some()) || lowered.iter().any(contains_await);
        for (arg, value) in call.args.iter().zip(lowered) {
            if !preserve_argument_order {
                lowered_arguments.push(value);
                continue;
            }
            if arg.spread.is_none() {
                let ty = self.infer_expr_type(&value)?;
                let name = format!("__thaw_call_arg_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(name.clone(), ty.clone());
                argument_bindings.push((name.clone(), ty, value));
                lowered_arguments.push(HirExpr::Var(name));
                continue;
            }

            if let HirExpr::ArrayLit(values) = value {
                for value in values {
                    let ty = self.infer_expr_type(&value)?;
                    let name = format!("__thaw_call_arg_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(name.clone(), ty.clone());
                    argument_bindings.push((name.clone(), ty, value));
                    lowered_arguments.push(HirExpr::Var(name));
                }
                continue;
            }

            let source_type = self.infer_expr_type(&value)?;
            let HirType::Tuple(elements) = &source_type else {
                return Err(format!(
                    "call spread source must have statically known tuple length, got {source_type:?}"
                ));
            };
            let elements = elements.clone();
            let name = format!("__thaw_call_spread_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(name.clone(), source_type.clone());
            argument_bindings.push((name.clone(), source_type, value));
            lowered_arguments.extend(elements.into_iter().enumerate().map(|(index, element)| {
                HirExpr::TypedIndex(
                    Box::new(HirExpr::Var(name.clone())),
                    Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                    element,
                )
            }));
        }

        if let Some(params) = &param_types {
            if lowered_arguments.len() != params.len() {
                return Err(format!(
                    "function `{callee_name}` expects {} argument(s), got {}",
                    params.len(),
                    lowered_arguments.len()
                ));
            }
        }

        let args = lowered_arguments
            .into_iter()
            .enumerate()
            .map(
                |(i, value)| match param_types.as_ref().and_then(|p| p.get(i)) {
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
                },
            )
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
            if !types.contains(&HirType::Dynamic) {
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
                let result = HirExpr::DynamicCall(
                    DynamicSignature {
                        backend,
                        symbol,
                        params: sig.params,
                        ret: sig.ret,
                    },
                    args,
                );
                return self.wrap_call_argument_bindings(result, &argument_bindings);
            }
            let param_count = sig.params.len();
            let ffi_signature = FfiSignature {
                symbol: callee_name,
                params: sig.params,
                ret: sig.ret,
                error_abi: FfiErrorAbi::Direct,
                return_ownership: FfiOwnership::Borrowed,
                error_ownership: FfiOwnership::Borrowed,
                param_string_abis: vec![FfiStringAbi::NullTerminated; param_count],
                return_string_abi: FfiStringAbi::NullTerminated,
                calling_convention: FfiCallingConvention::C,
                aggregate_return_abi: FfiAggregateAbi::Internal,
            };
            let result = HirExpr::FfiCall(ffi_signature, args);
            return self.wrap_call_argument_bindings(result, &argument_bindings);
        }

        let lowered_name = if generic_types
            .as_ref()
            .is_some_and(|types| !types.contains(&HirType::Dynamic))
        {
            let param_types = args
                .iter()
                .map(|arg| self.infer_expr_type(arg))
                .collect::<Result<Vec<_>, _>>()?;
            specialized_generic_name(&callee_name, &param_types)
        } else {
            callee_name
        };
        let result = HirExpr::Call(Box::new(HirExpr::Var(lowered_name)), args);
        self.wrap_call_argument_bindings(result, &argument_bindings)
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
    fn infers_void_for_expression_bodied_console_log_arrow() {
        let program = lower(
            r#"async function main(): Promise<void> {
                await new Promise<void>((resolve, reject) => resolve())
                    .finally(() => console.log("cleanup"));
            }"#,
        );
        let HirStmt::Expr(HirExpr::Await(inner)) = &program.functions[0].body[0] else {
            panic!("expected awaited finally chain");
        };
        let HirExpr::PromiseFinally(_, callback, HirType::Void, HirType::Void) = inner.as_ref()
        else {
            panic!("expected void finally callback");
        };
        assert!(matches!(
            callback.as_ref(),
            HirExpr::Lambda(_, _, HirType::Void, _)
        ));
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
    fn lowers_typed_unary_and_extended_comparison_operators() {
        let program = lower(
            r#"function main(): void {
                console.log(-1);
                console.log(+2);
                console.log(!false);
                console.log(1 <= 2);
                console.log(2 >= 2);
                console.log("a" !== "b");
            }"#,
        );
        assert_eq!(program.functions[0].body.len(), 6);
        for statement in &program.functions[0].body {
            let HirStmt::Expr(HirExpr::Call(_, arguments)) = statement else {
                panic!("expected console call");
            };
            assert_eq!(arguments.len(), 1);
            assert!(matches!(
                arguments[0],
                HirExpr::Call(..) | HirExpr::BinOp(..) | HirExpr::Lit(..)
            ));
        }
    }

    #[test]
    fn lowers_remainder_exponentiation_and_compound_assignments() {
        let program = lower(
            r#"function main(): void {
                let value = 10;
                console.log(value % 3);
                console.log(2 ** 3);
                value %= 4;
                value **= 3;
                console.log(value);
            }"#,
        );
        assert!(matches!(
            &program.functions[0].body[1],
            HirStmt::Expr(HirExpr::Call(_, args))
                if matches!(&args[0], HirExpr::BinOp(BinOp::Mod, _, _))
        ));
        assert!(matches!(
            &program.functions[0].body[2],
            HirStmt::Expr(HirExpr::Call(_, args))
                if matches!(&args[0], HirExpr::BinOp(BinOp::Exp, _, _))
        ));
        assert!(matches!(
            &program.functions[0].body[3],
            HirStmt::Expr(HirExpr::Assign(_, value))
                if matches!(value.as_ref(), HirExpr::BinOp(BinOp::Mod, _, _))
        ));
        assert!(matches!(
            &program.functions[0].body[4],
            HirStmt::Expr(HirExpr::Assign(_, value))
                if matches!(value.as_ref(), HirExpr::BinOp(BinOp::Exp, _, _))
        ));
    }

    #[test]
    fn lowers_bitwise_and_shift_operators() {
        let program = lower(
            r#"function main(): void {
                let value = 5;
                console.log(value | 2);
                console.log(value ^ 1);
                console.log(value & 3);
                value <<= 2;
                value >>= 1;
                value >>>= 1;
            }"#,
        );
        let expected = [BinOp::BitOr, BinOp::BitXor, BinOp::BitAnd];
        for (statement, expected) in program.functions[0].body[1..4].iter().zip(expected) {
            assert!(matches!(
                statement,
                HirStmt::Expr(HirExpr::Call(_, args))
                    if matches!(&args[0], HirExpr::BinOp(op, _, _) if *op == expected)
            ));
        }
        let expected = [BinOp::LShift, BinOp::RShift, BinOp::ZeroFillRShift];
        for (statement, expected) in program.functions[0].body[4..7].iter().zip(expected) {
            assert!(matches!(
                statement,
                HirStmt::Expr(HirExpr::Assign(_, value))
                    if matches!(value.as_ref(), HirExpr::BinOp(op, _, _) if *op == expected)
            ));
        }
    }

    #[test]
    fn lowers_bitwise_not() {
        let program = lower(
            r#"function main(): void {
                console.log(~5);
            }"#,
        );
        let main = program
            .functions
            .iter()
            .find(|function| function.name == "main")
            .unwrap();
        assert!(matches!(
            &main.body[0],
            HirStmt::Expr(HirExpr::Call(_, args))
                if matches!(&args[0], HirExpr::BinOp(BinOp::BitXor, _, rhs)
                    if matches!(rhs.as_ref(), HirExpr::Lit(HirLit::F64(value)) if *value == -1.0))
        ));
    }

    #[test]
    fn lowers_typeof_to_an_evaluating_typed_closure() {
        let program = lower(
            r#"function value(): number { return 1; }
            function callback(value: number): number { return value; }
            function main(): void {
                console.log(typeof value());
                console.log(typeof callback);
            }"#,
        );
        let main = program
            .functions
            .iter()
            .find(|function| function.name == "main")
            .unwrap();
        assert!(matches!(
            &main.body[0],
            HirStmt::Expr(HirExpr::Call(_, args))
                if matches!(&args[0], HirExpr::Call(lambda, values)
                    if matches!(lambda.as_ref(), HirExpr::Lambda(_, params, HirType::Str, body)
                        if params.len() == 1
                            && matches!(body.as_ref(), HirExpr::Lit(HirLit::Str(value)) if value == "number"))
                        && matches!(values.as_slice(), [HirExpr::Call(_, _)]))
        ));
        assert!(matches!(
            &main.body[1],
            HirStmt::Expr(HirExpr::Call(_, args))
                if matches!(&args[0], HirExpr::Lit(HirLit::Str(value)) if value == "function")
        ));
    }

    #[test]
    fn distinguishes_prefix_and_postfix_update_values() {
        let program = lower(
            r#"function main(): void {
                let value = 1;
                const old = value++;
                const current = ++value;
                let values = [4];
                const element = values[0]--;
            }"#,
        );
        let main = &program.functions[0];
        assert!(matches!(
            &main.body[1],
            HirStmt::Let(_, HirType::F64, HirExpr::Call(_, _))
        ));
        assert!(matches!(
            &main.body[2],
            HirStmt::Let(_, HirType::F64, HirExpr::Assign(_, value))
                if matches!(value.as_ref(), HirExpr::BinOp(BinOp::Add, _, _))
        ));
        assert!(matches!(
            &main.body[4],
            HirStmt::Let(_, HirType::F64, HirExpr::Call(_, _))
        ));
    }

    #[test]
    fn lowers_number_field_update_expressions() {
        let program = lower(
            r#"function main(): void {
                let point = { value: 2 };
                const old = point.value++;
                const current = --point.value;
            }"#,
        );
        assert!(matches!(
            &program.functions[0].body[1],
            HirStmt::Let(_, HirType::F64, HirExpr::Call(_, _))
        ));
        assert!(matches!(
            &program.functions[0].body[2],
            HirStmt::Let(_, HirType::F64, HirExpr::PropAssign(_, _, field, _)) if field == "value"
        ));
    }

    #[test]
    fn binds_compound_assignment_references_once() {
        let program = lower(
            r#"function values(): number[] { return [1]; }
            function index(): number { return 0; }
            function point(): { value: number } { return { value: 1 }; }
            function main(): void {
                values()[index()] += 2;
                point().value *= 3;
            }"#,
        );
        let main = program
            .functions
            .iter()
            .find(|function| function.name == "main")
            .unwrap();
        assert!(main
            .body
            .iter()
            .all(|statement| matches!(statement, HirStmt::Expr(HirExpr::Call(_, _)))));
    }

    #[test]
    fn lowers_static_computed_object_reads_and_targets() {
        let program = lower(
            r#"function main(): void {
                let point = { value: 1 };
                console.log(point["value"]);
                point["value"] = 2;
                point["value"] += 3;
                point["value"]++;
            }"#,
        );
        let body = &program.functions[0].body;
        assert!(matches!(
            &body[1],
            HirStmt::Expr(HirExpr::Call(_, args))
                if matches!(&args[0], HirExpr::PropAccess(_, _, field) if field == "value")
        ));
        assert!(matches!(
            &body[2],
            HirStmt::Expr(HirExpr::PropAssign(_, _, field, _)) if field == "value"
        ));
        assert!(matches!(&body[3], HirStmt::Expr(HirExpr::Call(_, _))));
        assert!(matches!(&body[4], HirStmt::Expr(HirExpr::Call(_, _))));
    }

    #[test]
    fn lowers_nested_object_and_tuple_destructuring_once() {
        let program = lower(
            r#"function source(): { x: number; label: string; nested: { flag: boolean }; extra: number } {
                return { x: 1, label: "ok", nested: { flag: true }, extra: 4 };
            }
            function main(): void {
                const { x: renamed, nested: { flag }, ...rest } = source();
                const [first, , pair, ...tail]: [number, string, { value: number }, number] =
                    [1, "skip", { value: 3 }, 4];
            }"#,
        );
        let main = program
            .functions
            .iter()
            .find(|function| function.name == "main")
            .unwrap();
        assert!(matches!(
            &main.body[0],
            HirStmt::Let(name, HirType::Object(_), HirExpr::Call(_, _))
                if name.starts_with("__thaw_destructure_")
        ));
        assert!(main.body.iter().any(|statement| matches!(
            statement,
            HirStmt::Let(name, HirType::Bool, _) if name == "flag"
        )));
        assert!(main.body.iter().any(|statement| matches!(
            statement,
            HirStmt::Let(name, HirType::Array(element), _) if name == "tail" && element.as_ref() == &HirType::F64
        )));
    }

    #[test]
    fn lowers_fixed_layout_destructuring_assignments() {
        let program = lower(
            r#"function source(): { x: number; label: string } {
                return { x: 1, label: "ok" };
            }
            function main(): void {
                let x = 0;
                let label = "";
                const returned = ({ x, label } = source());
                let first = 0;
                let tail = [0, 0];
                [first, ...tail] = [2, 3, 4];
            }"#,
        );
        let main = program
            .functions
            .iter()
            .find(|function| function.name == "main")
            .unwrap();
        assert!(matches!(
            &main.body[2],
            HirStmt::Let(_, HirType::Object(_), HirExpr::Call(_, _))
        ));
        assert!(matches!(
            main.body.last(),
            Some(HirStmt::Expr(HirExpr::Call(_, _)))
        ));
    }

    #[test]
    fn lowers_for_of_destructuring_bindings_and_assignment_heads() {
        let program = lower(
            r#"function main(): void {
                const rows = [{ x: 1, label: "a" }];
                for (const { x, label } of rows) { console.log(x); }
                let assigned = 0;
                for ({ x: assigned } of rows) { console.log(assigned); }
                const pairs: [number, string][] = [[2, "b"]];
                for (const [value, text] of pairs) { console.log(text); }
            }"#,
        );
        let loops = program.functions[0]
            .body
            .iter()
            .filter_map(|statement| match statement {
                HirStmt::While(_, body) => Some(body),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(loops.len(), 3);
        assert!(loops.iter().all(|body| matches!(
            body.first(),
            Some(HirStmt::Let(name, _, _)) if name.starts_with("__thaw_for_of_item_")
        )));
    }

    #[test]
    fn lowers_function_and_arrow_parameter_destructuring() {
        let program = lower(
            r#"function read(
                { x, nested: { flag }, ...rest }:
                    { x: number; nested: { flag: boolean }; label: string },
                [first, ...tail]: [number, number, number]
            ): number { return x + first + tail[0]; }
            function main(): void {
                const pick = ({ value }: { value: number }): number => value;
                console.log(read(
                    { x: 1, nested: { flag: true }, label: "ok" }, [2, 3, 4]
                ));
                console.log(pick({ value: 5 }));
            }"#,
        );
        let read = program
            .functions
            .iter()
            .find(|function| function.name == "read")
            .unwrap();
        assert!(read
            .params
            .iter()
            .all(|param| param.name.starts_with("__thaw_param_")));
        assert!(read.body.iter().any(|statement| matches!(
            statement,
            HirStmt::Let(name, HirType::Bool, _) if name == "flag"
        )));
    }

    #[test]
    fn lowers_optional_chains_on_statically_non_null_values() {
        let program = lower(
            r#"function invoke(callback: (value: number) => number): number {
                return callback?.(2);
            }
            function main(): void {
                const box = { value: 1 };
                console.log(box?.value);
                console.log(box?.["value"]);
            }"#,
        );
        let invoke = program
            .functions
            .iter()
            .find(|function| function.name == "invoke")
            .unwrap();
        assert!(matches!(
            &invoke.body[0],
            HirStmt::Return(Some(HirExpr::Call(_, _)))
        ));
    }

    #[test]
    fn lowers_fixed_object_in_checks_with_operand_evaluation() {
        let program = lower(
            r#"function object(): { value: number } { return { value: 1 }; }
            function main(): void {
                console.log("value" in object());
                console.log("missing" in object());
            }"#,
        );
        let main = program
            .functions
            .iter()
            .find(|function| function.name == "main")
            .unwrap();
        assert!(main.body.iter().all(|statement| matches!(
            statement,
            HirStmt::Expr(HirExpr::Call(_, args))
                if matches!(&args[0], HirExpr::Call(_, _))
        )));
    }

    #[test]
    fn lowers_sequence_expressions_to_ordered_closures() {
        let program = lower(
            r#"function effect(value: number): number { return value; }
            function main(): void {
                const result = (effect(1), effect(2), 3);
            }"#,
        );
        let main = program
            .functions
            .iter()
            .find(|function| function.name == "main")
            .unwrap();
        assert!(matches!(
            &main.body[0],
            HirStmt::Let(_, HirType::F64, HirExpr::Call(lambda, args))
                if args.is_empty()
                    && matches!(lambda.as_ref(), HirExpr::Lambda(_, _, HirType::F64, body)
                        if matches!(body.as_ref(), HirExpr::Block(statements) if statements.len() == 3))
        ));
    }

    #[test]
    fn lowers_void_to_an_evaluating_closure() {
        let program = lower(
            r#"function effect(): number { return 1; }
            function main(): void { void effect(); }"#,
        );
        let main = program
            .functions
            .iter()
            .find(|function| function.name == "main")
            .unwrap();
        assert!(matches!(
            &main.body[0],
            HirStmt::Expr(HirExpr::Call(lambda, args))
                if args.is_empty()
                    && matches!(lambda.as_ref(), HirExpr::Lambda(_, params, HirType::Void, body)
                        if params.is_empty() && matches!(body.as_ref(), HirExpr::Block(_)))
        ));
    }

    #[test]
    fn lowers_same_type_loose_equality() {
        let program = lower(
            r#"function main(): void {
                console.log(1 == 1);
                console.log("a" != "b");
                console.log(true == false);
            }"#,
        );
        assert_eq!(program.functions[0].body.len(), 3);
        for statement in &program.functions[0].body {
            assert!(matches!(
                statement,
                HirStmt::Expr(HirExpr::Call(_, args))
                    if matches!(&args[0], HirExpr::BinOp(BinOp::EqEqEq, _, _))
            ));
        }
    }

    #[test]
    fn lowers_static_and_tuple_call_argument_spreads() {
        let program = lower(
            r#"function emit(first: number, second: string, third: number): void {}
            function makeArgs(): [string, number] { return ["two", 3]; }
            function main(): void {
                emit(...[1, "two", 3]);
                emit(1, ...makeArgs());
            }"#,
        );
        let main = program
            .functions
            .iter()
            .find(|function| function.name == "main")
            .unwrap();
        assert!(matches!(
            &main.body[0],
            HirStmt::Expr(HirExpr::Call(lambda, _))
                if matches!(lambda.as_ref(), HirExpr::Lambda(_, _, HirType::Void, _))
        ));
        let HirStmt::Expr(HirExpr::Call(_, first_arguments)) = &main.body[1] else {
            panic!("expected bound leading argument");
        };
        assert!(matches!(
            first_arguments.as_slice(),
            [HirExpr::Lit(HirLit::F64(1.0))]
        ));
    }

    #[test]
    fn rejects_dynamic_length_call_spread() {
        let module = thaw_parser::parse_typescript(
            r#"function emit(first: number): void {}
            function main(values: number[]): void { emit(...values); }"#,
        )
        .unwrap();
        let error = lower_module(&module).unwrap_err();
        assert!(error.contains("statically known tuple length"), "{error}");
    }

    #[test]
    fn lowers_cross_type_primitive_loose_equality() {
        let program = lower(
            r#"function main(): void {
                console.log(1 == "1");
                console.log(false != "1");
            }"#,
        );
        assert_eq!(program.functions[0].body.len(), 2);
    }

    #[test]
    fn lowers_boolean_logical_operators_to_short_circuit_closures() {
        let program = lower(
            r#"function main(): void {
                const a = true;
                const b = false;
                console.log(a && b);
                console.log(a || b);
            }"#,
        );
        for statement in &program.functions[0].body[2..] {
            let HirStmt::Expr(HirExpr::Call(_, arguments)) = statement else {
                panic!("expected console call");
            };
            let HirExpr::Call(callee, call_arguments) = &arguments[0] else {
                panic!("expected immediately invoked logical closure");
            };
            assert_eq!(call_arguments.len(), 1);
            assert!(matches!(
                callee.as_ref(),
                HirExpr::Lambda(_, params, HirType::Bool, _) if params.len() == 1
            ));
        }
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
            [HirStmt::Expr(HirExpr::Call(_, _)), HirStmt::Continue]
        ));
        assert!(matches!(
            body.last(),
            Some(HirStmt::Expr(HirExpr::Call(_, _)))
        ));
    }

    #[test]
    fn desugars_do_while_and_checks_condition_before_continue() {
        let program = lower(
            r#"function main(): void {
                let i = 0;
                do {
                    i++;
                    if (i < 2) continue;
                    console.log(i);
                } while (i < 3);
            }"#,
        );
        let HirStmt::While(HirExpr::Lit(HirLit::Bool(true)), body) = &program.functions[0].body[1]
        else {
            panic!("expected unconditional desugared loop");
        };
        let guard_count = body
            .iter()
            .filter(|stmt| matches!(stmt, HirStmt::If(_, _, else_body) if else_body == &[HirStmt::Break]))
            .count();
        assert_eq!(guard_count, 1, "expected the ordinary tail guard");
        let HirStmt::If(_, continue_body, _) = &body[1] else {
            panic!("expected source if statement");
        };
        assert!(matches!(
            continue_body.as_slice(),
            [HirStmt::If(_, _, else_body), HirStmt::Continue]
                if else_body == &[HirStmt::Break]
        ));
    }

    #[test]
    fn desugars_for_of_to_single_evaluation_index_loop() {
        let program = lower(
            r#"function values(): number[] { return [1, 2, 3]; }
               function main(): void {
                   for (const value of values()) { console.log(value); }
               }"#,
        );
        let body = &program.functions[1].body;
        assert_eq!(body.len(), 3);
        assert!(matches!(
            &body[0],
            HirStmt::Let(_, HirType::Array(element), HirExpr::Call(_, _))
                if element.as_ref() == &HirType::F64
        ));
        let HirStmt::While(_, loop_body) = &body[2] else {
            panic!("expected indexed while loop");
        };
        assert!(matches!(
            &loop_body[0],
            HirStmt::Let(_, HirType::F64, HirExpr::TypedIndex(_, _, HirType::F64))
        ));
    }

    #[test]
    fn desugars_for_of_assignment_to_existing_variable() {
        let program = lower(
            r#"function main(): void {
                let value = 0;
                for (value of [1, 2]) { console.log(value); }
                console.log(value);
            }"#,
        );
        let HirStmt::While(_, loop_body) = &program.functions[0].body[3] else {
            panic!("expected indexed while loop");
        };
        assert!(matches!(
            &loop_body[0],
            HirStmt::Expr(HirExpr::Assign(name, value))
                if name == "value" && matches!(value.as_ref(), HirExpr::TypedIndex(_, _, HirType::F64))
        ));
    }

    #[test]
    fn desugars_for_await_of_promise_array_to_awaited_items() {
        let program = lower(
            r#"async function main(): Promise<void> {
                const values: Promise<number>[] = [
                    new Promise<number>((resolve, reject) => resolve(1))
                ];
                for await (const value of values) { console.log(value); }
            }"#,
        );
        let HirStmt::While(_, loop_body) = &program.functions[0].body[3] else {
            panic!("expected indexed while loop");
        };
        assert!(matches!(
            &loop_body[0],
            HirStmt::Let(
                _,
                HirType::F64,
                HirExpr::AwaitPromise(indexed, HirType::F64)
            ) if matches!(indexed.as_ref(), HirExpr::TypedIndex(_, _, HirType::Promise(inner)) if inner.as_ref() == &HirType::F64)
        ));
    }

    #[test]
    fn lowers_switch_to_selected_case_state_without_switch_breaks() {
        fn contains_break(stmts: &[HirStmt]) -> bool {
            stmts.iter().any(|stmt| match stmt {
                HirStmt::Break => true,
                HirStmt::If(_, then_body, else_body) => {
                    contains_break(then_body) || contains_break(else_body)
                }
                HirStmt::Try(body, _, catch_body) => {
                    contains_break(body) || contains_break(catch_body)
                }
                HirStmt::While(_, _) => false,
                _ => false,
            })
        }
        let program = lower(
            r#"function main(): void {
                switch (2) {
                    case 1: console.log("one"); break;
                    default: console.log("default");
                    case 2: console.log("two"); break;
                }
            }"#,
        );
        assert!(program.functions[0].body.len() > 4);
        assert!(!contains_break(&program.functions[0].body));
        assert!(matches!(
            &program.functions[0].body[0],
            HirStmt::Let(name, HirType::F64, HirExpr::Lit(HirLit::F64(2.0)))
                if name.starts_with("__thaw_switch_value_")
        ));
    }

    #[test]
    fn lowers_fixed_object_for_in_to_key_array_loop() {
        let program = lower(
            r#"function main(): void {
                const object = { first: 1, second: 2 };
                for (const key in object) { console.log(key); }
            }"#,
        );
        let body = &program.functions[0].body;
        assert_eq!(body.len(), 5);
        assert!(matches!(
            &body[2],
            HirStmt::Let(_, HirType::Array(element), HirExpr::ArrayLit(keys))
                if element.as_ref() == &HirType::Str
                    && keys == &[
                        HirExpr::Lit(HirLit::Str("first".into())),
                        HirExpr::Lit(HirLit::Str("second".into()))
                    ]
        ));
        assert!(matches!(&body[4], HirStmt::While(_, _)));
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
                vec![HirExpr::TypedIndex(
                    Box::new(HirExpr::Var("xs".into())),
                    Box::new(HirExpr::Lit(HirLit::F64(1.0))),
                    HirType::F64,
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
    fn lowers_typed_array_spreads_in_source_order() {
        let program = lower(
            r#"function part(): number[] { return [2, 3]; }
               function main(): void {
                   const tail: number[] = [4, 5];
                   const values: number[] = [1, ...part(), ...tail, 6];
                   console.log(values.length);
               }"#,
        );
        let HirStmt::Let(_, HirType::Array(element), HirExpr::ArrayConcat(parts, spread_element)) =
            &program.functions[1].body[1]
        else {
            panic!("expected typed array concat");
        };
        assert_eq!(element.as_ref(), &HirType::F64);
        assert_eq!(spread_element, &HirType::F64);
        assert_eq!(parts.len(), 4);
        assert!(matches!(&parts[0], HirExpr::ArrayLit(values) if values.len() == 1));
        assert!(matches!(&parts[1], HirExpr::Call(_, _)));
        assert!(matches!(&parts[2], HirExpr::Var(name) if name == "tail"));
        assert!(matches!(&parts[3], HirExpr::ArrayLit(values) if values.len() == 1));
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
    fn lowers_object_literal_shorthand_properties() {
        let program = lower(
            r#"function main(): void {
                const x: number = 1;
                const label: string = "point";
                const point: { x: number; label: string } = { x, label };
                console.log(point.x);
            }"#,
        );

        assert_eq!(
            program.functions[0].body[2],
            HirStmt::Let(
                "point".into(),
                HirType::Object(vec![
                    ("x".into(), HirType::F64),
                    ("label".into(), HirType::Str),
                ]),
                HirExpr::ObjectLit(vec![
                    ("x".into(), HirExpr::Var("x".into())),
                    ("label".into(), HirExpr::Var("label".into())),
                ]),
            )
        );
    }

    #[test]
    fn lowers_static_computed_object_literal_properties() {
        let program = lower(
            r#"function main(): void {
                const point: { x: number; label: string } = {
                    ["x"]: 1,
                    ["label"]: "point"
                };
                console.log(point.label);
            }"#,
        );

        assert_eq!(
            program.functions[0].body[0],
            HirStmt::Let(
                "point".into(),
                HirType::Object(vec![
                    ("x".into(), HirType::F64),
                    ("label".into(), HirType::Str),
                ]),
                HirExpr::ObjectLit(vec![
                    ("x".into(), HirExpr::Lit(HirLit::F64(1.0))),
                    ("label".into(), HirExpr::Lit(HirLit::Str("point".into()))),
                ]),
            )
        );
    }

    #[test]
    fn lowers_local_object_spread_and_later_property_overrides() {
        let program = lower(
            r#"function main(): void {
                const base: { x: number; label: string } = { x: 1, label: "base" };
                const point: { x: number; label: string } = { ...base, label: "point" };
                console.log(point.label);
            }"#,
        );
        let base_type = HirType::Object(vec![
            ("x".into(), HirType::F64),
            ("label".into(), HirType::Str),
        ]);

        assert_eq!(
            program.functions[0].body[1],
            HirStmt::Let(
                "point".into(),
                base_type.clone(),
                HirExpr::ObjectLit(vec![
                    (
                        "x".into(),
                        HirExpr::PropAccess(
                            Box::new(HirExpr::Var("base".into())),
                            base_type,
                            "x".into(),
                        ),
                    ),
                    ("label".into(), HirExpr::Lit(HirLit::Str("point".into()))),
                ]),
            )
        );
    }

    #[test]
    fn lowers_nested_object_literal_spread_without_reloading_fields() {
        let program = lower(
            r#"function main(): void {
                const point: { x: number; label: string } = {
                    ...{ x: 1, label: "base" },
                    label: "point"
                };
                console.log(point.x);
            }"#,
        );

        assert_eq!(
            program.functions[0].body[0],
            HirStmt::Let(
                "point".into(),
                HirType::Object(vec![
                    ("x".into(), HirType::F64),
                    ("label".into(), HirType::Str),
                ]),
                HirExpr::ObjectLit(vec![
                    ("x".into(), HirExpr::Lit(HirLit::F64(1.0))),
                    ("label".into(), HirExpr::Lit(HirLit::Str("point".into()))),
                ]),
            )
        );
    }

    #[test]
    fn evaluates_call_result_object_spread_once() {
        let program = lower(
            r#"function makeConfig(): { x: number; label: string } {
                return { x: 1, label: "base" };
            }
            function main(): void {
                const point: { x: number; label: string } = {
                    ...makeConfig(),
                    label: "point"
                };
                console.log(point.x);
            }"#,
        );
        let main = program
            .functions
            .iter()
            .find(|function| function.name == "main")
            .unwrap();
        let HirStmt::Let(_, _, HirExpr::Call(lambda, arguments)) = &main.body[0] else {
            panic!("expected spread source to be bound through a lambda call");
        };
        assert!(matches!(
            arguments.as_slice(),
            [HirExpr::Call(callee, arguments)]
                if matches!(callee.as_ref(), HirExpr::Var(name) if name == "makeConfig")
                    && arguments.is_empty()
        ));
        let HirExpr::Lambda(_, params, _, body) = lambda.as_ref() else {
            panic!("expected spread binding lambda");
        };
        assert_eq!(params.len(), 1);
        assert!(matches!(
            body.as_ref(),
            HirExpr::ObjectLit(fields)
                if matches!(&fields[0].1, HirExpr::PropAccess(object, _, field)
                    if matches!(object.as_ref(), HirExpr::Var(name) if name == &params[0].name)
                        && field == "x")
                    && fields[1] == ("label".into(), HirExpr::Lit(HirLit::Str("point".into())))
        ));
    }

    #[test]
    fn lowers_conditional_expressions_with_matching_native_types() {
        let program = lower(
            r#"function main(): void {
                const chooseLeft: boolean = true;
                const value: number = chooseLeft ? 1 : 2;
                console.log(value);
            }"#,
        );
        let HirStmt::Let(_, HirType::F64, HirExpr::Call(lambda, arguments)) =
            &program.functions[0].body[1]
        else {
            panic!("expected conditional expression closure call");
        };
        assert!(arguments.is_empty());
        assert!(matches!(
            lambda.as_ref(),
            HirExpr::Lambda(captures, params, HirType::F64, body)
                if captures.len() == 1 && captures[0].name == "chooseLeft"
                    && params.is_empty()
                    && matches!(body.as_ref(), HirExpr::Block(stmts)
                        if matches!(stmts.as_slice(), [HirStmt::If(_, _, _)]))
        ));
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
        // The function result stays unwrapped for native code generation;
        // the await node records the value carried by its runtime promise.
        assert_eq!(fetch_stage.ret, HirType::Str);

        let main = &program.functions[1];
        assert!(main.is_async);
        assert_eq!(main.ret, HirType::Void);
        assert_eq!(
            main.body[0],
            HirStmt::Let(
                "stage".into(),
                HirType::Str,
                HirExpr::AwaitPromise(
                    Box::new(HirExpr::Call(
                        Box::new(HirExpr::Var("fetchStage".into())),
                        vec![],
                    )),
                    HirType::Str,
                ),
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
    fn lowers_promise_constructor_then_and_catch_with_contextual_callbacks() {
        let program = lower(
            r#"async function main(): Promise<void> {
                const value: Promise<string> = new Promise<number>((resolve, reject) => {
                    resolve(20);
                }).then(number => "ready");
                const recovered: Promise<number> = new Promise<number>((resolve, reject) => {
                    reject("failure");
                }).catch(error => 42);
                console.log(await value);
                console.log(await recovered);
            }"#,
        );
        let HirStmt::Let(_, ty, HirExpr::PromiseThen(_, _, input, output, false, false)) =
            &program.functions[0].body[0]
        else {
            panic!("expected a typed Promise.then expression");
        };
        assert_eq!(ty, &HirType::Promise(Box::new(HirType::Str)));
        assert_eq!(input, &HirType::F64);
        assert_eq!(output, &HirType::Str);
        assert!(matches!(
            &program.functions[0].body[1],
            HirStmt::Let(
                _,
                HirType::Promise(_),
                HirExpr::PromiseThen(_, _, HirType::F64, HirType::F64, true, false)
            )
        ));
    }

    #[test]
    fn lowers_promise_void_constructor_with_zero_argument_resolve() {
        let program = lower(
            r#"async function main(): Promise<void> {
                await new Promise<void>((resolve, reject) => {
                    resolve();
                });
            }"#,
        );
        let HirStmt::Expr(HirExpr::Await(inner)) = &program.functions[0].body[0] else {
            panic!("expected awaited Promise<void>");
        };
        let HirExpr::PromiseNew(executor, HirType::Void, false) = inner.as_ref() else {
            panic!("expected Promise<void> constructor");
        };
        let HirExpr::Lambda(_, params, HirType::Void, _) = executor.as_ref() else {
            panic!("expected Promise executor lambda");
        };
        assert!(matches!(
            &params[0].ty,
            HirType::Function(resolve_params, ret)
                if resolve_params.is_empty() && ret.as_ref() == &HirType::Void
        ));
    }

    #[test]
    fn lowers_void_promise_continuations() {
        let program = lower(
            r#"async function main(): Promise<void> {
                await new Promise<void>((resolve, reject) => resolve()).then(() => {});
                await new Promise<void>((resolve, reject) => reject("failure")).catch(error => {
                    console.log(error);
                });
            }"#,
        );
        assert_eq!(program.functions[0].body.len(), 2);
        for stmt in &program.functions[0].body {
            let HirStmt::Expr(HirExpr::Await(inner)) = stmt else {
                panic!("expected awaited continuation");
            };
            assert!(matches!(
                inner.as_ref(),
                HirExpr::PromiseThen(_, _, HirType::Void, HirType::Void, _, false)
            ));
        }
    }

    #[test]
    fn infers_promise_constructor_type_and_reports_conflicting_resolves() {
        let program = lower(
            r#"async function main(): Promise<void> {
                const value: number = await new Promise((resolve, reject) => {
                    resolve(42);
                });
                console.log(value);
            }"#,
        );
        assert!(matches!(
            &program.functions[0].body[0],
            HirStmt::Let(_, HirType::F64, HirExpr::AwaitPromise(_, HirType::F64))
        ));

        let local_program = lower(
            r#"async function main(): Promise<void> {
                const value: number = await new Promise((resolve, reject) => {
                    const base = 20;
                    const answer = base + 22;
                    resolve(answer);
                });
                console.log(value);
            }"#,
        );
        assert!(matches!(
            &local_program.functions[0].body[0],
            HirStmt::Let(_, HirType::F64, HirExpr::AwaitPromise(_, HirType::F64))
        ));

        let module = thaw_parser::parse_typescript(
            r#"function main(): void {
                const value = new Promise((resolve, reject) => {
                    resolve(1);
                    resolve("wrong");
                });
            }"#,
        )
        .unwrap();
        let error = lower_module(&module).unwrap_err();
        assert!(
            error.contains("conflicting Promise resolve types"),
            "{error}"
        );
    }

    #[test]
    fn promise_all_requires_homogeneous_promises() {
        let module = thaw_parser::parse_typescript(
            r#"
            async function value(): Promise<number> {
                await sleep(1);
                return 1;
            }
            async function main(): Promise<void> {
                const values: number[] = await Promise.all([value(), sleep(1)]);
                console.log(values.length);
            }
            "#,
        )
        .unwrap();
        let error = lower_module(&module).unwrap_err();
        assert!(error.contains("Promise.all element 1 resolves to void"));
    }

    #[test]
    fn promise_combinators_accept_homogeneous_array_spreads() {
        let program = lower(
            r#"async function value(input: number): Promise<number> { return input; }
            function pending(): Promise<number>[] { return [value(2), value(3)]; }
            async function main(): Promise<void> {
                await Promise.all([value(1), ...pending()]);
                await Promise.allSettled([...pending(), value(4)]);
                await Promise.race([value(1), ...pending()]);
                await Promise.any([...pending(), value(4)]);
            }"#,
        );
        let main = program
            .functions
            .iter()
            .find(|function| function.name == "main")
            .unwrap();
        assert!(matches!(
            &main.body[0],
            HirStmt::Expr(HirExpr::AwaitPromise(inner, _))
                if matches!(inner.as_ref(), HirExpr::PromiseAllArray(array, _)
                    if matches!(array.as_ref(), HirExpr::ArrayConcat(_, _)))
        ));
        assert!(matches!(
            &main.body[1],
            HirStmt::Expr(HirExpr::AwaitPromise(inner, _))
                if matches!(inner.as_ref(), HirExpr::PromiseAllSettledArray(array, _)
                    if matches!(array.as_ref(), HirExpr::ArrayConcat(_, _)))
        ));
        assert!(matches!(
            &main.body[2],
            HirStmt::Expr(HirExpr::AwaitPromise(inner, _))
                if matches!(inner.as_ref(), HirExpr::PromiseRaceArray(array, _)
                    if matches!(array.as_ref(), HirExpr::ArrayConcat(_, _)))
        ));
        assert!(matches!(
            &main.body[3],
            HirStmt::Expr(HirExpr::AwaitPromise(inner, _))
                if matches!(inner.as_ref(), HirExpr::PromiseAnyArray(array, _)
                    if matches!(array.as_ref(), HirExpr::ArrayConcat(_, _)))
        ));
    }

    #[test]
    fn promise_race_rejects_empty_mixed_and_non_promise_inputs() {
        let empty = thaw_parser::parse_typescript(
            "async function main(): Promise<void> { await Promise.race([]); }",
        )
        .unwrap();
        assert!(lower_module(&empty)
            .unwrap_err()
            .contains("requires at least one promise"));

        let mixed = thaw_parser::parse_typescript(
            r#"
            async function numberValue(): Promise<number> { return 1; }
            async function stringValue(): Promise<string> { return "x"; }
            async function main(): Promise<void> {
                await Promise.race([numberValue(), stringValue()]);
            }
            "#,
        )
        .unwrap();
        assert!(lower_module(&mixed)
            .unwrap_err()
            .contains("Promise.race element 1 resolves to Str, expected F64"));

        let plain = thaw_parser::parse_typescript(
            "async function main(): Promise<void> { await Promise.race([1]); }",
        )
        .unwrap();
        assert!(lower_module(&plain)
            .unwrap_err()
            .contains("Promise.race element 0 must be a Promise"));
    }

    #[test]
    fn promise_any_rejects_empty_mixed_and_non_promise_inputs() {
        let empty = thaw_parser::parse_typescript(
            "async function main(): Promise<void> { await Promise.any([]); }",
        )
        .unwrap();
        assert!(lower_module(&empty)
            .unwrap_err()
            .contains("requires at least one promise"));

        let mixed = thaw_parser::parse_typescript(
            r#"
            async function numberValue(): Promise<number> { return 1; }
            async function stringValue(): Promise<string> { return "x"; }
            async function main(): Promise<void> {
                await Promise.any([numberValue(), stringValue()]);
            }
            "#,
        )
        .unwrap();
        assert!(lower_module(&mixed)
            .unwrap_err()
            .contains("Promise.any element 1 resolves to Str, expected F64"));

        let plain = thaw_parser::parse_typescript(
            "async function main(): Promise<void> { await Promise.any([1]); }",
        )
        .unwrap();
        assert!(lower_module(&plain)
            .unwrap_err()
            .contains("Promise.any element 0 must be a Promise"));
    }

    #[test]
    fn promise_all_settled_rejects_mixed_and_non_promise_inputs() {
        let mixed = thaw_parser::parse_typescript(
            r#"
            async function numberValue(): Promise<number> { return 1; }
            async function stringValue(): Promise<string> { return "x"; }
            async function main(): Promise<void> {
                await Promise.allSettled([numberValue(), stringValue()]);
            }
            "#,
        )
        .unwrap();
        assert!(lower_module(&mixed)
            .unwrap_err()
            .contains("Promise.allSettled element 1 resolves to Str, expected F64"));

        let plain = thaw_parser::parse_typescript(
            "async function main(): Promise<void> { await Promise.allSettled([1]); }",
        )
        .unwrap();
        assert!(lower_module(&plain)
            .unwrap_err()
            .contains("Promise.allSettled element 0 must be a Promise"));
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
    fn lowers_number_conversion_through_native_object_stringification() {
        let program = lower("function main(): void { const x: number = Number({ value: 1 }); }");
        assert!(matches!(
            &program.functions[0].body[0],
            HirStmt::Let(_, HirType::F64, HirExpr::Call(callee, _))
                if matches!(callee.as_ref(), HirExpr::Var(name) if name == "__thaw_string_to_number")
        ));
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
                param_string_abis: vec![crate::FfiStringAbi::NullTerminated; 2],
                return_string_abi: crate::FfiStringAbi::NullTerminated,
                calling_convention: crate::FfiCallingConvention::C,
                aggregate_return_abi: crate::FfiAggregateAbi::Internal,
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
                        param_string_abis: vec![crate::FfiStringAbi::NullTerminated; 2],
                        return_string_abi: crate::FfiStringAbi::NullTerminated,
                        calling_convention: crate::FfiCallingConvention::C,
                        aggregate_return_abi: crate::FfiAggregateAbi::Internal,
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

    #[test]
    fn lowers_calls_through_function_typed_object_properties() {
        let program = lower(
            r#"interface Operations { apply: (value: number) => number; }
            function main(): void {
                const operations: Operations = {
                    apply: (value: number): number => value + 1
                };
                console.log(operations.apply(41));
            }"#,
        );
        assert!(matches!(
            &program.functions[0].body[1],
            HirStmt::Expr(HirExpr::Call(_, args))
                if matches!(&args[0], HirExpr::Call(callee, _)
                    if matches!(callee.as_ref(), HirExpr::PropAccess(_, _, field) if field == "apply"))
        ));
    }
}
