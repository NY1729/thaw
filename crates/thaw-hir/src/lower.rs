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
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt;

use swc_common::{BytePos, SourceMap};
use swc_ecma_ast::{
    ArrowFunctionBody, AssignOp, AssignTarget, AwaitExpr, BinaryOp, CallExpr, Callee, ClassDecl,
    ClassMember, ClassMethod, ClassProp, ComputedPropName, Decl, Expr, FnDecl, ForHead, IdentName,
    KeyValueProp, Lit, MemberExpr, MemberProp, MethodKind, Module, ModuleDecl, ModuleItem,
    ObjectLit as SwcObjectLit, ObjectPatProp, OptChainBase, ParamOrTsParamProp, Pat, Prop,
    PropName, PropOrSpread, SimpleAssignTarget, Stmt, SuperProp, TsFnOrConstructorType, TsFnParam,
    TsInterfaceDecl, TsKeywordTypeKind, TsLit, TsParamPropParam, TsType, TsTypeElement,
    TsUnionOrIntersectionType, UnaryOp, UpdateOp, VarDecl, VarDeclOrExpr,
};
use swc_ecma_visit::{Visit, VisitMut, VisitMutWith, VisitWith};

use crate::{
    BinOp, DynamicBackend, DynamicSignature, FfiAggregateAbi, FfiCallingConvention, FfiErrorAbi,
    FfiOwnership, FfiSignature, FfiStringAbi, HirExpr, HirFunction, HirInitStep, HirLit,
    HirOptionalMask, HirParam, HirProgram, HirStmt, HirType, Symbol,
};

type LoweredBinding = (Symbol, HirType, HirExpr);

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
    variadic: Option<HirType>,
    native_rest: Option<HirType>,
    abstract_class_constructor: bool,
    ret: HirType,
    is_async: bool,
    /// Whether a native class method observes its call-site `this` value.
    /// This distinguishes safely extractable methods from methods that need
    /// the dedicated unbound-method ABI rather than an ordinary closure.
    uses_this: bool,
    /// A function declared with no body (`declare function foo(...): T;`,
    /// or the same syntax without `declare` in a regular `.ts` file --
    /// SWC represents both identically, `body: None`). See
    /// docs/design/bridge.md section 6: calls to these lower to
    /// `HirExpr::FfiCall`, not `HirExpr::Call`.
    is_extern: bool,
    source_range: (u32, u32),
    generic_type_params: Vec<Symbol>,
    generic_type_constraints: Vec<Option<Box<TsType>>>,
    generic_type_defaults: Vec<Option<Box<TsType>>>,
    generic_param_patterns: Vec<GenericTypePattern>,
    generic_param_optional: Vec<bool>,
    generic_return_type: Option<Box<TsType>>,
}

#[derive(Clone)]
struct NativeMethodValue {
    symbol: Symbol,
    receiver: Option<HirType>,
}

#[derive(Default)]
struct ThisUseCollector {
    found: bool,
}

impl Visit for ThisUseCollector {
    fn visit_this_expr(&mut self, _: &swc_ecma_ast::ThisExpr) {
        self.found = true;
    }
}

fn function_uses_this(function: &swc_ecma_ast::Function) -> bool {
    let mut collector = ThisUseCollector::default();
    function.visit_with(&mut collector);
    collector.found
}

struct IdentifierUseCollector<'a> {
    name: &'a str,
    found: bool,
}

impl Visit for IdentifierUseCollector<'_> {
    fn visit_ident(&mut self, identifier: &swc_ecma_ast::Ident) {
        if identifier.sym == *self.name {
            self.found = true;
        }
    }
}

fn named_function_is_recursive(expression: &swc_ecma_ast::FnExpr) -> bool {
    let Some(name) = &expression.ident else {
        return false;
    };
    let Some(body) = &expression.function.body else {
        return false;
    };
    let mut collector = IdentifierUseCollector {
        name: name.sym.as_ref(),
        found: false,
    };
    body.visit_with(&mut collector);
    collector.found
}

#[derive(Clone, Debug, PartialEq)]
enum GenericTypePattern {
    Variable(Symbol),
    Concrete(HirType),
    Array(Box<GenericTypePattern>),
    Promise(Box<GenericTypePattern>),
    Optional(Box<GenericTypePattern>),
    Awaited(Box<GenericTypePattern>),
    NonNullable(Box<GenericTypePattern>),
    Partial(Box<GenericTypePattern>),
    Required(Box<GenericTypePattern>),
    Record(Vec<Symbol>, Box<GenericTypePattern>),
    Pick(Box<GenericTypePattern>, Vec<Symbol>),
    Omit(Box<GenericTypePattern>, Vec<Symbol>),
    IndexedAccess(Box<GenericTypePattern>, Vec<Symbol>),
    Dictionary(Box<GenericTypePattern>),
    Object(Vec<(Symbol, GenericTypePattern)>),
}

#[derive(PartialEq)]
struct GenericClassMethodShape {
    parameters: Vec<GenericTypePattern>,
    optional: Vec<bool>,
    rest: bool,
    result: GenericTypePattern,
    constraints: Vec<Option<GenericTypePattern>>,
    defaults: Vec<Option<GenericTypePattern>>,
    is_async: bool,
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

include!("lower/normalize.rs");

include!("lower/classes.rs");

fn declaration_names_for_normalization(declaration: &Decl) -> Vec<String> {
    struct Collector(Vec<String>);
    impl Visit for Collector {
        fn visit_binding_ident(&mut self, binding: &swc_ecma_ast::BindingIdent) {
            self.0.push(binding.id.sym.to_string());
        }
    }
    let mut collector = Collector(Vec::new());
    declaration.visit_with(&mut collector);
    collector.0
}

fn class_property_name(name: &PropName) -> Result<Symbol, String> {
    match name {
        PropName::Ident(name) => Ok(name.sym.to_string()),
        PropName::Str(name) => Ok(name.value.to_string_lossy().into_owned()),
        PropName::Computed(computed) => match computed.expr.as_ref() {
            Expr::Lit(Lit::Str(name)) => Ok(name.value.to_string_lossy().into_owned()),
            _ => Err("native class computed members require a string-literal name".into()),
        },
        _ => Err("native class members require an identifier or string-literal name".into()),
    }
}

fn member_property_name(name: &MemberProp) -> Option<Symbol> {
    match name {
        MemberProp::Ident(name) => Some(name.sym.to_string()),
        MemberProp::Computed(computed) => match computed.expr.as_ref() {
            Expr::Lit(Lit::Str(name)) => Some(name.value.to_string_lossy().into_owned()),
            _ => None,
        },
        _ => None,
    }
}

fn ordinary_optional_chain_expression(chain: &swc_ecma_ast::OptChainExpr) -> Expr {
    match chain.base.as_ref() {
        OptChainBase::Member(member) => Expr::Member(member.clone()),
        OptChainBase::Call(call) => Expr::Call(CallExpr::from(call.clone())),
    }
}

fn ordinary_optional_expression(expression: &Expr) -> Expr {
    match expression {
        Expr::OptChain(chain) => ordinary_optional_chain_expression(chain),
        expression => expression.clone(),
    }
}

fn class_constructor_symbol(name: &str) -> Symbol {
    format!("__thaw_class_{name}_constructor")
}

fn class_initializer_symbol(name: &str) -> Symbol {
    format!("__thaw_class_{name}_initialize")
}

fn default_arity_symbol(symbol: &str, arity: usize) -> Symbol {
    format!("{symbol}__thawdefault_arity_{arity}")
}

fn omitted_parameter_symbol(symbol: &str, mask: usize) -> Symbol {
    format!("{symbol}__thawomitted_mask_{mask:x}")
}

fn pattern_is_omittable(pattern: &Pat) -> bool {
    matches!(pattern, Pat::Assign(_))
        || matches!(pattern, Pat::Ident(binding) if binding.id.optional)
}

fn optional_parameter_type(ty: HirType) -> HirType {
    match ty {
        HirType::Optional(_) | HirType::Nullish(_) => ty,
        HirType::Nullable(payload) => HirType::Nullish(payload),
        other => HirType::Optional(Box::new(other)),
    }
}

fn optional_parameter_mask(optional: &[bool]) -> HirOptionalMask {
    HirOptionalMask::from_bools(optional)
}

fn is_optional_parameter(mask: &HirOptionalMask, index: usize) -> bool {
    mask.contains(index)
}

fn omitted_parameter_value(ty: &HirType) -> Result<HirExpr, String> {
    match ty {
        HirType::Optional(payload) => Ok(HirExpr::OptionalNone(payload.as_ref().clone())),
        HirType::Nullish(payload) => Ok(HirExpr::NullishUndefined(payload.as_ref().clone())),
        other => Err(format!(
            "omittable callable parameter needs an undefined-capable ABI type, got {other:?}"
        )),
    }
}

fn trailing_omittable_start(patterns: &[Pat]) -> Option<usize> {
    let start = patterns
        .iter()
        .rposition(|pattern| !pattern_is_omittable(pattern))
        .map_or(0, |index| index + 1);
    (start < patterns.len()).then_some(start)
}

fn omitted_parameter_masks(patterns: &[Pat], receiver_count: usize) -> Result<Vec<usize>, String> {
    let omittable = patterns
        .iter()
        .enumerate()
        .filter_map(|(index, pattern)| {
            pattern_is_omittable(pattern).then_some(index + receiver_count)
        })
        .collect::<Vec<_>>();
    if omittable.len() > 16 {
        return Err("native class callables support at most 16 omittable parameters".into());
    }
    let mut masks = Vec::with_capacity((1usize << omittable.len()).saturating_sub(1));
    for subset in 1usize..(1usize << omittable.len()) {
        let mut mask = 0usize;
        for (bit, parameter) in omittable.iter().enumerate() {
            if subset & (1usize << bit) != 0 {
                mask |= 1usize << parameter;
            }
        }
        masks.push(mask);
    }
    Ok(masks)
}

fn native_rest_element(patterns: &[Pat], params: &[HirType]) -> Result<Option<HirType>, String> {
    if patterns
        .iter()
        .take(patterns.len().saturating_sub(1))
        .any(|pattern| matches!(pattern, Pat::Rest(_)))
    {
        return Err("native class rest parameter must be last".into());
    }
    if !patterns
        .last()
        .is_some_and(|pattern| matches!(pattern, Pat::Rest(_)))
    {
        return Ok(None);
    }
    match params.last() {
        Some(HirType::Array(element)) => Ok(Some(element.as_ref().clone())),
        Some(other) => Err(format!(
            "native class rest parameter needs an array annotation, got {other:?}"
        )),
        None => Err("native class rest parameter is missing its signature type".into()),
    }
}

fn native_rest_array(values: Vec<HirExpr>, element: &HirType) -> HirExpr {
    if values.is_empty() {
        HirExpr::ArrayAlloc(Box::new(HirExpr::Lit(HirLit::F64(0.0))), element.clone())
    } else {
        HirExpr::ArrayLit(values)
    }
}

fn insert_omitted_parameter_signatures(
    signatures: &mut HashMap<Symbol, FnSignature>,
    symbol: &str,
    patterns: &[Pat],
    receiver_count: usize,
) -> Result<(), String> {
    let full = signatures[symbol].clone();
    for mask in omitted_parameter_masks(patterns, receiver_count)? {
        let mut wrapper = full.clone();
        wrapper.params = wrapper
            .params
            .into_iter()
            .enumerate()
            .filter_map(|(index, parameter)| (mask & (1usize << index) == 0).then_some(parameter))
            .collect();
        signatures.insert(omitted_parameter_symbol(symbol, mask), wrapper);
    }
    Ok(())
}

fn class_method_symbol(class: &str, method: &str) -> Symbol {
    format!("__thaw_class_{class}_method_{method}")
}

fn unbound_class_method_symbol(method: &str) -> Symbol {
    format!("{method}__thaw_unbound")
}

fn class_static_method_symbol(class: &str, method: &str) -> Symbol {
    format!("__thaw_class_{class}_static_{method}")
}

fn class_static_field_symbol(class: &str, field: &str) -> Symbol {
    format!("__thaw_class_{class}_static_field_{field}")
}

fn is_class_static_field_symbol(symbol: &str) -> bool {
    symbol.starts_with("__thaw_class_") && symbol.contains("_static_field_")
}

fn class_getter_symbol(class: &str, property: &str, is_static: bool) -> Symbol {
    format!(
        "__thaw_class_{class}_{}_getter_{property}",
        if is_static { "static" } else { "instance" }
    )
}

fn class_setter_symbol(class: &str, property: &str, is_static: bool) -> Symbol {
    format!(
        "__thaw_class_{class}_{}_setter_{property}",
        if is_static { "static" } else { "instance" }
    )
}

fn class_member_symbol(class: &str, member: &swc_ecma_ast::ClassMethod) -> Result<Symbol, String> {
    let name = class_property_name(&member.key)?;
    Ok(match member.kind {
        MethodKind::Getter => class_getter_symbol(class, &name, member.is_static),
        MethodKind::Setter => class_setter_symbol(class, &name, member.is_static),
        MethodKind::Method if member.is_static => class_static_method_symbol(class, &name),
        MethodKind::Method => class_method_symbol(class, &name),
    })
}

fn super_property_name(property: &SuperProp) -> Result<Symbol, String> {
    match property {
        SuperProp::Ident(name) => Ok(name.sym.to_string()),
        SuperProp::Computed(computed) => match computed.expr.as_ref() {
            Expr::Lit(Lit::Str(name)) => Ok(name.value.to_string_lossy().into_owned()),
            _ => Err("computed super properties require a string literal name".into()),
        },
    }
}

fn class_name_from_type(ty: &HirType) -> Option<&str> {
    let HirType::Object(fields) = ty else {
        return None;
    };
    fields.first().and_then(|(name, ty)| {
        (*ty == HirType::Bool)
            .then(|| name.strip_prefix("__thaw_class_identity_"))
            .flatten()
            .and_then(|identities| identities.split('$').next())
    })
}

fn class_type_has_identity(ty: &HirType, expected: &str) -> bool {
    let HirType::Object(fields) = ty else {
        return false;
    };
    fields.first().is_some_and(|(name, ty)| {
        *ty == HirType::Bool
            && name
                .strip_prefix("__thaw_class_identity_")
                .is_some_and(|identities| identities.split('$').any(|name| name == expected))
    })
}

fn class_constructor_param_pattern(parameter: &ParamOrTsParamProp) -> Pat {
    match parameter {
        ParamOrTsParamProp::Param(parameter) => parameter.pat.clone(),
        ParamOrTsParamProp::TsParamProp(property) => match &property.param {
            TsParamPropParam::Ident(binding) => Pat::Ident(binding.clone()),
            TsParamPropParam::Assign(assignment) => Pat::Assign(assignment.clone()),
        },
    }
}

fn parameter_property_binding(
    property: &swc_ecma_ast::TsParamProp,
) -> Result<&swc_ecma_ast::BindingIdent, String> {
    match &property.param {
        TsParamPropParam::Ident(binding) => Ok(binding),
        TsParamPropParam::Assign(assignment) => match assignment.left.as_ref() {
            Pat::Ident(binding) => Ok(binding),
            _ => Err("constructor parameter properties require identifier bindings".into()),
        },
    }
}

fn collect_native_classes<'a>(
    module: &'a Module,
    interfaces: &mut HashMap<Symbol, HirType>,
    generic_interfaces: &GenericInterfaces,
) -> Result<Vec<&'a ClassDecl>, String> {
    fn resolve_layout(
        name: &str,
        declarations: &HashMap<Symbol, &ClassDecl>,
        interfaces: &mut HashMap<Symbol, HirType>,
        generic_interfaces: &GenericInterfaces,
        resolved: &mut HashSet<Symbol>,
        active: &mut Vec<Symbol>,
    ) -> Result<(), String> {
        if resolved.contains(name) {
            return Ok(());
        }
        if active.iter().any(|current| current == name) {
            active.push(name.to_string());
            return Err(format!("class inheritance cycle `{}`", active.join(" -> ")));
        }
        let declaration = declarations
            .get(name)
            .ok_or_else(|| format!("unknown native class `{name}`"))?;
        active.push(name.to_string());
        let mut inherited_fields = Vec::new();
        let mut identities = vec![name.to_string()];
        if let Some(base) = &declaration.class.super_class {
            let Expr::Ident(base) = base.as_ref() else {
                return Err(format!(
                    "class `{name}` requires an identifier in its extends clause"
                ));
            };
            let base_name = base.sym.as_ref();
            if !declarations.contains_key(base_name) {
                return Err(format!(
                    "class `{name}` extends unknown native class `{base_name}`"
                ));
            }
            resolve_layout(
                base_name,
                declarations,
                interfaces,
                generic_interfaces,
                resolved,
                active,
            )?;
            let HirType::Object(base_fields) = &interfaces[base_name] else {
                unreachable!("native class layouts are objects")
            };
            if let Some((marker, HirType::Bool)) = base_fields.first() {
                let inherited = marker
                    .strip_prefix("__thaw_class_identity_")
                    .expect("base class identity marker");
                identities.extend(inherited.split('$').map(str::to_owned));
            }
            inherited_fields.extend(base_fields.iter().skip(1).cloned());
        }
        let mut fields = vec![(
            format!("__thaw_class_identity_{}", identities.join("$")),
            HirType::Bool,
        )];
        fields.extend(inherited_fields);
        for member in &declaration.class.body {
            let ClassMember::ClassProp(property) = member else {
                continue;
            };
            if property.is_static {
                continue;
            }
            if property.declare {
                return Err(format!(
                    "class `{name}` field `{}` cannot be ambient",
                    class_property_name(&property.key)?
                ));
            }
            let field_name = class_property_name(&property.key)?;
            let annotation = property.type_ann.as_ref().ok_or_else(|| {
                format!("class `{name}` field `{field_name}` needs a type annotation")
            })?;
            let mut field_type =
                lower_ts_type(&annotation.type_ann, interfaces, generic_interfaces)?;
            if property.is_optional {
                field_type = optional_parameter_type(field_type);
            }
            if let Some((_, inherited_type)) =
                fields.iter().find(|(existing, _)| existing == &field_name)
            {
                let mut base_name =
                    declaration
                        .class
                        .super_class
                        .as_deref()
                        .and_then(|base| match base {
                            Expr::Ident(base) => Some(base.sym.to_string()),
                            _ => None,
                        });
                let mut overrides_abstract = false;
                while let Some(current) = base_name {
                    let base = declarations[&current];
                    if base.class.body.iter().any(|member| {
                        matches!(member, ClassMember::ClassProp(candidate)
                            if !candidate.is_static
                                && candidate.is_abstract
                                && class_property_name(&candidate.key).ok().as_deref()
                                    == Some(field_name.as_str()))
                    }) {
                        overrides_abstract = true;
                        break;
                    }
                    base_name = base
                        .class
                        .super_class
                        .as_deref()
                        .and_then(|parent| match parent {
                            Expr::Ident(parent) => Some(parent.sym.to_string()),
                            _ => None,
                        });
                }
                if !overrides_abstract {
                    return Err(format!(
                        "class `{name}` field `{field_name}` collides with an inherited or local field"
                    ));
                }
                if inherited_type != &field_type {
                    return Err(format!(
                        "class `{name}` implements abstract field `{field_name}` with type {field_type:?}, expected {inherited_type:?}"
                    ));
                }
            } else {
                fields.push((field_name, field_type));
            }
        }
        for property in declaration
            .class
            .body
            .iter()
            .filter_map(|member| match member {
                ClassMember::Constructor(constructor) => Some(&constructor.params),
                _ => None,
            })
            .flatten()
            .filter_map(|parameter| match parameter {
                ParamOrTsParamProp::TsParamProp(property) => Some(property),
                ParamOrTsParamProp::Param(_) => None,
            })
        {
            let binding = parameter_property_binding(property)?;
            let field_name = binding.id.sym.to_string();
            if fields.iter().any(|(existing, _)| existing == &field_name) {
                return Err(format!(
                    "class `{name}` parameter property duplicates an inherited or local field `{field_name}`"
                ));
            }
            let annotation = binding.type_ann.as_ref().ok_or_else(|| {
                format!("class `{name}` parameter property `{field_name}` needs a type annotation")
            })?;
            let mut ty = lower_ts_type(&annotation.type_ann, interfaces, generic_interfaces)?;
            if binding.id.optional {
                ty = optional_parameter_type(ty);
            }
            fields.push((field_name, ty));
        }
        for implementation in &declaration.class.implements {
            let Expr::Ident(target) = implementation.expr.as_ref() else {
                return Err(format!(
                    "class `{name}` requires an identifier in its implements clause"
                ));
            };
            if declarations.contains_key(target.sym.as_ref()) {
                return Err(format!(
                    "class `{name}` cannot use native class `{}` as an implements target",
                    target.sym
                ));
            }
            let reference = TsType::TsTypeRef(swc_ecma_ast::TsTypeRef {
                span: implementation.span,
                type_name: swc_ecma_ast::TsEntityName::Ident(target.clone()),
                type_params: implementation.type_args.clone(),
            });
            let required =
                lower_ts_type(&reference, interfaces, generic_interfaces).map_err(|error| {
                    format!(
                        "class `{name}` has invalid implements target `{}`: {error}",
                        target.sym
                    )
                })?;
            let HirType::Object(required_fields) = required else {
                return Err(format!(
                    "class `{name}` implements non-object type `{}`",
                    target.sym
                ));
            };
            for (field, required_type) in &required_fields {
                let actual_type = fields
                    .iter()
                    .find_map(|(candidate, ty)| (candidate == field).then_some(ty))
                    .ok_or_else(|| {
                        format!(
                            "class `{name}` is missing field `{field}` required by `{}`",
                            target.sym
                        )
                    })?;
                if actual_type != required_type {
                    return Err(format!(
                        "class `{name}` field `{field}` has type {actual_type:?}, but `{}` requires {required_type:?}",
                        target.sym
                    ));
                }
            }
        }
        interfaces.insert(name.to_string(), HirType::Object(fields));
        active.pop();
        resolved.insert(name.to_string());
        Ok(())
    }

    let mut classes = Vec::new();
    let mut declarations = HashMap::new();
    for item in &module.body {
        let ModuleItem::Stmt(Stmt::Decl(Decl::Class(declaration))) = item else {
            continue;
        };
        let name = declaration.ident.sym.to_string();
        if declaration.declare {
            return Err(format!(
                "ambient class `{name}` cannot use the native class path"
            ));
        }
        if declaration.class.type_params.is_some() {
            return Err(format!(
                "class `{name}` currently requires a non-generic class"
            ));
        }
        if !declaration.class.is_abstract
            && declaration.class.body.iter().any(|member| match member {
                ClassMember::Method(method) => method.is_abstract,
                ClassMember::ClassProp(property) => property.is_abstract,
                _ => false,
            })
        {
            return Err(format!(
                "concrete class `{name}` cannot declare abstract members"
            ));
        }
        if interfaces.contains_key(&name)
            || declarations.insert(name.clone(), declaration).is_some()
        {
            return Err(format!(
                "class `{name}` conflicts with an interface or type declaration"
            ));
        }
        classes.push(declaration);
    }
    let mut resolved = HashSet::new();
    for declaration in &classes {
        resolve_layout(
            declaration.ident.sym.as_ref(),
            &declarations,
            interfaces,
            generic_interfaces,
            &mut resolved,
            &mut Vec::new(),
        )?;
    }
    for declaration in &classes {
        if declaration.class.is_abstract {
            continue;
        }
        let name = declaration.ident.sym.as_ref();
        let mut seen = HashSet::new();
        let mut current = Some(name.to_string());
        while let Some(current_name) = current {
            let class = declarations[&current_name];
            for member in &class.class.body {
                let ClassMember::ClassProp(property) = member else {
                    continue;
                };
                if property.is_static {
                    continue;
                }
                let field = class_property_name(&property.key)?;
                if !seen.insert(field.clone()) {
                    continue;
                }
                if property.is_abstract {
                    return Err(format!(
                        "concrete class `{name}` must implement abstract field `{field}` from `{current_name}`"
                    ));
                }
            }
            current = class
                .class
                .super_class
                .as_deref()
                .and_then(|parent| match parent {
                    Expr::Ident(parent) => Some(parent.sym.to_string()),
                    _ => None,
                });
        }
    }
    Ok(classes)
}

pub fn lower_module(module: &Module) -> Result<HirProgram, String> {
    let normalized = normalize_top_level_destructuring(module)?;
    let normalized = normalize_top_level_class_expressions(&normalized)?;
    let normalized = normalize_static_computed_class_members(&normalized);
    let normalized = normalize_private_class_members(&normalized);
    lower_normalized_module(&normalized)
}

fn lower_normalized_module(module: &Module) -> Result<HirProgram, String> {
    let (mut interfaces, generic_interfaces) = resolve_interfaces(module)?;
    let specialized = specialize_generic_classes(module, &interfaces, &generic_interfaces)
        .map_err(|error| {
            if error.contains("generics are not supported yet") {
                format!(
                    "recursive or unresolved generic class type cannot use Thaw's fixed-size native layout: {error}"
                )
            } else {
                error
            }
        })?;
    if let Some(specialized) = specialized {
        return lower_normalized_module(&specialized).map_err(|error| {
            if error.contains("generics are not supported yet") {
                format!(
                    "recursive or unresolved generic class type cannot use Thaw's fixed-size native layout: {error}"
                )
            } else {
                error
            }
        });
    }
    let (enum_values, enum_reverse_values, enum_types) = collect_enums(module)?;
    for (name, ty) in enum_types {
        if interfaces.insert(name.clone(), ty).is_some() {
            return Err(format!(
                "enum `{name}` conflicts with an interface declaration"
            ));
        }
    }
    let class_decls = collect_native_classes(module, &mut interfaces, &generic_interfaces)?;
    if let Some(specialized) =
        specialize_generic_class_methods(module, &interfaces, &generic_interfaces)?
    {
        return lower_normalized_module(&specialized);
    }

    let mut signatures: HashMap<Symbol, FnSignature> = HashMap::new();
    let mut fn_decls = Vec::new();
    let mut global_decls = Vec::new();
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
                let generic_type_constraints = func
                    .type_params
                    .as_ref()
                    .map(|parameters| {
                        parameters
                            .params
                            .iter()
                            .map(|parameter| parameter.constraint.clone())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let generic_type_defaults = func
                    .type_params
                    .as_ref()
                    .map(|parameters| {
                        parameters
                            .params
                            .iter()
                            .map(|parameter| parameter.default.clone())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
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
                let variadic = if is_extern {
                    func.params.last().and_then(|param| match &param.pat {
                        Pat::Rest(rest) => Some(rest),
                        _ => None,
                    }).map(|rest| {
                        let annotation = rest.type_ann.as_ref().ok_or_else(|| {
                            format!("ambient variadic function `{name}` needs a rest parameter type annotation")
                        })?;
                        let ty = resolve_ts_type_with_substitution(
                            &annotation.type_ann,
                            &type_substitution,
                            &interfaces,
                            &generic_interfaces,
                            &mut Vec::new(),
                        )?;
                        match ty {
                            HirType::Array(element)
                                if supports_ffi_variadic_element(&element) => Ok(*element),
                            other => Err(format!(
                                "ambient variadic function `{name}` has unsupported rest element layout {other:?}"
                            )),
                        }
                    }).transpose()?
                } else {
                    None
                };
                let fixed_param_count = func.params.len() - usize::from(variadic.is_some());
                let params = func
                    .params
                    .iter()
                    .take(fixed_param_count)
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
                let generates_call_wrappers = !is_extern && generic_type_params.is_empty();
                signatures.insert(
                    name.clone(),
                    FnSignature {
                        params,
                        variadic,
                        native_rest: None,
                        abstract_class_constructor: false,
                        ret,
                        is_async: func.is_async,
                        uses_this: false,
                        is_extern,
                        source_range: (func.span.lo.0, func.span.hi.0),
                        generic_type_params,
                        generic_type_constraints,
                        generic_type_defaults,
                        generic_param_patterns,
                        generic_param_optional: func
                            .params
                            .iter()
                            .map(|parameter| {
                                matches!(&parameter.pat, Pat::Ident(binding) if binding.id.optional)
                            })
                            .collect(),
                        generic_return_type: func.return_type.as_ref().map(|ann| ann.type_ann.clone()),
                    },
                );
                if generates_call_wrappers {
                    let patterns = func
                        .params
                        .iter()
                        .map(|parameter| parameter.pat.clone())
                        .collect::<Vec<_>>();
                    if let Some(default_start) = trailing_omittable_start(&patterns) {
                        for arity in default_start..patterns.len() {
                            let mut wrapper = signatures[&name].clone();
                            wrapper.params.truncate(arity);
                            signatures.insert(default_arity_symbol(&name, arity), wrapper);
                        }
                    }
                    insert_omitted_parameter_signatures(&mut signatures, &name, &patterns, 0)?;
                }
                if !is_extern {
                    fn_decls.push(fn_decl);
                }
            }
            ModuleItem::Stmt(Stmt::Decl(Decl::Class(class_decl))) => {
                let name = class_decl.ident.sym.to_string();
                let instance_type = interfaces[&name].clone();
                let constructors = class_decl
                    .class
                    .body
                    .iter()
                    .filter_map(|member| match member {
                        ClassMember::Constructor(constructor) => Some(constructor),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                if constructors.len() > 1 {
                    return Err(format!(
                        "class `{name}` has multiple constructor implementations"
                    ));
                }
                let params = constructors
                    .first()
                    .map(|constructor| {
                        constructor
                            .params
                            .iter()
                            .map(|parameter| {
                                lower_param(
                                    &class_constructor_param_pattern(parameter),
                                    &interfaces,
                                    &generic_interfaces,
                                    false,
                                    &HashMap::new(),
                                )
                                .map(|parameter| parameter.ty)
                            })
                            .collect::<Result<Vec<_>, _>>()
                    })
                    .transpose()?
                    .unwrap_or_default();
                signatures.insert(
                    class_constructor_symbol(&name),
                    FnSignature {
                        params,
                        variadic: None,
                        native_rest: None,
                        abstract_class_constructor: class_decl.class.is_abstract,
                        ret: instance_type.clone(),
                        is_async: false,
                        uses_this: false,
                        is_extern: false,
                        source_range: (class_decl.class.span.lo.0, class_decl.class.span.hi.0),
                        generic_type_params: Vec::new(),
                        generic_type_constraints: Vec::new(),
                        generic_type_defaults: Vec::new(),
                        generic_param_patterns: Vec::new(),
                        generic_param_optional: Vec::new(),
                        generic_return_type: None,
                    },
                );
                let mut initializer_params = vec![instance_type.clone()];
                initializer_params.extend(
                    signatures[&class_constructor_symbol(&name)]
                        .params
                        .iter()
                        .cloned(),
                );
                signatures.insert(
                    class_initializer_symbol(&name),
                    FnSignature {
                        params: initializer_params,
                        variadic: None,
                        native_rest: None,
                        abstract_class_constructor: false,
                        ret: instance_type.clone(),
                        is_async: false,
                        uses_this: false,
                        is_extern: false,
                        source_range: (class_decl.class.span.lo.0, class_decl.class.span.hi.0),
                        generic_type_params: Vec::new(),
                        generic_type_constraints: Vec::new(),
                        generic_type_defaults: Vec::new(),
                        generic_param_patterns: Vec::new(),
                        generic_param_optional: Vec::new(),
                        generic_return_type: None,
                    },
                );
                if let Some(constructor) = constructors.first() {
                    let patterns = constructor
                        .params
                        .iter()
                        .map(class_constructor_param_pattern)
                        .collect::<Vec<_>>();
                    let constructor_symbol = class_constructor_symbol(&name);
                    let initializer_symbol = class_initializer_symbol(&name);
                    let native_rest =
                        native_rest_element(&patterns, &signatures[&constructor_symbol].params)?;
                    signatures
                        .get_mut(&constructor_symbol)
                        .expect("class constructor signature")
                        .native_rest = native_rest.clone();
                    signatures
                        .get_mut(&initializer_symbol)
                        .expect("class initializer signature")
                        .native_rest = native_rest;
                    if let Some(default_start) = trailing_omittable_start(&patterns) {
                        for arity in default_start..patterns.len() {
                            let mut constructor_signature = signatures[&constructor_symbol].clone();
                            constructor_signature.params.truncate(arity);
                            signatures.insert(
                                default_arity_symbol(&constructor_symbol, arity),
                                constructor_signature,
                            );
                            let mut initializer_signature = signatures[&initializer_symbol].clone();
                            initializer_signature.params.truncate(arity + 1);
                            signatures.insert(
                                default_arity_symbol(&initializer_symbol, arity + 1),
                                initializer_signature,
                            );
                        }
                    }
                    insert_omitted_parameter_signatures(
                        &mut signatures,
                        &class_constructor_symbol(&name),
                        &patterns,
                        0,
                    )?;
                    insert_omitted_parameter_signatures(
                        &mut signatures,
                        &class_initializer_symbol(&name),
                        &patterns,
                        1,
                    )?;
                }
                for member in &class_decl.class.body {
                    let ClassMember::Method(method) = member else {
                        continue;
                    };
                    if !matches!(
                        method.kind,
                        MethodKind::Method | MethodKind::Getter | MethodKind::Setter
                    ) {
                        continue;
                    }
                    if method.function.type_params.is_some() || method.function.is_generator {
                        return Err(format!(
                            "class `{name}` method `{}` cannot be generic or a generator yet",
                            class_property_name(&method.key)?
                        ));
                    }
                    let method_name = class_property_name(&method.key)?;
                    if method.kind == MethodKind::Getter && !method.function.params.is_empty() {
                        return Err(format!(
                            "class `{name}` getter `{method_name}` cannot accept parameters"
                        ));
                    }
                    if method.kind == MethodKind::Setter && method.function.params.len() != 1 {
                        return Err(format!(
                            "class `{name}` setter `{method_name}` requires exactly one parameter"
                        ));
                    }
                    let mut params = if method.is_static {
                        Vec::new()
                    } else {
                        vec![instance_type.clone()]
                    };
                    params.extend(
                        method
                            .function
                            .params
                            .iter()
                            .map(|parameter| {
                                lower_param(
                                    &parameter.pat,
                                    &interfaces,
                                    &generic_interfaces,
                                    false,
                                    &HashMap::new(),
                                )
                                .map(|parameter| parameter.ty)
                            })
                            .collect::<Result<Vec<_>, _>>()?,
                    );
                    let ret = if method.kind == MethodKind::Setter {
                        params
                            .last()
                            .expect("a setter has one declared value parameter")
                            .clone()
                    } else {
                        lower_fn_return_type(
                            method.function.is_async,
                            &method.function.return_type,
                            &format!("{name}.{method_name}"),
                            &interfaces,
                            &generic_interfaces,
                            &HashMap::new(),
                        )?
                    };
                    let symbol = match method.kind {
                        MethodKind::Getter => {
                            class_getter_symbol(&name, &method_name, method.is_static)
                        }
                        MethodKind::Setter => {
                            class_setter_symbol(&name, &method_name, method.is_static)
                        }
                        MethodKind::Method if method.is_static => {
                            class_static_method_symbol(&name, &method_name)
                        }
                        MethodKind::Method => class_method_symbol(&name, &method_name),
                    };
                    if signatures.contains_key(&symbol) {
                        return Err(format!(
                            "class `{name}` has duplicate method `{method_name}`"
                        ));
                    }
                    signatures.insert(
                        symbol,
                        FnSignature {
                            params,
                            variadic: None,
                            native_rest: None,
                            abstract_class_constructor: false,
                            ret,
                            is_async: method.function.is_async,
                            uses_this: function_uses_this(&method.function),
                            is_extern: false,
                            source_range: (method.span.lo.0, method.span.hi.0),
                            generic_type_params: Vec::new(),
                            generic_type_constraints: Vec::new(),
                            generic_type_defaults: Vec::new(),
                            generic_param_patterns: Vec::new(),
                            generic_param_optional: Vec::new(),
                            generic_return_type: None,
                        },
                    );
                    let patterns = method
                        .function
                        .params
                        .iter()
                        .map(|parameter| parameter.pat.clone())
                        .collect::<Vec<_>>();
                    let symbol = class_member_symbol(&name, method)?;
                    let native_rest = native_rest_element(&patterns, &signatures[&symbol].params)?;
                    signatures
                        .get_mut(&symbol)
                        .expect("class method signature")
                        .native_rest = native_rest;
                    let unbound_symbol = (method.kind == MethodKind::Method
                        && signatures[&symbol].uses_this)
                        .then(|| unbound_class_method_symbol(&symbol));
                    if let Some(unbound_symbol) = &unbound_symbol {
                        let mut unbound = signatures[&symbol].clone();
                        if !method.is_static {
                            unbound.params.remove(0);
                        }
                        unbound.uses_this = false;
                        signatures.insert(unbound_symbol.clone(), unbound);
                    }
                    if let Some(default_start) = trailing_omittable_start(&patterns) {
                        let receiver_count = usize::from(!method.is_static);
                        for arity in default_start..patterns.len() {
                            let total_arity = arity + receiver_count;
                            let mut wrapper = signatures[&symbol].clone();
                            wrapper.params.truncate(total_arity);
                            signatures.insert(default_arity_symbol(&symbol, total_arity), wrapper);
                        }
                    }
                    insert_omitted_parameter_signatures(
                        &mut signatures,
                        &class_member_symbol(&name, method)?,
                        &patterns,
                        usize::from(!method.is_static),
                    )?;
                    if let Some(unbound_symbol) = unbound_symbol {
                        if let Some(default_start) = trailing_omittable_start(&patterns) {
                            for arity in default_start..patterns.len() {
                                let mut wrapper = signatures[&unbound_symbol].clone();
                                wrapper.params.truncate(arity);
                                signatures
                                    .insert(default_arity_symbol(&unbound_symbol, arity), wrapper);
                            }
                        }
                        insert_omitted_parameter_signatures(
                            &mut signatures,
                            &unbound_symbol,
                            &patterns,
                            0,
                        )?;
                    }
                }
            }
            // Already consumed by `resolve_interfaces` above.
            ModuleItem::Stmt(Stmt::Decl(Decl::TsInterface(_))) => {}
            ModuleItem::Stmt(Stmt::Decl(Decl::TsEnum(_))) => {}
            ModuleItem::Stmt(Stmt::Decl(Decl::TsTypeAlias(_))) => {}
            ModuleItem::Stmt(Stmt::Decl(Decl::Var(var_decl))) => {
                for declaration in &var_decl.decls {
                    let Pat::Ident(binding) = &declaration.name else {
                        return Err(
                            "top-level destructuring declarations are not supported yet".into()
                        );
                    };
                    if declaration.init.is_none() {
                        return Err(format!(
                            "top-level binding `{}` needs an initializer",
                            binding.id.sym
                        ));
                    }
                }
                global_decls.push(var_decl.as_ref());
            }
            ModuleItem::Stmt(_) => {}
            ModuleItem::ModuleDecl(_) => {
                return Err("import/export declarations are not supported yet".into())
            }
        }
    }

    // JavaScript synthesizes `constructor(...args) { super(...args); }` for a
    // derived class without an explicit constructor. Propagate the nearest
    // base signature to a fixed point so forward declarations and multi-level
    // implicit constructor chains receive the same concrete argument tuple.
    for _ in 0..class_decls.len() {
        let mut changed = false;
        for derived in &class_decls {
            if derived
                .class
                .body
                .iter()
                .any(|member| matches!(member, ClassMember::Constructor(_)))
            {
                continue;
            }
            let Some(Expr::Ident(base)) = derived.class.super_class.as_deref() else {
                continue;
            };
            let derived_name = derived.ident.sym.as_ref();
            let base_params = signatures[&class_constructor_symbol(base.sym.as_ref())]
                .params
                .clone();
            let base_native_rest = signatures[&class_constructor_symbol(base.sym.as_ref())]
                .native_rest
                .clone();
            let constructor = signatures
                .get_mut(&class_constructor_symbol(derived_name))
                .expect("derived constructor signature");
            if constructor.params != base_params {
                constructor.params = base_params.clone();
                changed = true;
            }
            constructor.native_rest = base_native_rest.clone();
            let mut initializer_params = vec![interfaces[derived_name].clone()];
            initializer_params.extend(base_params.iter().cloned());
            let initializer = signatures
                .get_mut(&class_initializer_symbol(derived_name))
                .expect("derived initializer signature");
            initializer.params = initializer_params;
            initializer.native_rest = base_native_rest;
            for arity in 0..base_params.len() {
                let base_constructor =
                    default_arity_symbol(&class_constructor_symbol(base.sym.as_ref()), arity);
                if let Some(base_wrapper) = signatures.get(&base_constructor).cloned() {
                    let mut derived_wrapper = base_wrapper;
                    derived_wrapper.ret = interfaces[derived_name].clone();
                    signatures.insert(
                        default_arity_symbol(&class_constructor_symbol(derived_name), arity),
                        derived_wrapper,
                    );
                }
                let base_initializer =
                    default_arity_symbol(&class_initializer_symbol(base.sym.as_ref()), arity + 1);
                if let Some(base_wrapper) = signatures.get(&base_initializer).cloned() {
                    let mut derived_wrapper = base_wrapper;
                    derived_wrapper.params[0] = interfaces[derived_name].clone();
                    derived_wrapper.ret = interfaces[derived_name].clone();
                    signatures.insert(
                        default_arity_symbol(&class_initializer_symbol(derived_name), arity + 1),
                        derived_wrapper,
                    );
                }
            }
            if base_params.len() <= 16 {
                for mask in 1usize..(1usize << base_params.len()) {
                    let base_constructor = omitted_parameter_symbol(
                        &class_constructor_symbol(base.sym.as_ref()),
                        mask,
                    );
                    if let Some(base_wrapper) = signatures.get(&base_constructor).cloned() {
                        let mut derived_wrapper = base_wrapper;
                        derived_wrapper.ret = interfaces[derived_name].clone();
                        signatures.insert(
                            omitted_parameter_symbol(&class_constructor_symbol(derived_name), mask),
                            derived_wrapper,
                        );
                    }
                    let initializer_mask = mask << 1;
                    let base_initializer = omitted_parameter_symbol(
                        &class_initializer_symbol(base.sym.as_ref()),
                        initializer_mask,
                    );
                    if let Some(base_wrapper) = signatures.get(&base_initializer).cloned() {
                        let mut derived_wrapper = base_wrapper;
                        derived_wrapper.params[0] = interfaces[derived_name].clone();
                        derived_wrapper.ret = interfaces[derived_name].clone();
                        signatures.insert(
                            omitted_parameter_symbol(
                                &class_initializer_symbol(derived_name),
                                initializer_mask,
                            ),
                            derived_wrapper,
                        );
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }

    let class_by_name = class_decls
        .iter()
        .map(|declaration| (declaration.ident.sym.to_string(), *declaration))
        .collect::<HashMap<_, _>>();
    let member_key = |member: &swc_ecma_ast::ClassMethod| -> Result<(u8, bool, Symbol), String> {
        Ok((
            match member.kind {
                MethodKind::Method => 0,
                MethodKind::Getter => 1,
                MethodKind::Setter => 2,
            },
            member.is_static,
            class_property_name(&member.key)?,
        ))
    };
    let mut inherited_class_functions = Vec::new();
    let mut inherited_virtual_class_decls = Vec::new();
    for derived in &class_decls {
        let derived_name = derived.ident.sym.to_string();
        let derived_type = interfaces[&derived_name].clone();
        let mut seen = derived
            .class
            .body
            .iter()
            .filter_map(|member| match member {
                ClassMember::Method(method) => member_key(method).ok(),
                _ => None,
            })
            .collect::<HashSet<_>>();
        let mut base_name =
            derived
                .class
                .super_class
                .as_ref()
                .and_then(|base| match base.as_ref() {
                    Expr::Ident(base) => Some(base.sym.to_string()),
                    _ => None,
                });
        while let Some(current_name) = base_name {
            let base = class_by_name[&current_name];
            for member in &base.class.body {
                let ClassMember::Method(method) = member else {
                    continue;
                };
                let key = member_key(method)?;
                let newly_seen = seen.insert(key);
                if method.is_abstract {
                    if !newly_seen {
                        let base_symbol = class_member_symbol(&current_name, method)?;
                        let derived_symbol = class_member_symbol(&derived_name, method)?;
                        let base_signature = &signatures[&base_symbol];
                        let derived_signature = &signatures[&derived_symbol];
                        let receiver_count = usize::from(!method.is_static);
                        if base_signature.params[receiver_count..]
                            != derived_signature.params[receiver_count..]
                            || base_signature.ret != derived_signature.ret
                            || base_signature.is_async != derived_signature.is_async
                            || base_signature.native_rest != derived_signature.native_rest
                        {
                            return Err(format!(
                                "class `{derived_name}` implements abstract member `{}` from `{current_name}` with an incompatible signature",
                                class_property_name(&method.key)?
                            ));
                        }
                        continue;
                    }
                    if !derived.class.is_abstract {
                        return Err(format!(
                            "concrete class `{derived_name}` must implement abstract member `{}` from `{current_name}`",
                            class_property_name(&method.key)?
                        ));
                    }
                    continue;
                }
                if !newly_seen {
                    continue;
                }
                let base_symbol = class_member_symbol(&current_name, method)?;
                let derived_symbol = class_member_symbol(&derived_name, method)?;
                let mut signature = signatures[&base_symbol].clone();
                if !method.is_static {
                    signature.params[0] = derived_type.clone();
                }
                signatures.insert(derived_symbol.clone(), signature.clone());
                if method.kind == MethodKind::Method && signature.uses_this {
                    let base_unbound = unbound_class_method_symbol(&base_symbol);
                    let derived_unbound = unbound_class_method_symbol(&derived_symbol);
                    let mut unbound_signature =
                        signatures.get(&base_unbound).cloned().unwrap_or_else(|| {
                            let mut unbound = signature.clone();
                            if !method.is_static {
                                unbound.params.remove(0);
                            }
                            unbound
                        });
                    unbound_signature.uses_this = false;
                    signatures.insert(derived_unbound, unbound_signature);
                }
                if !derived.class.is_abstract {
                    let mut inherited = (*base).clone();
                    inherited.ident = derived.ident.clone();
                    inherited.class.body = vec![ClassMember::Method(method.clone())];
                    inherited.class.is_abstract = false;
                    inherited.class.implements.clear();
                    inherited_virtual_class_decls.push(inherited);
                }

                let patterns = method
                    .function
                    .params
                    .iter()
                    .map(|parameter| parameter.pat.clone())
                    .collect::<Vec<_>>();
                let receiver_count = usize::from(!method.is_static);
                if let Some(default_start) = trailing_omittable_start(&patterns) {
                    for arity in default_start..patterns.len() {
                        let total_arity = arity + receiver_count;
                        let base_wrapper = default_arity_symbol(&base_symbol, total_arity);
                        let derived_wrapper = default_arity_symbol(&derived_symbol, total_arity);
                        let Some(mut wrapper_signature) = signatures.get(&base_wrapper).cloned()
                        else {
                            continue;
                        };
                        if !method.is_static {
                            wrapper_signature.params[0] = derived_type.clone();
                        }
                        signatures.insert(derived_wrapper.clone(), wrapper_signature.clone());
                    }
                }
                for mask in omitted_parameter_masks(&patterns, receiver_count)? {
                    let base_wrapper = omitted_parameter_symbol(&base_symbol, mask);
                    let derived_wrapper = omitted_parameter_symbol(&derived_symbol, mask);
                    let Some(mut wrapper_signature) = signatures.get(&base_wrapper).cloned() else {
                        continue;
                    };
                    if !method.is_static {
                        wrapper_signature.params[0] = derived_type.clone();
                    }
                    signatures.insert(derived_wrapper.clone(), wrapper_signature.clone());
                }
                if method.kind == MethodKind::Method && signature.uses_this {
                    let base_unbound = unbound_class_method_symbol(&base_symbol);
                    let derived_unbound = unbound_class_method_symbol(&derived_symbol);
                    if let Some(default_start) = trailing_omittable_start(&patterns) {
                        for arity in default_start..patterns.len() {
                            let base_wrapper = default_arity_symbol(&base_unbound, arity);
                            let derived_wrapper = default_arity_symbol(&derived_unbound, arity);
                            if let Some(wrapper) = signatures.get(&base_wrapper).cloned() {
                                signatures.insert(derived_wrapper, wrapper);
                            }
                        }
                    }
                    for mask in omitted_parameter_masks(&patterns, 0)? {
                        let base_wrapper = omitted_parameter_symbol(&base_unbound, mask);
                        let derived_wrapper = omitted_parameter_symbol(&derived_unbound, mask);
                        if let Some(wrapper) = signatures.get(&base_wrapper).cloned() {
                            signatures.insert(derived_wrapper, wrapper);
                        }
                    }
                }
            }
            base_name = base
                .class
                .super_class
                .as_ref()
                .and_then(|parent| match parent.as_ref() {
                    Expr::Ident(parent) => Some(parent.sym.to_string()),
                    _ => None,
                });
        }
    }

    let mut global_types = HashMap::new();
    let mut immutable_globals = HashSet::new();
    for declaration in &global_decls {
        for declarator in &declaration.decls {
            let Pat::Ident(binding) = &declarator.name else {
                unreachable!("top-level patterns were validated above")
            };
            let name = binding.id.sym.to_string();
            if global_types.contains_key(&name) || signatures.contains_key(&name) {
                return Err(format!("duplicate top-level binding `{name}`"));
            }
            let ty = binding
                .type_ann
                .as_ref()
                .map(|annotation| {
                    lower_ts_type(&annotation.type_ann, &interfaces, &generic_interfaces)
                })
                .transpose()?
                .unwrap_or(HirType::Dynamic);
            global_types.insert(name, ty);
            if declaration.kind == swc_ecma_ast::VarDeclKind::Const {
                immutable_globals.insert(binding.id.sym.to_string());
            }
        }
    }
    for declaration in &class_decls {
        let class_name = declaration.ident.sym.as_ref();
        let mut fields = HashSet::new();
        for member in &declaration.class.body {
            let ClassMember::ClassProp(property) = member else {
                continue;
            };
            if !property.is_static {
                continue;
            }
            let field = class_property_name(&property.key)?;
            if property.is_abstract || property.declare {
                return Err(format!(
                    "class `{class_name}` static field `{field}` cannot be abstract or ambient"
                ));
            }
            if !fields.insert(field.clone()) {
                return Err(format!(
                    "class `{class_name}` has duplicate static field `{field}`"
                ));
            }
            for member_symbol in [
                class_static_method_symbol(class_name, &field),
                class_getter_symbol(class_name, &field, true),
                class_setter_symbol(class_name, &field, true),
            ] {
                if signatures.contains_key(&member_symbol) {
                    return Err(format!(
                        "class `{class_name}` static field `{field}` collides with a static method or accessor"
                    ));
                }
            }
            let annotation = property.type_ann.as_ref().ok_or_else(|| {
                format!("class `{class_name}` static field `{field}` needs a type annotation")
            })?;
            let symbol = class_static_field_symbol(class_name, &field);
            let mut ty = lower_ts_type(&annotation.type_ann, &interfaces, &generic_interfaces)?;
            if property.is_optional {
                ty = optional_parameter_type(ty);
            }
            if property.value.is_none() && !matches!(ty, HirType::Optional(_) | HirType::Nullish(_))
            {
                return Err(format!(
                    "class `{class_name}` static field `{field}` without an initializer needs an optional or undefined-capable type"
                ));
            }
            if global_types.insert(symbol.clone(), ty).is_some() {
                return Err(format!(
                    "class `{class_name}` static field `{field}` conflicts with an existing generated binding"
                ));
            }
            if property.readonly {
                immutable_globals.insert(symbol);
            }
        }
    }

    // Inherited static fields share their declaring class's single storage.
    // Derived getters/setters provide the same dispatch surface as inherited
    // static accessors without copying the field into a second global.
    for derived in &class_decls {
        let derived_name = derived.ident.sym.as_ref();
        let mut seen = derived
            .class
            .body
            .iter()
            .filter_map(|member| match member {
                ClassMember::ClassProp(property) if property.is_static => {
                    class_property_name(&property.key).ok()
                }
                ClassMember::Method(method) if method.is_static => {
                    class_property_name(&method.key).ok()
                }
                _ => None,
            })
            .collect::<HashSet<_>>();
        let mut base_name =
            derived
                .class
                .super_class
                .as_ref()
                .and_then(|base| match base.as_ref() {
                    Expr::Ident(base) => Some(base.sym.to_string()),
                    _ => None,
                });
        while let Some(current_name) = base_name {
            let base = class_by_name[&current_name];
            for member in &base.class.body {
                let field = match member {
                    ClassMember::ClassProp(property) if property.is_static => {
                        class_property_name(&property.key)?
                    }
                    ClassMember::Method(method) if method.is_static => {
                        class_property_name(&method.key)?
                    }
                    _ => continue,
                };
                if !seen.insert(field.clone()) {
                    continue;
                }
                let ClassMember::ClassProp(property) = member else {
                    continue;
                };
                let storage = class_static_field_symbol(&current_name, &field);
                let ty = global_types[&storage].clone();
                let getter = class_getter_symbol(derived_name, &field, true);
                let source_range = (property.span.lo.0, property.span.hi.0);
                signatures.insert(
                    getter.clone(),
                    FnSignature {
                        params: Vec::new(),
                        variadic: None,
                        native_rest: None,
                        abstract_class_constructor: false,
                        ret: ty.clone(),
                        is_async: false,
                        uses_this: false,
                        is_extern: false,
                        source_range,
                        generic_type_params: Vec::new(),
                        generic_type_constraints: Vec::new(),
                        generic_type_defaults: Vec::new(),
                        generic_param_patterns: Vec::new(),
                        generic_param_optional: Vec::new(),
                        generic_return_type: None,
                    },
                );
                inherited_class_functions.push(HirFunction {
                    name: getter,
                    params: Vec::new(),
                    ret: ty.clone(),
                    is_async: false,
                    body: vec![HirStmt::Return(Some(HirExpr::Var(storage.clone())))],
                });
                if !property.readonly {
                    let setter = class_setter_symbol(derived_name, &field, true);
                    signatures.insert(
                        setter.clone(),
                        FnSignature {
                            params: vec![ty.clone()],
                            variadic: None,
                            native_rest: None,
                            abstract_class_constructor: false,
                            ret: ty.clone(),
                            is_async: false,
                            uses_this: false,
                            is_extern: false,
                            source_range,
                            generic_type_params: Vec::new(),
                            generic_type_constraints: Vec::new(),
                            generic_type_defaults: Vec::new(),
                            generic_param_patterns: Vec::new(),
                            generic_param_optional: Vec::new(),
                            generic_return_type: None,
                        },
                    );
                    let parameter = "__thaw_inherited_static_value".to_string();
                    inherited_class_functions.push(HirFunction {
                        name: setter,
                        params: vec![HirParam {
                            name: parameter.clone(),
                            ty: ty.clone(),
                        }],
                        ret: ty,
                        is_async: false,
                        body: vec![
                            HirStmt::Expr(HirExpr::Assign(
                                storage,
                                Box::new(HirExpr::Var(parameter.clone())),
                            )),
                            HirStmt::Return(Some(HirExpr::Var(parameter))),
                        ],
                    });
                }
            }
            base_name = base
                .class
                .super_class
                .as_ref()
                .and_then(|parent| match parent.as_ref() {
                    Expr::Ident(parent) => Some(parent.sym.to_string()),
                    _ => None,
                });
        }
    }

    // Missing parameter/return annotations and global initializer types are type
    // variables. Re-lower them together until forward references reach a fixed point.
    // signatures discovered in the previous round until forward calls and
    // mutually recursive functions reach a fixed point.
    for _ in 0..=((fn_decls.len() + global_types.len()) * 2 + 1) {
        let mut changed = false;
        let call_constraints = RefCell::new(Vec::new());
        let globals = lower_global_decls(
            &global_decls,
            &global_types,
            &signatures,
            &interfaces,
            &generic_interfaces,
            &enum_values,
            &enum_reverse_values,
            Some(&call_constraints),
        )?;
        for global in globals {
            let ty = global_types.get_mut(&global.name).unwrap();
            if hir_type_contains_dynamic(ty) && *ty != global.ty {
                *ty = global.ty;
                changed = true;
            }
        }
        let _ = lower_top_level_initializers(
            module,
            &global_types,
            &immutable_globals,
            &signatures,
            &interfaces,
            &generic_interfaces,
            &enum_values,
            &enum_reverse_values,
            Some(&call_constraints),
        )?;
        for fn_decl in &fn_decls {
            let name = fn_decl.ident.sym.to_string();
            let function = lower_fn_decl(
                fn_decl,
                &signatures,
                &interfaces,
                &generic_interfaces,
                &enum_values,
                &enum_reverse_values,
                &global_types,
                &immutable_globals,
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
    if let Some((name, _)) = global_types
        .iter()
        .find(|(_, ty)| hir_type_contains_dynamic(ty))
    {
        return Err(format!(
            "cannot infer the type of top-level binding `{name}`; add an explicit type annotation"
        ));
    }

    let mut globals = lower_global_decls(
        &global_decls,
        &global_types,
        &signatures,
        &interfaces,
        &generic_interfaces,
        &enum_values,
        &enum_reverse_values,
        None,
    )?;
    globals.extend(lower_static_class_globals(
        &class_decls,
        &global_types,
        &immutable_globals,
        &signatures,
        &interfaces,
        &generic_interfaces,
        &enum_values,
        &enum_reverse_values,
    )?);
    let initializers = lower_top_level_initializers(
        module,
        &global_types,
        &immutable_globals,
        &signatures,
        &interfaces,
        &generic_interfaces,
        &enum_values,
        &enum_reverse_values,
        None,
    )?;

    let extern_functions = signatures
        .iter()
        .filter(|(name, sig)| sig.is_extern && dynamic_symbol(name).is_none())
        .map(|(name, sig)| FfiSignature {
            symbol: name.clone(),
            params: sig.params.clone(),
            variadic: sig.variadic.clone(),
            variadic_abi: crate::FfiVariadicAbi::Native,
            ret: sig.ret.clone(),
            error_abi: FfiErrorAbi::Direct,
            return_ownership: FfiOwnership::Borrowed,
            error_ownership: FfiOwnership::Borrowed,
            param_string_abis: vec![FfiStringAbi::NullTerminated; sig.params.len()],
            return_string_abi: FfiStringAbi::NullTerminated,
            calling_convention: FfiCallingConvention::C,
            aggregate_return_abi: FfiAggregateAbi::Internal,
            aggregate_return_layout: None,
        })
        .collect();

    let mut specialized = Vec::new();
    for fn_decl in fn_decls {
        let function = lower_fn_decl(
            fn_decl,
            &signatures,
            &interfaces,
            &generic_interfaces,
            &enum_values,
            &enum_reverse_values,
            &global_types,
            &immutable_globals,
            None,
        )?;
        if !signatures[&function.name].generic_type_params.is_empty() {
            continue;
        }
        let patterns = fn_decl
            .function
            .params
            .iter()
            .map(|parameter| parameter.pat.clone())
            .collect::<Vec<_>>();
        specialized.extend(lower_callable_default_wrappers(
            &function.name,
            &function.params,
            &patterns,
            0,
            None,
            &function.ret,
            &signatures,
            &interfaces,
            &generic_interfaces,
            &enum_values,
            &enum_reverse_values,
            &global_types,
            &immutable_globals,
        )?);
        specialized.extend(lower_callable_omitted_parameter_wrappers(
            &function.name,
            &function.params,
            &patterns,
            0,
            None,
            &function.ret,
            &signatures,
            &interfaces,
            &generic_interfaces,
            &enum_values,
            &enum_reverse_values,
            &global_types,
            &immutable_globals,
        )?);
        specialized.push(function);
    }
    for declaration in &class_decls {
        specialized.extend(lower_class_constructor(
            declaration,
            &signatures,
            &interfaces,
            &generic_interfaces,
            &enum_values,
            &enum_reverse_values,
            &global_types,
            &immutable_globals,
        )?);
    }
    for declaration in class_decls {
        specialized.extend(lower_class_methods(
            declaration,
            &signatures,
            &interfaces,
            &generic_interfaces,
            &enum_values,
            &enum_reverse_values,
            &global_types,
            &immutable_globals,
        )?);
    }
    for declaration in &inherited_virtual_class_decls {
        specialized.extend(lower_class_methods(
            declaration,
            &signatures,
            &interfaces,
            &generic_interfaces,
            &enum_values,
            &enum_reverse_values,
            &global_types,
            &immutable_globals,
        )?);
    }
    specialized.extend(inherited_class_functions);
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
            &enum_values,
            &enum_reverse_values,
            &global_types,
            &immutable_globals,
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
        globals,
        initializers,
        functions: specialized,
        extern_functions,
    })
}

#[allow(clippy::too_many_arguments)]
fn lower_top_level_initializers(
    module: &Module,
    global_types: &HashMap<Symbol, HirType>,
    immutable_globals: &HashSet<Symbol>,
    signatures: &HashMap<Symbol, FnSignature>,
    interfaces: &HashMap<Symbol, HirType>,
    generic_interfaces: &GenericInterfaces,
    enum_values: &EnumValues,
    enum_reverse_values: &EnumReverseValues,
    call_constraints: Option<&RefCell<Vec<CallConstraint>>>,
) -> Result<Vec<HirInitStep>, String> {
    let mut lowerer = FnLowerer::new(
        signatures,
        interfaces,
        generic_interfaces,
        enum_values,
        enum_reverse_values,
        HirType::Void,
        call_constraints,
    );
    for (name, ty) in global_types {
        if is_class_static_field_symbol(name) {
            lowerer.scope.insert(name.clone(), ty.clone());
            lowerer
                .bindings
                .entry(name.clone())
                .or_default()
                .push(name.clone());
            if immutable_globals.contains(name) {
                lowerer.immutable_bindings.insert(name.clone());
            }
        }
    }
    let mut steps = Vec::new();
    for item in &module.body {
        match item {
            ModuleItem::Stmt(Stmt::Decl(Decl::Var(declaration))) => {
                for declarator in &declaration.decls {
                    let Pat::Ident(binding) = &declarator.name else {
                        unreachable!("top-level patterns were validated above")
                    };
                    let name = binding.id.sym.to_string();
                    let expected = global_types[&name].clone();
                    let init = lowerer.lower_expr(
                        declarator
                            .init
                            .as_deref()
                            .expect("top-level initializers were validated above"),
                    )?;
                    let ty = if expected == HirType::Dynamic {
                        lowerer.infer_expr_type(&init)?
                    } else {
                        expected
                    };
                    let init = if ty == HirType::Dynamic {
                        init
                    } else {
                        lowerer.coerce_to_declared(&ty, init)?
                    };
                    steps.push(HirInitStep::StoreGlobal(name.clone(), init));
                    lowerer.scope.insert(name.clone(), ty);
                    lowerer
                        .bindings
                        .entry(name.clone())
                        .or_default()
                        .push(name.clone());
                    if declaration.kind == swc_ecma_ast::VarDeclKind::Const {
                        lowerer.immutable_bindings.insert(name);
                    }
                }
            }
            ModuleItem::Stmt(Stmt::Decl(Decl::Class(declaration))) => {
                let class_name = declaration.ident.sym.as_ref();
                let saved_super = lowerer.super_initializer.clone();
                let saved_static_context = lowerer.class_static_context;
                let saved_class_context = lowerer.class_context.clone();
                lowerer.class_static_context = true;
                lowerer.class_context = Some(class_name.to_string());
                lowerer.super_initializer =
                    declaration
                        .class
                        .super_class
                        .as_ref()
                        .and_then(|base| match base.as_ref() {
                            Expr::Ident(base) => Some((
                                class_initializer_symbol(base.sym.as_ref()),
                                interfaces[base.sym.as_ref()].clone(),
                                base.sym.to_string(),
                            )),
                            _ => None,
                        });
                for member in &declaration.class.body {
                    if let ClassMember::StaticBlock(block) = member {
                        steps.extend(
                            lowerer
                                .lower_stmts(&block.body.stmts)?
                                .into_iter()
                                .map(HirInitStep::Statement),
                        );
                        continue;
                    }
                    let ClassMember::ClassProp(property) = member else {
                        continue;
                    };
                    if !property.is_static {
                        continue;
                    }
                    let field = class_property_name(&property.key)?;
                    let symbol = class_static_field_symbol(class_name, &field);
                    let expected = global_types[&symbol].clone();
                    let init = match property.value.as_deref() {
                        Some(value) => {
                            let init = lowerer.lower_expr(value)?;
                            lowerer.coerce_to_declared(&expected, init)?
                        }
                        None => match &expected {
                            HirType::Optional(payload) => {
                                HirExpr::OptionalNone(payload.as_ref().clone())
                            }
                            HirType::Nullish(payload) => {
                                HirExpr::NullishUndefined(payload.as_ref().clone())
                            }
                            _ => unreachable!("uninitialized static fields were validated above"),
                        },
                    };
                    steps.push(HirInitStep::StoreGlobal(symbol, init));
                }
                lowerer.super_initializer = saved_super;
                lowerer.class_static_context = saved_static_context;
                lowerer.class_context = saved_class_context;
            }
            ModuleItem::Stmt(Stmt::Decl(
                Decl::Fn(_) | Decl::TsInterface(_) | Decl::TsEnum(_) | Decl::TsTypeAlias(_),
            )) => {}
            ModuleItem::Stmt(statement) => {
                steps.extend(
                    lowerer
                        .lower_stmt_seq(statement)?
                        .into_iter()
                        .map(HirInitStep::Statement),
                );
            }
            ModuleItem::ModuleDecl(_) => {
                return Err("import/export declarations are not supported yet".into())
            }
        }
    }
    Ok(steps)
}

#[allow(clippy::too_many_arguments)]
fn lower_global_decls(
    declarations: &[&VarDecl],
    global_types: &HashMap<Symbol, HirType>,
    signatures: &HashMap<Symbol, FnSignature>,
    interfaces: &HashMap<Symbol, HirType>,
    generic_interfaces: &GenericInterfaces,
    enum_values: &EnumValues,
    enum_reverse_values: &EnumReverseValues,
    call_constraints: Option<&RefCell<Vec<CallConstraint>>>,
) -> Result<Vec<crate::HirGlobal>, String> {
    let mut lowerer = FnLowerer::new(
        signatures,
        interfaces,
        generic_interfaces,
        enum_values,
        enum_reverse_values,
        HirType::Void,
        call_constraints,
    );
    for (name, ty) in global_types {
        if is_class_static_field_symbol(name) {
            lowerer.scope.insert(name.clone(), ty.clone());
            lowerer
                .bindings
                .entry(name.clone())
                .or_default()
                .push(name.clone());
        }
    }
    let mut globals = Vec::new();
    for declaration in declarations {
        for declarator in &declaration.decls {
            let Pat::Ident(binding) = &declarator.name else {
                unreachable!("top-level patterns were validated above")
            };
            let name = binding.id.sym.to_string();
            let init = lowerer.lower_expr(
                declarator
                    .init
                    .as_deref()
                    .expect("top-level initializers were validated above"),
            )?;
            let expected = global_types[&name].clone();
            let inferred = lowerer.infer_expr_type(&init)?;
            let ty = if expected == HirType::Dynamic {
                inferred
            } else {
                expected
            };
            let init = if ty == HirType::Dynamic {
                init
            } else {
                lowerer.coerce_to_declared(&ty, init)?
            };
            let scope_type = ty.clone();
            globals.push(crate::HirGlobal {
                name: name.clone(),
                ty,
                init,
                mutable: declaration.kind != swc_ecma_ast::VarDeclKind::Const,
            });
            lowerer.scope.insert(name.clone(), scope_type);
            lowerer
                .bindings
                .entry(name.clone())
                .or_default()
                .push(name.clone());
            if declaration.kind == swc_ecma_ast::VarDeclKind::Const {
                lowerer.immutable_bindings.insert(name);
            }
        }
    }
    Ok(globals)
}

#[allow(clippy::too_many_arguments)]
fn lower_static_class_globals(
    declarations: &[&ClassDecl],
    global_types: &HashMap<Symbol, HirType>,
    immutable_globals: &HashSet<Symbol>,
    signatures: &HashMap<Symbol, FnSignature>,
    interfaces: &HashMap<Symbol, HirType>,
    generic_interfaces: &GenericInterfaces,
    enum_values: &EnumValues,
    enum_reverse_values: &EnumReverseValues,
) -> Result<Vec<crate::HirGlobal>, String> {
    let mut lowerer = FnLowerer::new(
        signatures,
        interfaces,
        generic_interfaces,
        enum_values,
        enum_reverse_values,
        HirType::Void,
        None,
    );
    seed_global_scope(&mut lowerer, global_types, immutable_globals);
    let mut globals = Vec::new();
    for declaration in declarations {
        let class_name = declaration.ident.sym.as_ref();
        let saved_super = lowerer.super_initializer.clone();
        let saved_static_context = lowerer.class_static_context;
        let saved_class_context = lowerer.class_context.clone();
        lowerer.class_static_context = true;
        lowerer.class_context = Some(class_name.to_string());
        lowerer.super_initializer =
            declaration
                .class
                .super_class
                .as_ref()
                .and_then(|base| match base.as_ref() {
                    Expr::Ident(base) => Some((
                        class_initializer_symbol(base.sym.as_ref()),
                        interfaces[base.sym.as_ref()].clone(),
                        base.sym.to_string(),
                    )),
                    _ => None,
                });
        for member in &declaration.class.body {
            let ClassMember::ClassProp(property) = member else {
                continue;
            };
            if !property.is_static {
                continue;
            }
            let field = class_property_name(&property.key)?;
            let symbol = class_static_field_symbol(class_name, &field);
            let ty = global_types[&symbol].clone();
            let init = match property.value.as_deref() {
                Some(value) => {
                    let init = lowerer.lower_expr(value)?;
                    lowerer.coerce_to_declared(&ty, init)?
                }
                None => match &ty {
                    HirType::Optional(payload) => HirExpr::OptionalNone(payload.as_ref().clone()),
                    HirType::Nullish(payload) => {
                        HirExpr::NullishUndefined(payload.as_ref().clone())
                    }
                    _ => unreachable!("uninitialized static fields were validated above"),
                },
            };
            globals.push(crate::HirGlobal {
                name: symbol.clone(),
                ty,
                init,
                mutable: !immutable_globals.contains(&symbol),
            });
        }
        lowerer.super_initializer = saved_super;
        lowerer.class_static_context = saved_static_context;
        lowerer.class_context = saved_class_context;
    }
    Ok(globals)
}

include!("lower/types.rs");

include!("lower/metadata.rs");

include!("lower/interfaces.rs");

include!("lower/declarations.rs");

include!("lower/type_resolution.rs");

include!("lower/control_flow.rs");

/// Lowers one function body while retaining its typed lexical scope.
struct FnLowerer<'a> {
    scope: HashMap<Symbol, HirType>,
    immutable_bindings: HashSet<Symbol>,
    narrowings: HashMap<Symbol, HirType>,
    nullable_narrowings: HashMap<Symbol, HirType>,
    nullish_narrowings: HashMap<Symbol, HirType>,
    union_narrowings: HashMap<Symbol, (Vec<usize>, Vec<HirType>)>,
    union_discriminants: HashMap<Symbol, HashMap<Symbol, Vec<Option<HirLit>>>>,
    array_element_discriminants: HashMap<Symbol, HashMap<Symbol, Vec<Option<HirLit>>>>,
    nested_array_discriminants: HashMap<Symbol, NestedArrayDiscriminants>,
    object_array_property_discriminants: HashMap<Symbol, ObjectArrayPropertyDiscriminants>,
    object_function_property_discriminants: HashMap<Symbol, ObjectFunctionPropertyDiscriminants>,
    destructured_union_correlations: HashMap<Symbol, DestructuredUnionCorrelation>,
    destructuring_default_types: HashMap<Symbol, HirType>,
    function_value_discriminants: HashMap<Symbol, HashMap<Symbol, Vec<Option<HirLit>>>>,
    function_value_array_discriminants: HashMap<Symbol, HashMap<Symbol, Vec<Option<HirLit>>>>,
    function_value_nested_array_discriminants: HashMap<Symbol, NestedArrayDiscriminants>,
    function_value_object_array_property_discriminants:
        HashMap<Symbol, ObjectArrayPropertyDiscriminants>,
    function_value_object_function_property_discriminants:
        HashMap<Symbol, ObjectFunctionPropertyDiscriminants>,
    bindings: HashMap<Symbol, Vec<Symbol>>,
    used_hir_bindings: HashSet<Symbol>,
    next_binding: usize,
    signatures: &'a HashMap<Symbol, FnSignature>,
    interfaces: &'a HashMap<Symbol, HirType>,
    generic_interfaces: &'a GenericInterfaces<'a>,
    enum_values: &'a EnumValues,
    enum_reverse_values: &'a EnumReverseValues,
    ret_type: HirType,
    call_constraints: Option<&'a RefCell<Vec<CallConstraint>>>,
    generic_call_returns: HashMap<Symbol, HirType>,
    generic_arrows: HashMap<Symbol, swc_ecma_ast::ArrowExpr>,
    generic_arrow_self_names: HashMap<Symbol, Symbol>,
    generic_named_templates: HashMap<Symbol, Symbol>,
    native_method_values: HashMap<Symbol, NativeMethodValue>,
    loop_depth: usize,
    labels: Vec<(Symbol, usize, bool)>,
    super_initializer: Option<(Symbol, HirType, Symbol)>,
    class_static_context: bool,
    class_context: Option<Symbol>,
    unbound_this_context: bool,
}

#[derive(Clone)]
struct UnionNarrowingTarget {
    name: Symbol,
    matching: Vec<usize>,
    allowed: Vec<usize>,
    elements: Vec<HirType>,
}

type UnionTypeofNarrowing = (Vec<UnionNarrowingTarget>, bool, bool);

#[derive(Clone)]
struct CorrelatedUnionTarget {
    name: Symbol,
    elements: Vec<HirType>,
    source_members: Vec<Vec<usize>>,
}

#[derive(Clone)]
struct DestructuredUnionCorrelation {
    literals: Vec<Option<HirLit>>,
    targets: Vec<CorrelatedUnionTarget>,
}

struct CorrelatedDestructuredBinding {
    name: Symbol,
    ty: HirType,
    source_types: Vec<Vec<HirType>>,
}

include!("lower/calls.rs");

impl<'a> FnLowerer<'a> {}

include!("lower/statements.rs");

impl<'a> FnLowerer<'a> {}

include!("lower/destructuring.rs");

impl<'a> FnLowerer<'a> {}

include!("lower/inference.rs");

impl<'a> FnLowerer<'a> {
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
            | HirType::Function(_, _)
            | HirType::CallableFunction(..) => Ok(HirExpr::Lit(HirLit::Bool(true))),
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

    fn lower_undefined_default(&mut self, lhs: HirExpr, rhs: HirExpr) -> Result<HirExpr, String> {
        let lhs_type = self.infer_expr_type(&lhs)?;
        if lhs_type == HirType::Undefined {
            return Ok(rhs);
        }
        if let HirType::Union(elements) = &lhs_type {
            if !elements.contains(&HirType::Undefined) {
                return Ok(lhs);
            }
            let rhs_type = self.infer_expr_type(&rhs)?;
            let mut result_members = elements
                .iter()
                .filter(|element| element != &&HirType::Undefined)
                .cloned()
                .collect::<Vec<_>>();
            if !result_members.contains(&rhs_type) {
                result_members.push(rhs_type);
            }
            let result_type = match result_members.as_slice() {
                [member] => member.clone(),
                members => HirType::Union(members.to_vec()),
            };
            let name = format!("__thaw_default_union_left_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(name.clone(), lhs_type.clone());
            let left = HirExpr::Var(name.clone());
            let mut body = Vec::new();
            for (index, element) in elements.iter().enumerate() {
                let value = if element == &HirType::Undefined {
                    self.coerce_to_declared(&result_type, rhs.clone())?
                } else {
                    self.coerce_to_declared(
                        &result_type,
                        HirExpr::UnionValue(Box::new(left.clone()), index, elements.clone()),
                    )?
                };
                if index + 1 == elements.len() {
                    body.push(HirStmt::Return(Some(value)));
                } else {
                    body.push(HirStmt::If(
                        HirExpr::BinOp(
                            BinOp::EqEqEq,
                            Box::new(HirExpr::UnionTag(Box::new(left.clone()), elements.clone())),
                            Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                        ),
                        vec![HirStmt::Return(Some(value))],
                        Vec::new(),
                    ));
                }
            }
            return self
                .wrap_call_argument_bindings(HirExpr::Block(body), &[(name, lhs_type, lhs)]);
        }
        match lhs_type.clone() {
            HirType::Optional(payload) => {
                self.expect_type(payload.as_ref(), &rhs, "destructuring default")?;
                let name = format!("__thaw_default_left_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(name.clone(), lhs_type.clone());
                let left = HirExpr::Var(name.clone());
                let result = HirExpr::Block(vec![HirStmt::If(
                    HirExpr::OptionalIsNone(Box::new(left.clone()), payload.as_ref().clone()),
                    vec![HirStmt::Return(Some(rhs))],
                    vec![HirStmt::Return(Some(HirExpr::OptionalValue(
                        Box::new(left),
                        payload.as_ref().clone(),
                    )))],
                )]);
                self.wrap_call_argument_bindings(result, &[(name, lhs_type, lhs)])
            }
            HirType::Nullish(payload) => {
                self.expect_type(payload.as_ref(), &rhs, "destructuring default")?;
                let result_type = HirType::Nullable(payload.clone());
                let name = format!("__thaw_default_left_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(name.clone(), lhs_type.clone());
                let left = HirExpr::Var(name.clone());
                let result = HirExpr::Block(vec![
                    HirStmt::If(
                        HirExpr::NullishIsUndefined(
                            Box::new(left.clone()),
                            payload.as_ref().clone(),
                        ),
                        vec![HirStmt::Return(Some(HirExpr::NullableSome(
                            Box::new(rhs),
                            payload.as_ref().clone(),
                        )))],
                        Vec::new(),
                    ),
                    HirStmt::If(
                        HirExpr::NullishIsNull(Box::new(left.clone()), payload.as_ref().clone()),
                        vec![HirStmt::Return(Some(HirExpr::NullableNone(
                            payload.as_ref().clone(),
                        )))],
                        Vec::new(),
                    ),
                    HirStmt::Return(Some(HirExpr::NullableSome(
                        Box::new(HirExpr::NullishValue(
                            Box::new(left),
                            payload.as_ref().clone(),
                        )),
                        payload.as_ref().clone(),
                    ))),
                ]);
                let result = self.wrap_call_argument_bindings(result, &[(name, lhs_type, lhs)])?;
                self.expect_type(&result_type, &result, "destructuring default result")?;
                Ok(result)
            }
            _ => Ok(lhs),
        }
    }

    fn lower_nullish_coalescing(&mut self, lhs: HirExpr, rhs: HirExpr) -> Result<HirExpr, String> {
        let lhs_type = self.infer_expr_type(&lhs)?;
        if let HirType::Union(elements) = &lhs_type {
            if !elements
                .iter()
                .any(|element| matches!(element, HirType::Null | HirType::Undefined))
            {
                return Ok(lhs);
            }
            let rhs_type = self.infer_expr_type(&rhs)?;
            let mut result_members = elements
                .iter()
                .filter(|element| !matches!(element, HirType::Null | HirType::Undefined))
                .cloned()
                .collect::<Vec<_>>();
            if !result_members.contains(&rhs_type) {
                result_members.push(rhs_type);
            }
            let result_type = match result_members.as_slice() {
                [member] => member.clone(),
                members => HirType::Union(members.to_vec()),
            };
            let name = format!("__thaw_nullish_union_left_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(name.clone(), lhs_type.clone());
            let left = HirExpr::Var(name.clone());
            let mut body = Vec::new();
            for (index, element) in elements.iter().enumerate() {
                let value = if matches!(element, HirType::Null | HirType::Undefined) {
                    self.coerce_to_declared(&result_type, rhs.clone())?
                } else {
                    self.coerce_to_declared(
                        &result_type,
                        HirExpr::UnionValue(Box::new(left.clone()), index, elements.clone()),
                    )?
                };
                if index + 1 == elements.len() {
                    body.push(HirStmt::Return(Some(value)));
                } else {
                    body.push(HirStmt::If(
                        HirExpr::BinOp(
                            BinOp::EqEqEq,
                            Box::new(HirExpr::UnionTag(Box::new(left.clone()), elements.clone())),
                            Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                        ),
                        vec![HirStmt::Return(Some(value))],
                        Vec::new(),
                    ));
                }
            }
            return self
                .wrap_call_argument_bindings(HirExpr::Block(body), &[(name, lhs_type, lhs)]);
        }
        let (payload, is_none, value) = match lhs_type.clone() {
            HirType::Optional(payload) => {
                let is_none = HirExpr::OptionalIsNone(
                    Box::new(HirExpr::Var(String::new())),
                    payload.as_ref().clone(),
                );
                (payload, is_none, 0)
            }
            HirType::Nullable(payload) => {
                let is_none = HirExpr::NullableIsNone(
                    Box::new(HirExpr::Var(String::new())),
                    payload.as_ref().clone(),
                );
                (payload, is_none, 1)
            }
            HirType::Nullish(payload) => {
                let is_none = HirExpr::NullishIsNone(
                    Box::new(HirExpr::Var(String::new())),
                    payload.as_ref().clone(),
                );
                (payload, is_none, 2)
            }
            _ => return Ok(lhs),
        };
        self.expect_type(payload.as_ref(), &rhs, "nullish fallback")?;
        let name = format!("__thaw_nullish_left_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(name.clone(), lhs_type.clone());
        let left = HirExpr::Var(name.clone());
        let is_none = match is_none {
            HirExpr::OptionalIsNone(_, payload) => {
                HirExpr::OptionalIsNone(Box::new(left.clone()), payload)
            }
            HirExpr::NullableIsNone(_, payload) => {
                HirExpr::NullableIsNone(Box::new(left.clone()), payload)
            }
            HirExpr::NullishIsNone(_, payload) => {
                HirExpr::NullishIsNone(Box::new(left.clone()), payload)
            }
            _ => unreachable!(),
        };
        let present = match value {
            0 => HirExpr::OptionalValue(Box::new(left), payload.as_ref().clone()),
            1 => HirExpr::NullableValue(Box::new(left), payload.as_ref().clone()),
            2 => HirExpr::NullishValue(Box::new(left), payload.as_ref().clone()),
            _ => unreachable!(),
        };
        let result = HirExpr::Block(vec![HirStmt::If(
            is_none,
            vec![HirStmt::Return(Some(rhs))],
            vec![HirStmt::Return(Some(present))],
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
        let nullish_check =
            match (&lhs_type, &rhs_type) {
                (HirType::Optional(payload), HirType::Null)
                | (HirType::Optional(payload), HirType::Undefined) => Some(
                    HirExpr::OptionalIsNone(Box::new(lhs.clone()), payload.as_ref().clone()),
                ),
                (HirType::Null, HirType::Optional(payload))
                | (HirType::Undefined, HirType::Optional(payload)) => Some(
                    HirExpr::OptionalIsNone(Box::new(rhs.clone()), payload.as_ref().clone()),
                ),
                (HirType::Nullable(payload), HirType::Null)
                | (HirType::Nullable(payload), HirType::Undefined) => Some(
                    HirExpr::NullableIsNone(Box::new(lhs.clone()), payload.as_ref().clone()),
                ),
                (HirType::Null, HirType::Nullable(payload))
                | (HirType::Undefined, HirType::Nullable(payload)) => Some(
                    HirExpr::NullableIsNone(Box::new(rhs.clone()), payload.as_ref().clone()),
                ),
                (HirType::Nullish(payload), HirType::Null)
                | (HirType::Nullish(payload), HirType::Undefined) => Some(HirExpr::NullishIsNone(
                    Box::new(lhs.clone()),
                    payload.as_ref().clone(),
                )),
                (HirType::Null, HirType::Nullish(payload))
                | (HirType::Undefined, HirType::Nullish(payload)) => Some(HirExpr::NullishIsNone(
                    Box::new(rhs.clone()),
                    payload.as_ref().clone(),
                )),
                _ => None,
            };
        if let Some(check) = nullish_check {
            return Ok(check);
        }
        if matches!(
            (&lhs_type, &rhs_type),
            (HirType::Null, HirType::Undefined) | (HirType::Undefined, HirType::Null)
        ) {
            let lhs_name = format!("__thaw_loose_nullish_left_{}", self.next_binding);
            self.next_binding += 1;
            let rhs_name = format!("__thaw_loose_nullish_right_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(lhs_name.clone(), lhs_type.clone());
            self.scope.insert(rhs_name.clone(), rhs_type.clone());
            return self.wrap_call_argument_bindings(
                HirExpr::Lit(HirLit::Bool(true)),
                &[(lhs_name, lhs_type, lhs), (rhs_name, rhs_type, rhs)],
            );
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

    fn lower_optional_undefined_equality(
        &self,
        lhs: HirExpr,
        rhs: HirExpr,
    ) -> Result<Option<HirExpr>, String> {
        let lhs_type = self.infer_expr_type(&lhs)?;
        let rhs_type = self.infer_expr_type(&rhs)?;
        let result =
            match (&lhs_type, &rhs_type) {
                (HirType::Optional(payload), HirType::Undefined) => Some(HirExpr::OptionalIsNone(
                    Box::new(lhs),
                    payload.as_ref().clone(),
                )),
                (HirType::Undefined, HirType::Optional(payload)) => Some(HirExpr::OptionalIsNone(
                    Box::new(rhs),
                    payload.as_ref().clone(),
                )),
                (HirType::Nullable(payload), HirType::Null) => Some(HirExpr::NullableIsNone(
                    Box::new(lhs),
                    payload.as_ref().clone(),
                )),
                (HirType::Null, HirType::Nullable(payload)) => Some(HirExpr::NullableIsNone(
                    Box::new(rhs),
                    payload.as_ref().clone(),
                )),
                (HirType::Nullish(payload), HirType::Null) => Some(HirExpr::NullishIsNull(
                    Box::new(lhs),
                    payload.as_ref().clone(),
                )),
                (HirType::Null, HirType::Nullish(payload)) => Some(HirExpr::NullishIsNull(
                    Box::new(rhs),
                    payload.as_ref().clone(),
                )),
                (HirType::Nullish(payload), HirType::Undefined) => Some(
                    HirExpr::NullishIsUndefined(Box::new(lhs), payload.as_ref().clone()),
                ),
                (HirType::Undefined, HirType::Nullish(payload)) => Some(
                    HirExpr::NullishIsUndefined(Box::new(rhs), payload.as_ref().clone()),
                ),
                (HirType::Union(left), HirType::Union(right)) if left == right => Some(
                    HirExpr::UnionIsEqual(Box::new(lhs), Box::new(rhs), left.clone()),
                ),
                (HirType::Union(left), HirType::Union(right))
                    if equivalent_union_members(left, right) =>
                {
                    let rhs = self.coerce_to_declared(&HirType::Union(left.clone()), rhs)?;
                    Some(HirExpr::UnionIsEqual(
                        Box::new(lhs),
                        Box::new(rhs),
                        left.clone(),
                    ))
                }
                (HirType::Union(elements), member) => elements
                    .iter()
                    .position(|element| element == member)
                    .map(|index| {
                        HirExpr::UnionMemberIsEqual(
                            Box::new(lhs),
                            Box::new(rhs),
                            index,
                            elements.clone(),
                        )
                    }),
                (member, HirType::Union(elements)) => elements
                    .iter()
                    .position(|element| element == member)
                    .map(|index| {
                        HirExpr::UnionMemberIsEqual(
                            Box::new(rhs),
                            Box::new(lhs),
                            index,
                            elements.clone(),
                        )
                    }),
                (HirType::Null, HirType::Null) => Some(HirExpr::Lit(HirLit::Bool(true))),
                (HirType::Null, _) | (_, HirType::Null) => Some(HirExpr::Lit(HirLit::Bool(false))),
                (HirType::Undefined, HirType::Undefined) => Some(HirExpr::Lit(HirLit::Bool(true))),
                (HirType::Undefined, _) | (_, HirType::Undefined) => {
                    Some(HirExpr::Lit(HirLit::Bool(false)))
                }
                _ => None,
            };
        Ok(result)
    }

    fn lower_expr(&mut self, expr: &Expr) -> Result<HirExpr, String> {
        match expr {
            Expr::Lit(Lit::Num(n)) => Ok(HirExpr::Lit(HirLit::F64(n.value))),
            Expr::Lit(Lit::Str(s)) => Ok(HirExpr::Lit(HirLit::Str(
                s.value.to_string_lossy().into_owned(),
            ))),
            Expr::Lit(Lit::Bool(b)) => Ok(HirExpr::Lit(HirLit::Bool(b.value))),
            Expr::Lit(Lit::Null(_)) => Ok(HirExpr::Lit(HirLit::Null)),
            Expr::Ident(ident) => {
                let name = self.resolve_binding(ident.sym.as_ref());
                if !self.scope.contains_key(&name) {
                    if let Some(signature) = self.signatures.get(&name) {
                        if signature.is_extern || !signature.generic_type_params.is_empty() {
                            return Err(format!(
                                "function value `{name}` needs a monomorphic native implementation"
                            ));
                        }
                        let ret = if signature.is_async {
                            HirType::Promise(Box::new(signature.ret.clone()))
                        } else {
                            signature.ret.clone()
                        };
                        return Ok(HirExpr::FunctionRef(
                            name,
                            signature.params.clone(),
                            ret,
                        ));
                    }
                }
                if !self.scope.contains_key(&name) && !self.signatures.contains_key(&name) {
                    match ident.sym.as_ref() {
                        "NaN" => return Ok(HirExpr::Lit(HirLit::F64(f64::NAN))),
                        "Infinity" => return Ok(HirExpr::Lit(HirLit::F64(f64::INFINITY))),
                        "undefined" => return Ok(HirExpr::Lit(HirLit::Undefined)),
                        _ => {}
                    }
                }
                if let Some((allowed, elements)) = self.union_narrowings.get(&name) {
                    if let [index] = allowed.as_slice() {
                        return Ok(HirExpr::UnionValue(
                            Box::new(HirExpr::Var(name)),
                            *index,
                            elements.clone(),
                        ));
                    }
                }
                match self
                    .narrowings
                    .get(&name)
                    .map(|payload| (payload, 0))
                    .or_else(|| {
                        self.nullable_narrowings
                            .get(&name)
                            .map(|payload| (payload, 1))
                    })
                    .or_else(|| {
                        self.nullish_narrowings
                            .get(&name)
                            .map(|payload| (payload, 2))
                    })
                {
                    Some((payload, 1)) => Ok(HirExpr::NullableValue(
                        Box::new(HirExpr::Var(name)),
                        payload.clone(),
                    )),
                    Some((payload, 0)) => Ok(HirExpr::OptionalValue(
                        Box::new(HirExpr::Var(name)),
                        payload.clone(),
                    )),
                    Some((payload, 2)) => Ok(HirExpr::NullishValue(
                        Box::new(HirExpr::Var(name)),
                        payload.clone(),
                    )),
                    Some(_) => unreachable!(),
                    None => Ok(HirExpr::Var(name)),
                }
            }

            Expr::This(_) => {
                if self.unbound_this_context {
                    return Ok(HirExpr::Lit(HirLit::Undefined));
                }
                if self.class_static_context {
                    let class = self
                        .class_context
                        .as_deref()
                        .ok_or("static `this` is missing its class context")?;
                    let constructor = class_constructor_symbol(class);
                    let signature = self.signatures.get(&constructor).ok_or_else(|| {
                        format!("static `this` constructor `{constructor}` is not declared")
                    })?;
                    let result = self
                        .interfaces
                        .get(class)
                        .cloned()
                        .ok_or_else(|| format!("static `this` class `{class}` has no layout"))?;
                    return Ok(HirExpr::FunctionRef(
                        constructor,
                        signature.params.clone(),
                        result,
                    ));
                }
                let name = self.resolve_binding("this");
                if self.scope.contains_key(&name) {
                    Ok(HirExpr::Var(name))
                } else {
                    Err("`this` is only available inside a native class constructor or method".into())
                }
            }
            Expr::Paren(paren) => self.lower_expr(&paren.expr),
            Expr::TsAs(assertion) => self.lower_expr(&assertion.expr),
            Expr::TsTypeAssertion(assertion) => self.lower_expr(&assertion.expr),
            Expr::TsSatisfies(satisfies) => self.lower_satisfies(satisfies),
            Expr::TsNonNull(assertion) => self.lower_non_null_assertion(assertion),
            Expr::TsConstAssertion(assertion) => self.lower_expr(&assertion.expr),
            Expr::TsInstantiation(instantiation) => {
                self.lower_generic_instantiation_expression(instantiation)
            }

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
                if bin.op == BinaryOp::InstanceOf {
                    let Expr::Ident(class) = bin.right.as_ref() else {
                        return Err(
                            "native `instanceof` requires a class identifier on the right"
                                .into(),
                        );
                    };
                    if !self
                        .signatures
                        .contains_key(&class_constructor_symbol(class.sym.as_ref()))
                    {
                        return Err(format!(
                            "native `instanceof` right operand `{}` is not a known class",
                            class.sym
                        ));
                    }
                    let value = self.lower_expr(&bin.left)?;
                    let value_type = self.infer_expr_type(&value)?;
                    let result = class_type_has_identity(&value_type, class.sym.as_ref());
                    let name = format!("__thaw_instanceof_value_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(name.clone(), value_type.clone());
                    return self.wrap_call_argument_bindings(
                        HirExpr::Lit(HirLit::Bool(result)),
                        &[(name, value_type, value)],
                    );
                }
                let mut lhs = self.lower_expr(&bin.left)?;
                let rhs_narrowing = self
                    .optional_undefined_narrowing(&bin.left)
                    .filter(|(_, _, present, _)| {
                        (bin.op == BinaryOp::LogicalAnd && *present)
                            || (bin.op == BinaryOp::LogicalOr && !*present)
                    })
                    .map(|(name, payload, _, nullable)| (name, payload, nullable));
                let rhs_union_narrowing = self.union_narrowing(&bin.left).and_then(
                    |(targets, equal, complement)| {
                        let required_truth = match bin.op {
                            BinaryOp::LogicalAnd => true,
                            BinaryOp::LogicalOr => false,
                            _ => return None,
                        };
                        Some(
                            targets
                                .into_iter()
                                .map(|target| UnionNarrowingTarget {
                                    matching: if required_truth == equal {
                                        target.matching.clone()
                                    } else if complement {
                                        target
                                            .allowed
                                            .iter()
                                            .filter(|index| !target.matching.contains(index))
                                            .copied()
                                            .collect()
                                    } else {
                                        target.allowed.clone()
                                    },
                                    ..target
                                })
                                .collect::<Vec<_>>(),
                        )
                    },
                );
                let mut rhs = self
                    .lower_expr_with_union_narrowing(
                        &bin.right,
                        rhs_union_narrowing.as_deref(),
                        rhs_narrowing.as_ref(),
                    )?;
                let mut bindings = Vec::new();
                if !matches!(
                    bin.op,
                    BinaryOp::In
                        | BinaryOp::LogicalAnd
                        | BinaryOp::LogicalOr
                        | BinaryOp::NullishCoalescing
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
                    BinaryOp::NullishCoalescing => self.lower_nullish_coalescing(lhs, rhs)?,
                    BinaryOp::Lt => self.lower_relational(lhs, rhs, BinOp::Lt)?,
                    BinaryOp::Gt => self.lower_relational(lhs, rhs, BinOp::Gt)?,
                    BinaryOp::LtEq => self.lower_relational(lhs, rhs, BinOp::LtEq)?,
                    BinaryOp::GtEq => self.lower_relational(lhs, rhs, BinOp::GtEq)?,
                    BinaryOp::EqEqEq => self
                        .lower_optional_undefined_equality(lhs.clone(), rhs.clone())?
                        .unwrap_or(HirExpr::BinOp(
                            BinOp::EqEqEq,
                            Box::new(lhs),
                            Box::new(rhs),
                        )),
                    BinaryOp::NotEqEq => {
                        let equality = self
                            .lower_optional_undefined_equality(lhs.clone(), rhs.clone())?
                            .unwrap_or(HirExpr::BinOp(
                                BinOp::EqEqEq,
                                Box::new(lhs),
                                Box::new(rhs),
                            ));
                        HirExpr::BinOp(
                            BinOp::EqEqEq,
                            Box::new(equality),
                            Box::new(HirExpr::Lit(HirLit::Bool(false))),
                        )
                    }
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
                            self.generic_arrows
                                .get(name)
                                .map(|arrow| {
                                    HirType::Function(
                                        vec![HirType::Dynamic; arrow.params.len()],
                                        Box::new(HirType::Dynamic),
                                    )
                                })
                                .or_else(|| {
                                    self.generic_named_templates.get(name).and_then(|target| {
                                        self.signatures.get(target).map(|signature| {
                                            HirType::Function(
                                                signature.params.clone(),
                                                Box::new(signature.ret.clone()),
                                            )
                                        })
                                    })
                                })
                                .or_else(|| {
                                    self.signatures.get(name).map(|signature| {
                                        let ret = if signature.is_async {
                                            HirType::Promise(Box::new(signature.ret.clone()))
                                        } else {
                                            signature.ret.clone()
                                        };
                                        HirType::Function(signature.params.clone(), Box::new(ret))
                                    })
                                })
                        } else {
                            None
                        }
                        .map(Ok)
                        .unwrap_or_else(|| self.infer_expr_type(&value))?;
                        if let HirType::Union(elements) = &operand_type {
                            let parameter = format!("__thaw_typeof_union_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(parameter.clone(), operand_type.clone());
                            let bound = HirExpr::Var(parameter.clone());
                            let mut branches = vec![HirStmt::Return(Some(HirExpr::Lit(
                                HirLit::Str(
                                    native_typeof_name(elements.last().ok_or(
                                        "`typeof` cannot inspect an empty union",
                                    )?)
                                    .ok_or_else(|| {
                                        format!(
                                            "`typeof` union member has no runtime category: {:?}",
                                            elements.last().unwrap()
                                        )
                                    })?
                                    .into(),
                                ),
                            )))];
                            for (index, member) in elements
                                .iter()
                                .enumerate()
                                .rev()
                                .skip(1)
                            {
                                let type_name = native_typeof_name(member).ok_or_else(|| {
                                    format!(
                                        "`typeof` union member has no runtime category: {member:?}"
                                    )
                                })?;
                                branches = vec![HirStmt::If(
                                    HirExpr::BinOp(
                                        BinOp::EqEqEq,
                                        Box::new(HirExpr::UnionTag(
                                            Box::new(bound.clone()),
                                            elements.clone(),
                                        )),
                                        Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                                    ),
                                    vec![HirStmt::Return(Some(HirExpr::Lit(HirLit::Str(
                                        type_name.into(),
                                    ))))],
                                    branches,
                                )];
                            }
                            let result = HirExpr::Block(branches);
                            return self.wrap_call_argument_bindings(
                                result,
                                &[(parameter, operand_type, value)],
                            );
                        }
                        if let HirType::Nullish(payload) = &operand_type {
                            let Some(type_name) = native_typeof_name(payload) else {
                                return Err(format!(
                                    "`typeof` nullish payload has no supported runtime category: {payload:?}"
                                ));
                            };
                            let parameter =
                                format!("__thaw_typeof_nullish_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(parameter.clone(), operand_type.clone());
                            let bound = HirExpr::Var(parameter.clone());
                            let result = HirExpr::Block(vec![HirStmt::If(
                                HirExpr::NullishIsUndefined(
                                    Box::new(bound.clone()),
                                    payload.as_ref().clone(),
                                ),
                                vec![HirStmt::Return(Some(HirExpr::Lit(HirLit::Str(
                                    "undefined".into(),
                                ))))],
                                vec![HirStmt::If(
                                    HirExpr::NullishIsNull(
                                        Box::new(bound),
                                        payload.as_ref().clone(),
                                    ),
                                    vec![HirStmt::Return(Some(HirExpr::Lit(HirLit::Str(
                                        "object".into(),
                                    ))))],
                                    vec![HirStmt::Return(Some(HirExpr::Lit(HirLit::Str(
                                        type_name.into(),
                                    ))))],
                                )],
                            )]);
                            return self.wrap_call_argument_bindings(
                                result,
                                &[(parameter, operand_type, value)],
                            );
                        }
                        if let Some((payload, absent, nullable)) = match &operand_type {
                            HirType::Optional(payload) => {
                                Some((payload.as_ref(), "undefined", false))
                            }
                            HirType::Nullable(payload) => {
                                Some((payload.as_ref(), "object", true))
                            }
                            _ => None,
                        } {
                            let Some(type_name) = native_typeof_name(payload) else {
                                return Err(format!(
                                    "`typeof` optional payload has no supported runtime category: {payload:?}"
                                ));
                            };
                            let parameter = format!("__thaw_typeof_optional_{}", self.next_binding);
                            self.next_binding += 1;
                            self.scope.insert(parameter.clone(), operand_type.clone());
                            let bound = HirExpr::Var(parameter.clone());
                            let is_none = if nullable {
                                HirExpr::NullableIsNone(Box::new(bound), payload.clone())
                            } else {
                                HirExpr::OptionalIsNone(Box::new(bound), payload.clone())
                            };
                            let result = HirExpr::Block(vec![HirStmt::If(
                                is_none,
                                vec![HirStmt::Return(Some(HirExpr::Lit(HirLit::Str(
                                    absent.into(),
                                ))))],
                                vec![HirStmt::Return(Some(HirExpr::Lit(HirLit::Str(
                                    type_name.into(),
                                ))))],
                            )]);
                            self.wrap_call_argument_bindings(
                                result,
                                &[(parameter, operand_type, value)],
                            )?
                        } else {
                        let Some(type_name) = native_typeof_name(&operand_type) else {
                            return Err(format!(
                                "`typeof` requires one statically known runtime category, got {operand_type:?}"
                            ));
                        };
                        if matches!(&value, HirExpr::Var(_) | HirExpr::FunctionRef(..))
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
                let undefined_default_binding = if let Expr::Bin(test) = conditional.test.as_ref() {
                    if test.op == BinaryOp::EqEqEq && !self.scope.contains_key("undefined") {
                        match (test.left.as_ref(), test.right.as_ref(), conditional.alt.as_ref()) {
                            (Expr::Ident(value), Expr::Ident(undefined), Expr::Ident(alternate))
                                if undefined.sym == *"undefined"
                                    && value.sym == alternate.sym =>
                            {
                                Some(alternate)
                            }
                            (Expr::Ident(undefined), Expr::Ident(value), Expr::Ident(alternate))
                                if undefined.sym == *"undefined"
                                    && value.sym == alternate.sym =>
                            {
                                Some(alternate)
                            }
                            _ => None,
                        }
                    } else {
                        None
                    }
                } else {
                    None
                };
                if let Some(binding) = undefined_default_binding {
                    let lhs = self.lower_expr(&Expr::Ident(binding.clone()))?;
                    let rhs = self.lower_expr(&conditional.cons)?;
                    return self.lower_undefined_default(lhs, rhs);
                }
                let test = self.lower_expr(&conditional.test)?;
                self.expect_type(&HirType::Bool, &test, "conditional expression test")?;
                let mut consequent = self.lower_expr(&conditional.cons)?;
                let mut alternate = self.lower_expr(&conditional.alt)?;
                let consequent_type = self.infer_expr_type(&consequent)?;
                let alternate_type = self.infer_expr_type(&alternate)?;
                let result_type = if consequent_type == alternate_type {
                    consequent_type
                } else if !matches!(consequent_type, HirType::Union(_))
                    && !matches!(alternate_type, HirType::Union(_))
                {
                    let result = match (&consequent_type, &alternate_type) {
                        (HirType::Undefined, payload) | (payload, HirType::Undefined) => {
                            HirType::Optional(Box::new(payload.clone()))
                        }
                        (HirType::Null, payload) | (payload, HirType::Null) => {
                            HirType::Nullable(Box::new(payload.clone()))
                        }
                        _ => HirType::Union(vec![consequent_type, alternate_type]),
                    };
                    consequent = self.coerce_to_declared(&result, consequent)?;
                    alternate = self.coerce_to_declared(&result, alternate)?;
                    result
                } else {
                    return Err(format!(
                        "conditional expression branches have incompatible types {consequent_type:?} and {alternate_type:?}"
                    ));
                };
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
                        result_type,
                        Box::new(body),
                    )),
                    Vec::new(),
                ))
            }

            Expr::Call(call) => self.lower_call(call),

            Expr::OptChain(chain) => match chain.base.as_ref() {
                OptChainBase::Member(member) => self.lower_optional_member_read(member),
                OptChainBase::Call(call) => self.lower_optional_call(call),
            },

            Expr::Arrow(arrow) => self.lower_arrow(arrow),
            Expr::Fn(function) => {
                let arrow = function_expression_as_arrow(function)?;
                self.lower_arrow(&arrow)
            }

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
                    let mut value = self.lower_expr(&element.expr)?;
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
                                if matches!(expected, HirType::Union(members) if members.contains(&actual))
                                {
                                    value = self.coerce_to_declared(expected, value)?;
                                } else {
                                    return Err(format!(
                                        "array element type {actual:?} does not match {expected:?}"
                                    ));
                                }
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

            Expr::SuperProp(member) => {
                let (_, _, base_name) = self
                    .super_initializer
                    .clone()
                    .ok_or("`super` property access is only valid in a derived class")?;
                let property = super_property_name(&member.prop)?;
                if self.class_static_context {
                    let storage = class_static_field_symbol(&base_name, &property);
                    if self.scope.contains_key(&storage) {
                        return Ok(HirExpr::Var(storage));
                    }
                }
                let symbol = class_getter_symbol(
                    &base_name,
                    &property,
                    self.class_static_context,
                );
                if !self.signatures.contains_key(&symbol) {
                    return Err(format!(
                        "base class `{base_name}` has no getter `{property}`"
                    ));
                }
                Ok(HirExpr::Call(
                    Box::new(HirExpr::Var(symbol)),
                    if self.class_static_context {
                        Vec::new()
                    } else {
                        vec![HirExpr::Var(self.resolve_binding("this"))]
                    },
                ))
            }

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

            Expr::New(new_expr) => {
                if let Expr::Ident(class) = new_expr.callee.as_ref() {
                    let constructor = class_constructor_symbol(class.sym.as_ref());
                    if let Some(signature) = self.signatures.get(&constructor) {
                        if signature.abstract_class_constructor {
                            return Err(format!(
                                "cannot construct abstract class `{}`",
                                class.sym
                            ));
                        }
                        let mut callee = class.clone();
                        callee.sym = constructor.into();
                        return self.lower_call(&CallExpr {
                            span: new_expr.span,
                            ctxt: new_expr.ctxt,
                            callee: Callee::Expr(Box::new(Expr::Ident(callee))),
                            args: new_expr.args.clone().unwrap_or_default(),
                            type_args: new_expr.type_args.clone(),
                        });
                    }
                }
                self.lower_promise_new(new_expr)
            }

            other => Err(format!(
                "unsupported expression {other:?} (Phase 0/1/2 support literals, identifiers, binary ops, calls, arrays, objects, member access, assignment, ++/--)"
            )),
        }
    }

    fn lower_satisfies(
        &mut self,
        satisfies: &swc_ecma_ast::TsSatisfiesExpr,
    ) -> Result<HirExpr, String> {
        let value = self.lower_expr(&satisfies.expr)?;
        let expected = lower_ts_type(
            &satisfies.type_ann,
            self.interfaces,
            self.generic_interfaces,
        )?;
        self.coerce_to_declared(&expected, value.clone())?;
        Ok(value)
    }

    fn lower_non_null_assertion(
        &mut self,
        assertion: &swc_ecma_ast::TsNonNullExpr,
    ) -> Result<HirExpr, String> {
        let value = self.lower_expr(&assertion.expr)?;
        match self.infer_expr_type(&value)? {
            HirType::Optional(payload) => Ok(HirExpr::OptionalValue(
                Box::new(value),
                payload.as_ref().clone(),
            )),
            HirType::Nullable(payload) => Ok(HirExpr::NullableValue(
                Box::new(value),
                payload.as_ref().clone(),
            )),
            HirType::Nullish(payload) => Ok(HirExpr::NullishValue(
                Box::new(value),
                payload.as_ref().clone(),
            )),
            HirType::Null | HirType::Undefined => Err(
                "non-null assertion cannot produce a native value from a statically null or undefined expression"
                    .into(),
            ),
            _ => Ok(value),
        }
    }

    fn lower_recursive_function_expression(
        &mut self,
        outer_name: &str,
        expression: &swc_ecma_ast::FnExpr,
        annotated: Option<&HirType>,
    ) -> Result<Option<(Symbol, HirType, HirExpr)>, String> {
        if !named_function_is_recursive(expression) {
            return Ok(None);
        }
        if expression.function.type_params.is_some() {
            return Ok(None);
        }
        let internal = expression
            .ident
            .as_ref()
            .expect("recursive named function expression")
            .sym
            .to_string();
        let arrow = function_expression_as_arrow(expression)?;
        let ty = if let Some(annotated) = annotated {
            annotated.clone()
        } else {
            let params = arrow
                .params
                .iter()
                .map(|parameter| {
                    lower_param(
                        parameter,
                        self.interfaces,
                        self.generic_interfaces,
                        false,
                        &HashMap::new(),
                    )
                    .map(|parameter| parameter.ty)
                })
                .collect::<Result<Vec<_>, _>>()?;
            let result = arrow
                .return_type
                .as_ref()
                .ok_or_else(|| "recursive local function values need a return annotation or an outer function type".to_string())
                .and_then(|annotation| {
                    lower_ts_type(
                        &annotation.type_ann,
                        self.interfaces,
                        self.generic_interfaces,
                    )
                })?;
            HirType::Function(params, Box::new(result))
        };
        let HirType::Function(params, ret) = &ty else {
            return Err("recursive named function expression needs a function type".into());
        };
        let hir_name = self.bind_local(outer_name, ty.clone());
        let saved_internal = (internal != outer_name)
            .then(|| self.bindings.get(&internal).cloned())
            .flatten();
        if internal != outer_name {
            self.bindings
                .entry(internal.clone())
                .or_default()
                .push(hir_name.clone());
        }
        let lowered = self.lower_contextual_arrow(&arrow, params, Some(ret));
        if internal != outer_name {
            if let Some(saved) = saved_internal {
                self.bindings.insert(internal, saved);
            } else {
                self.bindings.remove(&internal);
            }
        }
        let closure = self.coerce_to_declared(&ty, lowered?)?;
        Ok(Some((
            hir_name.clone(),
            ty.clone(),
            HirExpr::RecursiveClosure(hir_name, ty, Box::new(closure)),
        )))
    }

    fn lower_arrow(&mut self, arrow: &swc_ecma_ast::ArrowExpr) -> Result<HirExpr, String> {
        if arrow.is_generator || arrow.type_params.is_some() {
            return Err("generator and generic arrow functions are not supported yet".into());
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
                if !matches!(pattern, Pat::Ident(_) | Pat::Rest(_)) {
                    destructuring.push((pattern, name.clone(), param.ty.clone()));
                }
                params.push(HirParam { name, ty: param.ty });
            }
            let mut prefix = Vec::new();
            for (pattern, name, ty) in destructuring {
                self.lower_binding_pattern(pattern, HirExpr::Var(name), &ty, &mut prefix)?;
            }
            let declared_async_result = if arrow.is_async {
                match &declared_return {
                    Some(HirType::Promise(result)) => Some(result.as_ref().clone()),
                    Some(other) => {
                        return Err(format!(
                            "async arrow return annotation must be Promise<T>, got {other:?}"
                        ))
                    }
                    None => None,
                }
            } else {
                None
            };
            self.ret_type = declared_async_result
                .clone()
                .or_else(|| declared_return.clone())
                .unwrap_or(HirType::Dynamic);
            let (body, inferred_return) = match arrow.body.as_ref() {
                ArrowFunctionBody::Expr(expr) => {
                    let mut expression = self.lower_expr(expr)?;
                    let mut inferred = self.infer_expr_type(&expression)?;
                    if arrow.is_async {
                        if let HirExpr::AwaitPromise(promise, resolved) = expression {
                            expression = *promise;
                            inferred = HirType::Promise(Box::new(resolved));
                        } else if let HirExpr::Await(promise) = expression {
                            expression = *promise;
                            inferred = HirType::Promise(Box::new(inferred));
                        }
                        let resolved = declared_async_result.clone().unwrap_or_else(|| {
                            if let HirType::Promise(inner) = &inferred {
                                inner.as_ref().clone()
                            } else {
                                inferred.clone()
                            }
                        });
                        if contains_await(&expression) {
                            expression = self.coerce_to_declared(&resolved, expression)?;
                            inferred = HirType::Promise(Box::new(resolved));
                        } else {
                            let assimilates = matches!(
                                &inferred,
                                HirType::Promise(inner) if inner.as_ref() == &resolved
                            );
                            if !assimilates {
                                expression = self.coerce_to_declared(&resolved, expression)?;
                            }
                            let resolve_type = HirType::Function(
                                vec![if assimilates {
                                    HirType::Promise(Box::new(resolved.clone()))
                                } else {
                                    resolved.clone()
                                }],
                                Box::new(HirType::Void),
                            );
                            let resolve_name =
                                format!("__thaw_async_arrow_resolve_{}", self.next_binding);
                            self.next_binding += 1;
                            let mut referenced = BTreeSet::new();
                            collect_referenced_bindings(&expression, &mut referenced);
                            let executor_captures = referenced
                                .into_iter()
                                .filter_map(|name| {
                                    self.scope
                                        .get(&name)
                                        .cloned()
                                        .map(|ty| HirParam { name, ty })
                                })
                                .collect();
                            let executor = HirExpr::Lambda(
                                executor_captures,
                                vec![HirParam {
                                    name: resolve_name.clone(),
                                    ty: resolve_type,
                                }],
                                HirType::Void,
                                Box::new(HirExpr::Call(
                                    Box::new(HirExpr::Var(resolve_name)),
                                    vec![expression],
                                )),
                            );
                            expression = HirExpr::PromiseNew(
                                Box::new(executor),
                                resolved.clone(),
                                assimilates,
                            );
                            inferred = HirType::Promise(Box::new(resolved));
                        }
                    } else if let Some(expected) = &declared_return {
                        expression = self.coerce_to_declared(expected, expression)?;
                        inferred = self.infer_expr_type(&expression)?;
                    }
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
                    if arrow.is_async {
                        let has_await = stmts.iter().any(stmt_contains_await);
                        let only_tail_awaits =
                            has_await && async_arrow_has_only_tail_await_returns(&stmts);
                        let resolved = declared_async_result.clone().unwrap_or(inferred);
                        if has_await && !only_tail_awaits {
                            (HirExpr::Block(stmts), HirType::Promise(Box::new(resolved)))
                        } else {
                            let assimilates = if only_tail_awaits {
                                stmts = strip_async_arrow_tail_awaits(stmts);
                                true
                            } else {
                                false
                            };
                            let resolve_type = HirType::Function(
                                if assimilates {
                                    vec![HirType::Promise(Box::new(resolved.clone()))]
                                } else if resolved == HirType::Void {
                                    Vec::new()
                                } else {
                                    vec![resolved.clone()]
                                },
                                Box::new(HirType::Void),
                            );
                            let resolve_name =
                                format!("__thaw_async_arrow_resolve_{}", self.next_binding);
                            self.next_binding += 1;
                            let mut executor_body =
                                rewrite_async_arrow_returns(stmts, &resolve_name, &resolved)?;
                            if resolved == HirType::Void && !assimilates {
                                executor_body.push(HirStmt::Expr(HirExpr::Call(
                                    Box::new(HirExpr::Var(resolve_name.clone())),
                                    Vec::new(),
                                )));
                            }
                            let executor_body = HirExpr::Block(executor_body);
                            let mut referenced = BTreeSet::new();
                            collect_referenced_bindings(&executor_body, &mut referenced);
                            let executor_captures = referenced
                                .into_iter()
                                .filter(|name| name != &resolve_name)
                                .filter_map(|name| {
                                    self.scope
                                        .get(&name)
                                        .cloned()
                                        .map(|ty| HirParam { name, ty })
                                })
                                .collect();
                            let executor = HirExpr::Lambda(
                                executor_captures,
                                vec![HirParam {
                                    name: resolve_name,
                                    ty: resolve_type,
                                }],
                                HirType::Void,
                                Box::new(executor_body),
                            );
                            (
                                HirExpr::PromiseNew(
                                    Box::new(executor),
                                    resolved.clone(),
                                    assimilates,
                                ),
                                HirType::Promise(Box::new(resolved)),
                            )
                        }
                    } else {
                        (HirExpr::Block(stmts), inferred)
                    }
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

    fn lower_generic_instantiation_expression(
        &mut self,
        instantiation: &swc_ecma_ast::TsInstantiation,
    ) -> Result<HirExpr, String> {
        let Expr::Ident(identifier) = instantiation.expr.as_ref() else {
            return Err(
                "generic instantiation expressions require a named top-level function".into(),
            );
        };
        let name = identifier.sym.to_string();
        let signature = self
            .signatures
            .get(&name)
            .ok_or_else(|| format!("unknown function `{name}` in instantiation expression"))?;
        if signature.generic_type_params.is_empty() {
            return Err(format!(
                "non-generic function `{name}` cannot be used in an instantiation expression"
            ));
        }
        let types = resolve_explicit_generic_type_tuple(
            signature,
            &instantiation.type_args.params,
            &[],
            self.interfaces,
            self.generic_interfaces,
        )?;
        for ty in &types {
            if !supports_generic_native_layout(ty) {
                return Err(format!(
                    "generic function `{name}` cannot specialize for native layout {ty:?}"
                ));
            }
        }
        let substitution = signature
            .generic_type_params
            .iter()
            .cloned()
            .zip(types.iter().cloned())
            .collect::<HashMap<_, _>>();
        let params = signature
            .generic_param_patterns
            .iter()
            .map(|pattern| instantiate_generic_pattern(pattern, &substitution))
            .collect::<Result<Vec<_>, _>>()?;
        let mut ret = resolve_ts_type_with_substitution(
            signature
                .generic_return_type
                .as_ref()
                .expect("generic instantiation return type"),
            &substitution,
            self.interfaces,
            self.generic_interfaces,
            &mut Vec::new(),
        )?;
        if signature.is_async && !matches!(ret, HirType::Promise(_)) {
            ret = HirType::Promise(Box::new(ret));
        }
        if let Some(constraints) = self.call_constraints {
            constraints
                .borrow_mut()
                .push(CallConstraint::Generic(name.clone(), types.clone()));
        }
        let specialized = specialized_generic_function_name(&name, &params, signature, &types);
        Ok(HirExpr::FunctionRef(specialized, params, ret))
    }

    fn lower_stored_generic_arrow(
        &mut self,
        name: &str,
        arrow: &swc_ecma_ast::ArrowExpr,
        parameter_types: &[HirType],
        expected_return: Option<&HirType>,
    ) -> Result<HirExpr, String> {
        let Some(internal) = self.generic_arrow_self_names.get(name).cloned() else {
            return self.lower_contextual_arrow(arrow, parameter_types, expected_return);
        };
        let signature = self.generic_arrow_signature(arrow)?;
        let concrete_types = infer_generic_type_tuple(
            &signature,
            parameter_types,
            self.interfaces,
            self.generic_interfaces,
        )?;
        let return_type =
            if let Some(expected) = expected_return.filter(|ty| **ty != HirType::Dynamic) {
                expected.clone()
            } else {
                let declared = signature.generic_return_type.as_ref().ok_or_else(|| {
                    format!("recursive generic local function `{name}` needs a return annotation")
                })?;
                let substitution = signature
                    .generic_type_params
                    .iter()
                    .cloned()
                    .zip(concrete_types)
                    .collect::<HashMap<_, _>>();
                resolve_ts_type_with_substitution(
                    declared,
                    &substitution,
                    self.interfaces,
                    self.generic_interfaces,
                    &mut Vec::new(),
                )?
            };
        let self_type = HirType::Function(parameter_types.to_vec(), Box::new(return_type));
        let self_name = format!("__thaw_recursive_generic_{}", self.next_binding);
        self.next_binding += 1;
        let saved_binding = self.bindings.get(&internal).cloned();
        self.scope.insert(self_name.clone(), self_type.clone());
        self.bindings
            .entry(internal.clone())
            .or_default()
            .push(self_name.clone());
        let lowered = self.lower_contextual_arrow(arrow, parameter_types, expected_return);
        self.scope.remove(&self_name);
        if let Some(saved) = saved_binding {
            self.bindings.insert(internal.clone(), saved);
        } else {
            self.bindings.remove(&internal);
        }
        lowered.map(|closure| HirExpr::RecursiveClosure(self_name, self_type, Box::new(closure)))
    }

    fn lower_contextual_arrow(
        &mut self,
        arrow: &swc_ecma_ast::ArrowExpr,
        parameter_types: &[HirType],
        expected_return: Option<&HirType>,
    ) -> Result<HirExpr, String> {
        if arrow.params.len() != parameter_types.len() {
            return Err(format!(
                "Promise callback expects {} parameter(s), got {}",
                parameter_types.len(),
                arrow.params.len()
            ));
        }
        if arrow.is_async {
            if arrow.is_generator {
                return Err("async generator arrow functions are not supported".into());
            }
            let mut contextual = arrow.clone();
            for (parameter, ty) in contextual.params.iter_mut().zip(parameter_types) {
                let annotation = match parameter {
                    Pat::Ident(binding) => &mut binding.type_ann,
                    Pat::Rest(rest) => &mut rest.type_ann,
                    _ => return Err(
                        "contextual async arrows currently require identifier or rest parameters"
                            .into(),
                    ),
                };
                if annotation.is_none() {
                    *annotation = Some(Box::new(swc_ecma_ast::TsTypeAnn {
                        span: swc_common::DUMMY_SP,
                        type_ann: Box::new(hir_type_as_ts_type(ty)?),
                    }));
                }
            }
            if contextual.return_type.is_none() {
                if let Some(expected) = expected_return {
                    contextual.return_type = Some(Box::new(swc_ecma_ast::TsTypeAnn {
                        span: swc_common::DUMMY_SP,
                        type_ann: Box::new(hir_type_as_ts_type(expected)?),
                    }));
                }
            }
            return self.lower_arrow(&contextual);
        }
        if arrow.is_generator {
            return Err("generator Promise callbacks are not supported".into());
        }
        let generic_return = if let Some(type_params) = &arrow.type_params {
            validate_trailing_type_parameter_defaults(
                "generic arrow function",
                "<anonymous>",
                type_params,
            )?;
            let generic_type_params = type_params
                .params
                .iter()
                .map(|parameter| parameter.name.sym.to_string())
                .collect::<Vec<_>>();
            let substitutions = generic_type_params
                .iter()
                .map(|name| (name.clone(), GenericTypePattern::Variable(name.clone())))
                .collect::<HashMap<_, _>>();
            let generic_param_patterns = arrow
                .params
                .iter()
                .map(|parameter| {
                    let Pat::Ident(binding) = parameter else {
                        return Err(
                            "generic contextual arrows require identifier parameters".into()
                        );
                    };
                    let annotation = binding
                        .type_ann
                        .as_ref()
                        .ok_or("generic contextual arrow parameters need type annotations")?;
                    generic_type_pattern(
                        &annotation.type_ann,
                        &substitutions,
                        self.interfaces,
                        self.generic_interfaces,
                        &mut Vec::new(),
                    )
                })
                .collect::<Result<Vec<_>, String>>()?;
            let signature = FnSignature {
                params: Vec::new(),
                variadic: None,
                native_rest: None,
                abstract_class_constructor: false,
                ret: HirType::Dynamic,
                is_async: false,
                uses_this: false,
                is_extern: false,
                source_range: (arrow.span.lo.0, arrow.span.hi.0),
                generic_type_params,
                generic_type_constraints: type_params
                    .params
                    .iter()
                    .map(|parameter| parameter.constraint.clone())
                    .collect(),
                generic_type_defaults: type_params
                    .params
                    .iter()
                    .map(|parameter| parameter.default.clone())
                    .collect(),
                generic_param_patterns,
                generic_param_optional: arrow
                    .params
                    .iter()
                    .map(
                        |parameter| matches!(parameter, Pat::Ident(binding) if binding.id.optional),
                    )
                    .collect(),
                generic_return_type: arrow
                    .return_type
                    .as_ref()
                    .map(|annotation| annotation.type_ann.clone()),
            };
            let types = infer_generic_type_tuple(
                &signature,
                parameter_types,
                self.interfaces,
                self.generic_interfaces,
            )?;
            let substitution = signature
                .generic_type_params
                .iter()
                .cloned()
                .zip(types)
                .collect::<HashMap<_, _>>();
            arrow
                .return_type
                .as_ref()
                .map(|annotation| {
                    resolve_ts_type_with_substitution(
                        &annotation.type_ann,
                        &substitution,
                        self.interfaces,
                        self.generic_interfaces,
                        &mut Vec::new(),
                    )
                })
                .transpose()?
        } else {
            None
        };
        if let (Some(declared), Some(contextual)) = (&generic_return, expected_return) {
            if contextual != &HirType::Dynamic && declared != contextual {
                return Err(format!(
                    "generic callback declares return type {declared:?}, expected {contextual:?}"
                ));
            }
        }
        let expected_return = generic_return.as_ref().or(expected_return);
        let saved_scope = self.scope.clone();
        let saved_bindings = self.bindings.clone();
        let saved_return = self.ret_type.clone();
        let result = (|| {
            let mut params = Vec::with_capacity(parameter_types.len());
            let mut destructuring = Vec::new();
            for (pat, ty) in arrow.params.iter().zip(parameter_types) {
                let source_name = match pat {
                    Pat::Ident(binding) => binding.id.sym.to_string(),
                    Pat::Assign(assignment) => match assignment.left.as_ref() {
                        Pat::Ident(binding) => binding.id.sym.to_string(),
                        _ => {
                            return Err(
                                "default callback parameters require an identifier binding".into()
                            )
                        }
                    },
                    Pat::Rest(rest) => match rest.arg.as_ref() {
                        Pat::Ident(binding) => binding.id.sym.to_string(),
                        _ => {
                            return Err(
                                "rest callback parameters require an identifier binding".into()
                            )
                        }
                    },
                    Pat::Object(pattern) => format!("__thaw_param_{}", pattern.span.lo.0),
                    Pat::Array(pattern) => format!("__thaw_param_{}", pattern.span.lo.0),
                    _ => return Err("unsupported Promise callback parameter pattern".into()),
                };
                let name = self.bind_local(&source_name, ty.clone());
                if !matches!(pat, Pat::Ident(_) | Pat::Rest(_)) {
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
            Expr::Fn(function) => {
                let arrow = function_expression_as_arrow(function)?;
                return self.lower_contextual_arrow(&arrow, parameter_types, expected_return);
            }
            Expr::Ident(ident) => {
                let mut name = self.resolve_binding(ident.sym.as_ref());
                if let Some(arrow) = self.generic_arrows.get(&name).cloned() {
                    return self.lower_stored_generic_arrow(
                        &name,
                        &arrow,
                        parameter_types,
                        expected_return,
                    );
                }
                if let Some(target) = self.generic_named_templates.get(&name) {
                    name = target.clone();
                }
                if self.scope.contains_key(&name) {
                    self.lower_expr(expr)?
                } else {
                    let signature = self
                        .signatures
                        .get(&name)
                        .ok_or_else(|| format!("unknown Promise callback `{name}`"))?;
                    if !signature.generic_type_params.is_empty() {
                        if signature.params.len() != parameter_types.len() {
                            return Err(format!(
                                "generic callback `{name}` expects {} argument(s), contextual call provides {}",
                                signature.params.len(),
                                parameter_types.len()
                            ));
                        }
                        let types = infer_generic_type_tuple(
                            signature,
                            parameter_types,
                            self.interfaces,
                            self.generic_interfaces,
                        )
                        .map_err(|error| {
                            format!("cannot specialize generic callback `{name}`: {error}")
                        })?;
                        if let Some(constraints) = self.call_constraints {
                            constraints
                                .borrow_mut()
                                .push(CallConstraint::Generic(name.clone(), types.clone()));
                        }
                        let substitution = signature
                            .generic_type_params
                            .iter()
                            .cloned()
                            .zip(types.iter().cloned())
                            .collect::<HashMap<_, _>>();
                        let mut ret = resolve_ts_type_with_substitution(
                            signature
                                .generic_return_type
                                .as_ref()
                                .expect("generic callback return type"),
                            &substitution,
                            self.interfaces,
                            self.generic_interfaces,
                            &mut Vec::new(),
                        )?;
                        if signature.is_async && !matches!(ret, HirType::Promise(_)) {
                            ret = HirType::Promise(Box::new(ret));
                        }
                        let specialized = specialized_generic_function_name(
                            &name,
                            parameter_types,
                            signature,
                            &types,
                        );
                        return Ok(HirExpr::FunctionRef(
                            specialized,
                            parameter_types.to_vec(),
                            ret,
                        ));
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
        let callback = if parameter_types
            .iter()
            .any(|ty| matches!(ty, HirType::Optional(_) | HirType::Nullish(_)))
        {
            if let Some(expected_return) = expected_return {
                let optional = parameter_types
                    .iter()
                    .map(|ty| matches!(ty, HirType::Optional(_) | HirType::Nullish(_)))
                    .collect::<Vec<_>>();
                let declared = HirType::CallableFunction(
                    parameter_types.to_vec(),
                    optional_parameter_mask(&optional),
                    None,
                    Box::new(expected_return.clone()),
                );
                self.adapt_named_function_to_callable(&declared, &callback)?
                    .unwrap_or(callback)
            } else {
                callback
            }
        } else {
            callback
        };
        let (params, ret) = match self.infer_expr_type(&callback)? {
            HirType::Function(params, ret) => (params, ret),
            HirType::CallableFunction(mut params, _, rest, ret) => {
                if let Some(rest) = rest {
                    params.push(HirType::Array(rest));
                }
                (params, ret)
            }
            _ => return Err("Promise callback is not a function value".into()),
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

    fn callback_parameter_count(&self, expr: &Expr, label: &str) -> Result<usize, String> {
        match expr {
            Expr::Arrow(arrow) => Ok(arrow.params.len()),
            Expr::Fn(function) => Ok(function.function.params.len()),
            Expr::Ident(ident) => {
                let name = self.resolve_binding(ident.sym.as_ref());
                self.scope
                    .get(&name)
                    .and_then(|ty| match ty {
                        HirType::Function(params, _)
                        | HirType::CallableFunction(params, _, _, _) => Some(params.len()),
                        _ => None,
                    })
                    .or_else(|| {
                        self.generic_arrows
                            .get(&name)
                            .map(|arrow| arrow.params.len())
                    })
                    .or_else(|| {
                        self.generic_named_templates
                            .get(&name)
                            .and_then(|target| self.signatures.get(target))
                            .map(|signature| signature.params.len())
                    })
                    .or_else(|| {
                        self.signatures
                            .get(&name)
                            .map(|signature| signature.params.len())
                    })
                    .ok_or_else(|| format!("unknown {label} `{name}`"))
            }
            _ => Err(format!("{label} must be an arrow or function value")),
        }
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
        let arity = self.callback_parameter_count(&executor.expr, "Promise executor")?;
        if arity > 2 {
            return Err(format!(
                "Promise executor accepts at most two parameters, got {arity}"
            ));
        }
        let available = [resolve, reject];
        let executor =
            self.lower_promise_callback(&executor.expr, &available[..arity], Some(&HirType::Void))?;
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

    fn unreachable_value(ty: &HirType) -> Result<HirExpr, String> {
        match ty {
            HirType::F64 => Ok(HirExpr::Lit(HirLit::F64(0.0))),
            HirType::Bool => Ok(HirExpr::Lit(HirLit::Bool(false))),
            HirType::Str => Ok(HirExpr::Lit(HirLit::Str(String::new()))),
            HirType::Undefined | HirType::Void => Ok(HirExpr::Lit(HirLit::Undefined)),
            HirType::Null => Ok(HirExpr::Lit(HirLit::Null)),
            HirType::Object(_) => Ok(HirExpr::ObjectAlloc(ty.clone())),
            HirType::Array(element) => Ok(HirExpr::ArrayAlloc(
                Box::new(HirExpr::Lit(HirLit::F64(0.0))),
                element.as_ref().clone(),
            )),
            HirType::Optional(payload) => Ok(HirExpr::OptionalNone(payload.as_ref().clone())),
            HirType::Nullable(payload) => Ok(HirExpr::NullableNone(payload.as_ref().clone())),
            HirType::Nullish(payload) => Ok(HirExpr::NullishUndefined(payload.as_ref().clone())),
            HirType::Function(params, result) => Ok(HirExpr::Lambda(
                Vec::new(),
                params
                    .iter()
                    .enumerate()
                    .map(|(index, ty)| HirParam {
                        name: format!("__thaw_unreachable_parameter_{index}"),
                        ty: ty.clone(),
                    })
                    .collect(),
                result.as_ref().clone(),
                Box::new(Self::unreachable_value(result)?),
            )),
            other => Err(format!(
                "unbound `this` cannot synthesize unreachable value of type {other:?}"
            )),
        }
    }

    fn unbound_this_member_type(&self, property: &str) -> Option<HirType> {
        let class = self.class_context.as_deref()?;
        if self.class_static_context {
            let field = class_static_field_symbol(class, property);
            if let Some(ty) = self.scope.get(&field) {
                return Some(ty.clone());
            }
            let getter = class_getter_symbol(class, property, true);
            if let Some(signature) = self.signatures.get(&getter) {
                return Some(signature.ret.clone());
            }
            let method = class_static_method_symbol(class, property);
            let signature = self.signatures.get(&method)?;
            let result = if signature.is_async && !matches!(signature.ret, HirType::Promise(_)) {
                HirType::Promise(Box::new(signature.ret.clone()))
            } else {
                signature.ret.clone()
            };
            return Some(HirType::Function(
                signature.params.clone(),
                Box::new(result),
            ));
        }
        let instance = self.interfaces.get(class)?;
        if let HirType::Object(fields) = instance {
            if let Some((_, ty)) = fields.iter().find(|(name, _)| name == property) {
                return Some(ty.clone());
            }
        }
        let getter = class_getter_symbol(class, property, false);
        if let Some(signature) = self.signatures.get(&getter) {
            return Some(signature.ret.clone());
        }
        let method = class_method_symbol(class, property);
        let signature = self.signatures.get(&method)?;
        let result = if signature.is_async && !matches!(signature.ret, HirType::Promise(_)) {
            HirType::Promise(Box::new(signature.ret.clone()))
        } else {
            signature.ret.clone()
        };
        Some(HirType::Function(
            signature.params[1..].to_vec(),
            Box::new(result),
        ))
    }

    fn lower_unbound_this_error(&self, property: &str, ty: &HirType) -> Result<HirExpr, String> {
        Ok(HirExpr::ThrowValue(
            Box::new(HirExpr::Lit(HirLit::Str(format!(
                "Cannot read properties of undefined (reading '{property}')"
            )))),
            Box::new(Self::unreachable_value(ty)?),
        ))
    }

    fn lower_unbound_this_member(&self, property: &str) -> Result<HirExpr, String> {
        let ty = self.unbound_this_member_type(property).ok_or_else(|| {
            format!(
                "class `{}` has no native member `{property}`",
                self.class_context.as_deref().unwrap_or("<unknown>")
            )
        })?;
        self.lower_unbound_this_error(property, &ty)
    }

    fn typed_dictionary_read(value: HirExpr, element: &HirType) -> Result<HirExpr, String> {
        match element {
            HirType::F64 => Ok(HirExpr::JsonAsNumber(Box::new(value))),
            HirType::Str => Ok(HirExpr::JsonAsString(Box::new(value))),
            HirType::Bool => Ok(HirExpr::JsonAsBool(Box::new(value))),
            HirType::Json => Ok(value),
            other => Err(format!(
                "dictionary reads do not yet support value type {other:?}"
            )),
        }
    }

    fn lower_member_read(&mut self, member: &MemberExpr) -> Result<HirExpr, String> {
        if self.unbound_this_context && matches!(member.obj.as_ref(), Expr::This(_)) {
            let property = member_property_name(&member.prop)
                .ok_or("unbound `this` member access requires a statically known property")?;
            return self.lower_unbound_this_member(&property);
        }
        if let Expr::Ident(enum_name) = member.obj.as_ref() {
            let member_name = match &member.prop {
                MemberProp::Ident(member) => Some(member.sym.to_string()),
                MemberProp::Computed(computed) => match computed.expr.as_ref() {
                    Expr::Lit(Lit::Str(member)) => {
                        Some(member.value.to_string_lossy().into_owned())
                    }
                    _ => None,
                },
                _ => None,
            };
            if let Some(member_name) = member_name {
                let key = (enum_name.sym.to_string(), member_name.clone());
                if let Some(value) = self.enum_values.get(&key) {
                    return Ok(HirExpr::Lit(value.clone()));
                }
                if self
                    .enum_values
                    .keys()
                    .any(|(candidate, _)| candidate == enum_name.sym.as_str())
                {
                    return Err(format!(
                        "enum `{}` has no member `{member_name}`",
                        enum_name.sym
                    ));
                }
            }
            if let MemberProp::Computed(computed) = &member.prop {
                if let Some(entries) = self.enum_reverse_values.get(enum_name.sym.as_str()) {
                    let index = self.lower_expr(&computed.expr)?;
                    self.expect_type(&HirType::F64, &index, "numeric enum reverse lookup")?;
                    return Ok(HirExpr::EnumReverseLookup(Box::new(index), entries.clone()));
                }
                if self
                    .enum_values
                    .keys()
                    .any(|(candidate, _)| candidate == enum_name.sym.as_str())
                {
                    return Err(format!(
                        "string enum `{}` does not support numeric reverse lookup",
                        enum_name.sym
                    ));
                }
            }
        }
        if let Some(property) = member_property_name(&member.prop) {
            if matches!(member.obj.as_ref(), Expr::This(_)) && self.class_static_context {
                let class = self
                    .class_context
                    .as_deref()
                    .expect("static class lowering retains its class context");
                let field_symbol = class_static_field_symbol(class, &property);
                if self.scope.contains_key(&field_symbol) {
                    return Ok(HirExpr::Var(field_symbol));
                }
                let getter = class_getter_symbol(class, &property, true);
                if self.signatures.contains_key(&getter) {
                    return Ok(HirExpr::Call(Box::new(HirExpr::Var(getter)), Vec::new()));
                }
            }
            if let Expr::Ident(receiver) = member.obj.as_ref() {
                let field_symbol = class_static_field_symbol(receiver.sym.as_ref(), &property);
                if self.scope.contains_key(&field_symbol) {
                    return Ok(HirExpr::Var(field_symbol));
                }
                let static_symbol = class_getter_symbol(receiver.sym.as_ref(), &property, true);
                if self.signatures.contains_key(&static_symbol) {
                    return Ok(HirExpr::Call(
                        Box::new(HirExpr::Var(static_symbol)),
                        Vec::new(),
                    ));
                }
                let binding = self.resolve_binding(receiver.sym.as_ref());
                if let Some(receiver_type) = self.scope.get(&binding).cloned() {
                    if let Some(class_name) = class_name_from_type(&receiver_type) {
                        let symbol = class_getter_symbol(class_name, &property, false);
                        if self.signatures.contains_key(&symbol) {
                            return Ok(HirExpr::Call(
                                Box::new(HirExpr::Var(symbol)),
                                vec![HirExpr::Var(binding)],
                            ));
                        }
                    }
                }
            }
            if matches!(member.obj.as_ref(), Expr::This(_)) {
                let binding = self.resolve_binding("this");
                if let Some(receiver_type) = self.scope.get(&binding).cloned() {
                    if let Some(class_name) = class_name_from_type(&receiver_type) {
                        let symbol = class_getter_symbol(class_name, &property, false);
                        if self.signatures.contains_key(&symbol) {
                            return Ok(HirExpr::Call(
                                Box::new(HirExpr::Var(symbol)),
                                vec![HirExpr::Var(binding)],
                            ));
                        }
                    }
                }
            }
            if let Some(reference) = self.lower_native_method_reference(member, &property)? {
                return Ok(reference);
            }
        }
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
                        if let Expr::Lit(Lit::Str(key)) = computed.expr.as_ref() {
                            let key = key.value.to_string_lossy().into_owned();
                            if fields.iter().any(|(name, _)| name == &key) {
                                return Ok(HirExpr::PropAccess(
                                    Box::new(obj),
                                    HirType::Object(fields),
                                    key,
                                ));
                            }
                            return Err(format!("object has no field `{key}`"));
                        }
                        let Some((_, payload)) = fields.first() else {
                            return Err("cannot dynamically index an empty object".into());
                        };
                        let payload = payload.clone();
                        if fields.iter().any(|(_, ty)| ty != &payload) {
                            let mut members = Vec::new();
                            for (_, ty) in &fields {
                                let (payload, absences) = match ty {
                                    HirType::Optional(payload) => {
                                        (payload.as_ref(), &[HirType::Undefined][..])
                                    }
                                    HirType::Nullable(payload) => {
                                        (payload.as_ref(), &[HirType::Null][..])
                                    }
                                    HirType::Nullish(payload) => {
                                        (payload.as_ref(), &[HirType::Null, HirType::Undefined][..])
                                    }
                                    other => (other, &[][..]),
                                };
                                if !matches!(
                                    payload,
                                    HirType::F64
                                        | HirType::I64
                                        | HirType::Bool
                                        | HirType::Str
                                        | HirType::Json
                                        | HirType::JsValue
                                        | HirType::Array(_)
                                        | HirType::Tuple(_)
                                        | HirType::Object(_)
                                        | HirType::Function(_, _)
                                        | HirType::Null
                                        | HirType::Undefined
                                ) {
                                    return Err(
                                        "dynamic heterogeneous object index requires word-sized union-compatible field payloads"
                                            .into(),
                                    );
                                }
                                if !members.contains(payload) {
                                    members.push(payload.clone());
                                }
                                for absence in absences {
                                    if !members.contains(absence) {
                                        members.push(absence.clone());
                                    }
                                }
                            }
                            if !members.contains(&HirType::Undefined) {
                                members.push(HirType::Undefined);
                            }
                            let key = self.lower_expr(&computed.expr)?;
                            self.expect_type(&HirType::Str, &key, "computed object key")?;
                            return Ok(HirExpr::DynamicPropAccess(
                                Box::new(obj),
                                Box::new(key),
                                fields,
                                HirType::Union(members),
                            ));
                        }
                        let result = match &payload {
                            HirType::Optional(inner) => HirType::Optional(inner.clone()),
                            HirType::Nullable(inner) | HirType::Nullish(inner) => {
                                HirType::Nullish(inner.clone())
                            }
                            other => HirType::Optional(Box::new(other.clone())),
                        };
                        let key = self.lower_expr(&computed.expr)?;
                        self.expect_type(&HirType::Str, &key, "computed object key")?;
                        Ok(HirExpr::DynamicPropAccess(
                            Box::new(obj),
                            Box::new(key),
                            fields,
                            result,
                        ))
                    }
                    HirType::Dictionary(element) => {
                        let key = self.lower_expr(&computed.expr)?;
                        self.expect_type(&HirType::Str, &key, "dictionary key")?;
                        Self::typed_dictionary_read(
                            HirExpr::JsonKey(Box::new(obj), Box::new(key)),
                            element.as_ref(),
                        )
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
                    HirType::Union(elements) => {
                        self.lower_union_property_read(obj, elements, prop.sym.as_ref())
                    }
                    HirType::Json => Ok(HirExpr::JsonGet(Box::new(obj), prop.sym.to_string())),
                    HirType::Dictionary(element) => Self::typed_dictionary_read(
                        HirExpr::JsonGet(Box::new(obj), prop.sym.to_string()),
                        element.as_ref(),
                    ),
                    other => Err(format!(
                        "unsupported property access `.{}` on a value of type {other:?}",
                        prop.sym
                    )),
                }
            }
            _ => Err("unsupported property access".into()),
        }
    }

    fn lower_optional_member_read(&mut self, member: &MemberExpr) -> Result<HirExpr, String> {
        let object = self.lower_expr(&member.obj)?;
        let object_type = self.infer_expr_type(&object)?;
        let (payload, absence_kind) = match object_type.clone() {
            HirType::Optional(payload) => (payload, 0),
            HirType::Nullable(payload) => (payload, 1),
            HirType::Nullish(payload) => (payload, 2),
            _ => return self.lower_member_read(member),
        };
        let name = format!("__thaw_optional_receiver_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(name.clone(), object_type.clone());
        let bound = HirExpr::Var(name.clone());
        let unwrapped = match absence_kind {
            0 => HirExpr::OptionalValue(Box::new(bound.clone()), payload.as_ref().clone()),
            1 => HirExpr::NullableValue(Box::new(bound.clone()), payload.as_ref().clone()),
            2 => HirExpr::NullishValue(Box::new(bound.clone()), payload.as_ref().clone()),
            _ => unreachable!(),
        };
        let (access, field_type) = match payload.as_ref() {
            HirType::Object(fields) => {
                let field = match &member.prop {
                    MemberProp::Ident(field) => field.sym.to_string(),
                    MemberProp::Computed(computed) => match computed.expr.as_ref() {
                        Expr::Lit(Lit::Str(field)) => field.value.to_string_lossy().into_owned(),
                        _ => {
                            return Err(
                                "optional computed object keys must be string literals".into()
                            )
                        }
                    },
                    _ => return Err("unsupported optional object member".into()),
                };
                let field_type = fields
                    .iter()
                    .find(|(name, _)| name == &field)
                    .map(|(_, ty)| ty.clone())
                    .ok_or_else(|| format!("object has no field `{field}`"))?;
                (
                    HirExpr::PropAccess(Box::new(unwrapped), payload.as_ref().clone(), field),
                    field_type,
                )
            }
            HirType::Array(element) => match &member.prop {
                MemberProp::Ident(property) if property.sym == *"length" => {
                    (HirExpr::ArrayLen(Box::new(unwrapped)), HirType::F64)
                }
                MemberProp::Computed(computed) => {
                    let index = self.lower_expr(&computed.expr)?;
                    self.expect_type(&HirType::F64, &index, "optional array index")?;
                    (
                        HirExpr::TypedIndex(Box::new(unwrapped), Box::new(index), *element.clone()),
                        *element.clone(),
                    )
                }
                _ => return Err("unsupported optional array member".into()),
            },
            HirType::Tuple(elements) => match &member.prop {
                MemberProp::Ident(property) if property.sym == *"length" => {
                    (HirExpr::ArrayLen(Box::new(unwrapped)), HirType::F64)
                }
                MemberProp::Computed(computed) => {
                    let Expr::Lit(Lit::Num(index)) = computed.expr.as_ref() else {
                        return Err("optional tuple index must be a numeric literal".into());
                    };
                    let index = index.value as usize;
                    let element = elements
                        .get(index)
                        .cloned()
                        .ok_or_else(|| format!("tuple index {index} is out of bounds"))?;
                    (
                        HirExpr::TypedIndex(
                            Box::new(unwrapped),
                            Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                            element.clone(),
                        ),
                        element,
                    )
                }
                _ => return Err("unsupported optional tuple member".into()),
            },
            HirType::Str => match &member.prop {
                MemberProp::Ident(property) if property.sym == *"length" => (
                    HirExpr::Call(
                        Box::new(HirExpr::Var("__thaw_string_length".into())),
                        vec![unwrapped],
                    ),
                    HirType::F64,
                ),
                _ => return Err("unsupported optional string member".into()),
            },
            other => {
                return Err(format!(
                    "optional member access is not yet supported on {other:?}"
                ))
            }
        };
        let (result_payload, present_value) = match &field_type {
            HirType::Optional(inner) => (inner.as_ref().clone(), None),
            other => (other.clone(), Some(other.clone())),
        };
        let present_value = match present_value {
            Some(payload) => HirExpr::OptionalSome(Box::new(access), payload),
            None => access,
        };
        let is_none = match absence_kind {
            0 => HirExpr::OptionalIsNone(Box::new(bound), payload.as_ref().clone()),
            1 => HirExpr::NullableIsNone(Box::new(bound), payload.as_ref().clone()),
            2 => HirExpr::NullishIsNone(Box::new(bound), payload.as_ref().clone()),
            _ => unreachable!(),
        };
        let result = HirExpr::Block(vec![HirStmt::If(
            is_none,
            vec![HirStmt::Return(Some(HirExpr::OptionalNone(
                result_payload.clone(),
            )))],
            vec![HirStmt::Return(Some(present_value))],
        )]);
        self.wrap_call_argument_bindings(result, &[(name, object_type, object)])
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
            HirType::Array(_) => {
                let index = self.lower_expr(&computed.expr)?;
                self.expect_type(&HirType::F64, &index, "index expression")?;
                Ok(Target::Index(object, Box::new(index)))
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
            HirType::Dictionary(element) => {
                let key = self.lower_expr(&computed.expr)?;
                self.expect_type(&HirType::Str, &key, "dictionary assignment key")?;
                if !matches!(
                    element.as_ref(),
                    HirType::F64 | HirType::Str | HirType::Bool | HirType::Json
                ) {
                    return Err(format!(
                        "unsupported dictionary value type {:?}",
                        element.as_ref()
                    ));
                }
                Ok(Target::Dictionary(object, Box::new(key), *element.clone()))
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
            SimpleAssignTarget::Member(member) => {
                if matches!(member.obj.as_ref(), Expr::This(_)) && self.class_static_context {
                    if let Some(property) = member_property_name(&member.prop) {
                        let class = self
                            .class_context
                            .as_deref()
                            .expect("static assignment retains its class context");
                        let symbol = class_static_field_symbol(class, &property);
                        if self.scope.contains_key(&symbol) {
                            return Ok(Target::Var(symbol));
                        }
                    }
                }
                if let (Expr::Ident(class), Some(property)) =
                    (member.obj.as_ref(), member_property_name(&member.prop))
                {
                    let symbol = class_static_field_symbol(class.sym.as_ref(), &property);
                    if self.scope.contains_key(&symbol) {
                        return Ok(Target::Var(symbol));
                    }
                }
                match &member.prop {
                    MemberProp::Computed(computed) => self.lower_computed_target(member, computed),
                    MemberProp::Ident(prop) => {
                        if let Expr::Ident(class) = member.obj.as_ref() {
                            let symbol =
                                class_static_field_symbol(class.sym.as_ref(), prop.sym.as_ref());
                            if self.scope.contains_key(&symbol) {
                                return Ok(Target::Var(symbol));
                            }
                        }
                        let obj = self.lower_expr(&member.obj)?;
                        let obj_ty = self.infer_expr_type(&obj)?;
                        match &obj_ty {
                            HirType::Object(fields)
                                if fields.iter().any(|(n, _)| n == prop.sym.as_str()) =>
                            {
                                Ok(Target::Prop(obj, obj_ty.clone(), prop.sym.to_string()))
                            }
                            HirType::Dictionary(element)
                                if matches!(
                                    element.as_ref(),
                                    HirType::F64 | HirType::Str | HirType::Bool | HirType::Json
                                ) =>
                            {
                                Ok(Target::Dictionary(
                                    obj,
                                    Box::new(HirExpr::Lit(HirLit::Str(prop.sym.to_string()))),
                                    *element.clone(),
                                ))
                            }
                            other => Err(format!(
                                "cannot assign to `.{}` on a value of type {other:?}",
                                prop.sym
                            )),
                        }
                    }
                    _ => Err(
                        "only `arr[i] = ...` / `obj.field = ...` member assignment is supported"
                            .into(),
                    ),
                }
            }
            _ => Err("unsupported assignment target".into()),
        }
    }

    fn lower_assign(&mut self, assign: &swc_ecma_ast::AssignExpr) -> Result<HirExpr, String> {
        let assigned_function_property = if assign.op == AssignOp::Assign {
            match &assign.left {
                AssignTarget::Simple(SimpleAssignTarget::Member(member)) => {
                    match (
                        self.expression_static_property_path(&member.obj),
                        member_property_name(&member.prop),
                    ) {
                        (Some((object, mut path)), Some(property)) => {
                            path.push(property);
                            Some((
                                object,
                                path,
                                self.expression_function_property_result_discriminants(
                                    &assign.right,
                                ),
                                self.expression_object_function_property_discriminants(
                                    &assign.right,
                                ),
                            ))
                        }
                        _ => None,
                    }
                }
                _ => None,
            }
        } else {
            None
        };
        if self.unbound_this_context {
            if let AssignTarget::Simple(SimpleAssignTarget::Member(member)) = &assign.left {
                if matches!(member.obj.as_ref(), Expr::This(_)) {
                    let property = member_property_name(&member.prop)
                        .ok_or("unbound `this` assignment requires a statically known property")?;
                    if assign.op != AssignOp::Assign {
                        return self.lower_unbound_this_member(&property);
                    }
                    let value = self.lower_expr(&assign.right)?;
                    let value_type = self.infer_expr_type(&value)?;
                    let value_name = format!("__thaw_unbound_assignment_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(value_name.clone(), value_type.clone());
                    let result = HirExpr::ThrowValue(
                        Box::new(HirExpr::Lit(HirLit::Str(format!(
                            "Cannot set properties of undefined (setting '{property}')"
                        )))),
                        Box::new(HirExpr::Var(value_name.clone())),
                    );
                    return self
                        .wrap_call_argument_bindings(result, &[(value_name, value_type, value)]);
                }
            }
        }
        if self.class_static_context {
            if let AssignTarget::Simple(SimpleAssignTarget::SuperProp(member)) = &assign.left {
                let (_, _, base_name) = self
                    .super_initializer
                    .clone()
                    .ok_or("`super` property assignment is only valid in a derived class")?;
                let property = super_property_name(&member.prop)?;
                let storage = class_static_field_symbol(&base_name, &property);
                if let Some(expected) = self.scope.get(&storage).cloned() {
                    if self.immutable_bindings.contains(&storage) {
                        return Err(format!(
                            "cannot assign to readonly static member `super.{property}`"
                        ));
                    }
                    let rhs = self.lower_expr(&assign.right)?;
                    let value = if assign.op == AssignOp::Assign {
                        rhs
                    } else if let Some(operator) = compound_op(assign.op) {
                        let current = HirExpr::Var(storage.clone());
                        if assign.op == AssignOp::AddAssign
                            && (expected == HirType::Str
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
                            HirExpr::BinOp(operator, Box::new(current), Box::new(rhs))
                        }
                    } else {
                        return Err(format!(
                            "unsupported super static-field assignment operator {:?}",
                            assign.op
                        ));
                    };
                    let value = self.coerce_to_declared(&expected, value)?;
                    return Ok(HirExpr::Assign(storage, Box::new(value)));
                }
            }
        }
        if let AssignTarget::Simple(SimpleAssignTarget::Member(member)) = &assign.left {
            let static_class = match member.obj.as_ref() {
                Expr::Ident(receiver) => Some(receiver.sym.to_string()),
                Expr::This(_) if self.class_static_context => self.class_context.clone(),
                _ => None,
            };
            if let (Some(receiver), Some(property)) =
                (static_class, member_property_name(&member.prop))
            {
                let getter = class_getter_symbol(&receiver, &property, true);
                let setter = class_setter_symbol(&receiver, &property, true);
                let has_getter = self.signatures.contains_key(&getter);
                let has_setter = self.signatures.contains_key(&setter);
                if has_getter && !has_setter {
                    return Err(format!(
                        "cannot assign to readonly static member `{}.{}`",
                        receiver, property
                    ));
                }
                if assign.op != AssignOp::Assign && has_setter {
                    let signature = self.signatures[&setter].clone();
                    let rhs = self.lower_expr(&assign.right)?;
                    let current = HirExpr::Call(Box::new(HirExpr::Var(getter)), Vec::new());
                    let value = if let Some(operator) = compound_op(assign.op) {
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
                            HirExpr::BinOp(operator, Box::new(current), Box::new(rhs))
                        }
                    } else {
                        return Err(format!(
                            "unsupported inherited static-field assignment operator {:?}",
                            assign.op
                        ));
                    };
                    let value = self.coerce_to_declared(&signature.params[0], value)?;
                    return Ok(HirExpr::Call(Box::new(HirExpr::Var(setter)), vec![value]));
                }
            }
        }
        if assign.op == AssignOp::Assign {
            if let AssignTarget::Simple(SimpleAssignTarget::SuperProp(member)) = &assign.left {
                let (_, _, base_name) = self
                    .super_initializer
                    .clone()
                    .ok_or("`super` property assignment is only valid in a derived class")?;
                let property = super_property_name(&member.prop)?;
                let symbol = class_setter_symbol(&base_name, &property, self.class_static_context);
                let signature = self.signatures.get(&symbol).cloned().ok_or_else(|| {
                    format!("base class `{base_name}` has no setter `{property}`")
                })?;
                let rhs = self.lower_expr(&assign.right)?;
                let value_index = usize::from(!self.class_static_context);
                let rhs = self.coerce_to_declared(&signature.params[value_index], rhs)?;
                let mut args = if self.class_static_context {
                    Vec::new()
                } else {
                    vec![HirExpr::Var(self.resolve_binding("this"))]
                };
                args.push(rhs);
                return Ok(HirExpr::Call(Box::new(HirExpr::Var(symbol)), args));
            }
            if let AssignTarget::Simple(SimpleAssignTarget::Member(member)) = &assign.left {
                if let (Expr::Ident(receiver), Some(property)) =
                    (member.obj.as_ref(), member_property_name(&member.prop))
                {
                    let static_symbol = class_setter_symbol(receiver.sym.as_ref(), &property, true);
                    let (symbol, receiver_argument) =
                        if self.signatures.contains_key(&static_symbol) {
                            (Some(static_symbol), None)
                        } else {
                            let binding = self.resolve_binding(receiver.sym.as_ref());
                            let instance_symbol = self.scope.get(&binding).and_then(|ty| {
                                class_name_from_type(ty).map(|class_name| {
                                    class_setter_symbol(class_name, &property, false)
                                })
                            });
                            match instance_symbol {
                                Some(symbol) if self.signatures.contains_key(&symbol) => {
                                    (Some(symbol), Some(HirExpr::Var(binding)))
                                }
                                _ => (None, None),
                            }
                        };
                    if let Some(symbol) = symbol {
                        let signature = self.signatures[&symbol].clone();
                        let value_index = usize::from(receiver_argument.is_some());
                        let rhs = self.lower_expr(&assign.right)?;
                        let rhs = self.coerce_to_declared(&signature.params[value_index], rhs)?;
                        let mut args = Vec::with_capacity(value_index + 1);
                        if let Some(receiver) = receiver_argument {
                            args.push(receiver);
                        }
                        args.push(rhs);
                        return Ok(HirExpr::Call(Box::new(HirExpr::Var(symbol)), args));
                    }
                }
                if let (Expr::This(_), Some(property)) =
                    (member.obj.as_ref(), member_property_name(&member.prop))
                {
                    if self.class_static_context {
                        let class = self
                            .class_context
                            .as_deref()
                            .expect("static setter retains its class context");
                        let symbol = class_setter_symbol(class, &property, true);
                        if let Some(signature) = self.signatures.get(&symbol).cloned() {
                            let rhs = self.lower_expr(&assign.right)?;
                            let rhs = self.coerce_to_declared(&signature.params[0], rhs)?;
                            return Ok(HirExpr::Call(Box::new(HirExpr::Var(symbol)), vec![rhs]));
                        }
                    }
                    let binding = self.resolve_binding("this");
                    let symbol = self.scope.get(&binding).and_then(|ty| {
                        class_name_from_type(ty)
                            .map(|class_name| class_setter_symbol(class_name, &property, false))
                    });
                    if let Some(symbol) =
                        symbol.filter(|symbol| self.signatures.contains_key(symbol))
                    {
                        let signature = self.signatures[&symbol].clone();
                        let rhs = self.lower_expr(&assign.right)?;
                        let rhs = self.coerce_to_declared(&signature.params[1], rhs)?;
                        return Ok(HirExpr::Call(
                            Box::new(HirExpr::Var(symbol)),
                            vec![HirExpr::Var(binding), rhs],
                        ));
                    }
                }
            }
        }
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
            let propagated_discriminants = self.expression_union_discriminants(&assign.right);
            let propagated_function_property_discriminants =
                self.expression_object_function_property_discriminants(&assign.right);
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
            let destructurable_union = matches!(
                &ty,
                HirType::Union(elements)
                    if (matches!(pattern, Pat::Object(_))
                        && elements.iter().all(|element| matches!(element, HirType::Object(_))))
                        || (matches!(pattern, Pat::Array(_))
                            && elements.iter().all(|element| matches!(element, HirType::Tuple(_))))
            );
            if !matches!(ty, HirType::Object(_) | HirType::Tuple(_)) && !destructurable_union {
                return Err(format!(
                    "destructuring assignment requires a fixed-shape object, tuple, or destructurable union, got {ty:?}"
                ));
            }
            let temporary = format!("__thaw_destructure_assign_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(temporary.clone(), ty.clone());
            if let Some(discriminants) = propagated_discriminants {
                self.union_discriminants
                    .insert(temporary.clone(), discriminants);
            }
            if let Some(discriminants) = propagated_function_property_discriminants {
                self.object_function_property_discriminants
                    .insert(temporary.clone(), discriminants);
            }
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
        if let Target::Var(name) = &target {
            if self.immutable_bindings.contains(name) {
                return Err(format!("cannot assign to constant `{name}`"));
            }
        }
        let rhs = self.lower_expr(&assign.right)?;
        let rhs_type = self.infer_expr_type(&rhs)?;
        let assigned_variable = match &target {
            Target::Var(name) => Some(name.clone()),
            _ => None,
        };
        let assigned_method_value = (assign.op == AssignOp::Assign)
            .then(|| {
                self.native_instance_method_value(&assign.right)
                    .or_else(|| {
                        let Expr::Ident(identifier) = assign.right.as_ref() else {
                            return None;
                        };
                        self.native_method_values
                            .get(&self.resolve_binding(identifier.sym.as_ref()))
                            .cloned()
                    })
            })
            .flatten();
        let assigned_function_metadata =
            (assign.op == AssignOp::Assign && assigned_variable.is_some()).then(|| {
                (
                    self.expression_function_discriminants(&assign.right),
                    self.expression_function_array_discriminants(&assign.right),
                    self.expression_function_nested_array_discriminants(&assign.right),
                    self.expression_function_object_array_property_discriminants(&assign.right),
                    self.expression_function_object_function_property_discriminants(&assign.right),
                )
            });
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
                    bindings.push((index_name.clone(), HirType::F64, *index));
                    Target::Index(HirExpr::Var(array_name), Box::new(HirExpr::Var(index_name)))
                }
                Target::Prop(object, object_type, field) => {
                    let object_name = format!("__thaw_assign_object_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(object_name.clone(), object_type.clone());
                    bindings.push((object_name.clone(), object_type.clone(), object));
                    Target::Prop(HirExpr::Var(object_name), object_type, field)
                }
                Target::Dictionary(object, key, element) => {
                    let object_name = format!("__thaw_assign_dictionary_{}", self.next_binding);
                    self.next_binding += 1;
                    let object_type = HirType::Dictionary(Box::new(element.clone()));
                    self.scope.insert(object_name.clone(), object_type.clone());
                    bindings.push((object_name.clone(), object_type, object));

                    let key_name = format!("__thaw_assign_key_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(key_name.clone(), HirType::Str);
                    bindings.push((key_name.clone(), HirType::Str, *key));
                    Target::Dictionary(
                        HirExpr::Var(object_name),
                        Box::new(HirExpr::Var(key_name)),
                        element,
                    )
                }
            };
        }

        if assign.op == AssignOp::NullishAssign {
            let current = target_to_read_expr(&target)?;
            let current_type = self.infer_expr_type(&current)?;
            let (payload, absence_kind) = match current_type.clone() {
                HirType::Optional(payload) => (payload, 0),
                HirType::Nullable(payload) => (payload, 1),
                HirType::Nullish(payload) => (payload, 2),
                _ => return self.wrap_call_argument_bindings(current, &bindings),
            };
            let rhs = self.coerce_to_declared(payload.as_ref(), rhs)?;
            let current_name = format!("__thaw_nullish_assign_current_{}", self.next_binding);
            self.next_binding += 1;
            self.scope
                .insert(current_name.clone(), current_type.clone());
            let rhs_name = format!("__thaw_nullish_assign_rhs_{}", self.next_binding);
            self.next_binding += 1;
            self.scope
                .insert(rhs_name.clone(), payload.as_ref().clone());

            let stored = match absence_kind {
                0 => HirExpr::OptionalSome(
                    Box::new(HirExpr::Var(rhs_name.clone())),
                    payload.as_ref().clone(),
                ),
                1 => HirExpr::NullableSome(
                    Box::new(HirExpr::Var(rhs_name.clone())),
                    payload.as_ref().clone(),
                ),
                2 => HirExpr::NullishSome(
                    Box::new(HirExpr::Var(rhs_name.clone())),
                    payload.as_ref().clone(),
                ),
                _ => unreachable!(),
            };
            let assigned = HirExpr::Block(vec![
                HirStmt::Expr(build_assign(target, stored)),
                HirStmt::Return(Some(HirExpr::Var(rhs_name.clone()))),
            ]);
            let assigned = self.wrap_call_argument_bindings(
                assigned,
                &[(rhs_name, payload.as_ref().clone(), rhs)],
            )?;
            let current_value = HirExpr::Var(current_name.clone());
            let is_none = match absence_kind {
                0 => HirExpr::OptionalIsNone(
                    Box::new(current_value.clone()),
                    payload.as_ref().clone(),
                ),
                1 => HirExpr::NullableIsNone(
                    Box::new(current_value.clone()),
                    payload.as_ref().clone(),
                ),
                2 => HirExpr::NullishIsNone(
                    Box::new(current_value.clone()),
                    payload.as_ref().clone(),
                ),
                _ => unreachable!(),
            };
            let present = match absence_kind {
                0 => HirExpr::OptionalValue(Box::new(current_value), payload.as_ref().clone()),
                1 => HirExpr::NullableValue(Box::new(current_value), payload.as_ref().clone()),
                2 => HirExpr::NullishValue(Box::new(current_value), payload.as_ref().clone()),
                _ => unreachable!(),
            };
            let result = HirExpr::Block(vec![HirStmt::If(
                is_none,
                vec![HirStmt::Return(Some(assigned))],
                vec![HirStmt::Return(Some(present))],
            )]);
            bindings.push((current_name, current_type, current));
            if let Some(name) = assigned_variable {
                if absence_kind == 1 {
                    self.nullable_narrowings
                        .insert(name, payload.as_ref().clone());
                } else if absence_kind == 0 {
                    self.narrowings.insert(name, payload.as_ref().clone());
                } else if absence_kind == 2 {
                    self.nullish_narrowings
                        .insert(name, payload.as_ref().clone());
                }
            }
            return self.wrap_call_argument_bindings(result, &bindings);
        }

        let value = if assign.op == AssignOp::Assign {
            rhs
        } else if let Some(op) = compound_op(assign.op) {
            let current = target_to_read_expr(&target)?;
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
            Target::Dictionary(_, _, element) => self.coerce_to_declared(element, value)?,
            Target::Prop(_, other, field) => {
                return Err(format!(
                    "cannot assign to field `{field}` on value of type {other:?}"
                ));
            }
        };

        let result = build_assign(target, value);
        if let Some((object, path, value, nested)) = assigned_function_property {
            let metadata = self
                .object_function_property_discriminants
                .entry(object.clone())
                .or_default();
            metadata.retain(|existing, _| !existing.starts_with(&path));
            if let Some(value) = value {
                metadata.insert(path.clone(), value);
            }
            if let Some(nested) = nested {
                metadata.extend(nested.into_iter().map(|(suffix, discriminants)| {
                    let mut nested_path = path.clone();
                    nested_path.extend(suffix);
                    (nested_path, discriminants)
                }));
            }
            if metadata.is_empty() {
                self.object_function_property_discriminants.remove(&object);
            }
        }
        if assign.op == AssignOp::Assign {
            if let Some(name) = assigned_variable.as_ref() {
                if let Some(method) = assigned_method_value {
                    self.native_method_values.insert(name.clone(), method);
                } else {
                    self.native_method_values.remove(name);
                }
                let (value, array, nested_array, object, functions) =
                    assigned_function_metadata.expect("simple assignment metadata was collected");
                replace_metadata(&mut self.function_value_discriminants, name, value);
                replace_metadata(&mut self.function_value_array_discriminants, name, array);
                replace_metadata(
                    &mut self.function_value_nested_array_discriminants,
                    name,
                    nested_array,
                );
                replace_metadata(
                    &mut self.function_value_object_array_property_discriminants,
                    name,
                    object,
                );
                replace_metadata(
                    &mut self.function_value_object_function_property_discriminants,
                    name,
                    functions,
                );
            }
        }
        if let Some(name) = assigned_variable.as_ref() {
            self.invalidate_destructured_union_correlation(name);
        }
        if let Some(name) = assigned_variable {
            if let Some(HirType::Optional(payload)) = self.scope.get(&name) {
                if rhs_type == **payload || assign.op != AssignOp::Assign {
                    self.narrowings.insert(name, payload.as_ref().clone());
                } else {
                    self.narrowings.remove(&name);
                }
            } else if let Some(HirType::Nullable(payload)) = self.scope.get(&name) {
                if rhs_type == **payload || assign.op != AssignOp::Assign {
                    self.nullable_narrowings
                        .insert(name, payload.as_ref().clone());
                } else {
                    self.nullable_narrowings.remove(&name);
                }
            } else if let Some(HirType::Nullish(payload)) = self.scope.get(&name) {
                if rhs_type == **payload || assign.op != AssignOp::Assign {
                    self.nullish_narrowings
                        .insert(name, payload.as_ref().clone());
                } else {
                    self.nullish_narrowings.remove(&name);
                }
            } else if let Some(HirType::Union(elements)) = self.scope.get(&name) {
                if let Some(index) = elements.iter().position(|member| member == &rhs_type) {
                    self.union_narrowings
                        .insert(name, (vec![index], elements.clone()));
                } else {
                    self.union_narrowings.remove(&name);
                }
            }
        }
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
                let function = self.hir_function_property_discriminants(&value);
                let name = self.resolve_binding(binding.id.sym.as_ref());
                if self.immutable_bindings.contains(&name) {
                    return Err(format!("cannot assign to constant `{name}`"));
                }
                let expected = self
                    .scope
                    .get(&name)
                    .cloned()
                    .ok_or_else(|| format!("assignment to unknown binding `{name}`"))?;
                let value = self.coerce_to_declared(&expected, value)?;
                self.invalidate_destructured_union_correlation(&name);
                let (result, array, nested_array, object, functions) = function
                    .map(|metadata| {
                        (
                            metadata.value,
                            metadata.array,
                            (!metadata.nested_array.is_empty()).then_some(metadata.nested_array),
                            (!metadata.object.is_empty()).then_some(metadata.object),
                            (!metadata.functions.is_empty()).then_some(metadata.functions),
                        )
                    })
                    .unwrap_or_default();
                replace_metadata(&mut self.function_value_discriminants, &name, result);
                replace_metadata(&mut self.function_value_array_discriminants, &name, array);
                replace_metadata(
                    &mut self.function_value_nested_array_discriminants,
                    &name,
                    nested_array,
                );
                replace_metadata(
                    &mut self.function_value_object_array_property_discriminants,
                    &name,
                    object,
                );
                replace_metadata(
                    &mut self.function_value_object_function_property_discriminants,
                    &name,
                    functions,
                );
                statements.push(HirStmt::Expr(HirExpr::Assign(name, Box::new(value))));
                Ok(())
            }
            Pat::Object(pattern) => {
                if let HirType::Union(elements) = ty {
                    return self.lower_union_object_assignment_pattern(
                        pattern, value, elements, statements,
                    );
                }
                let HirType::Object(fields) = ty else {
                    return Err(format!("object pattern cannot destructure {ty:?}"));
                };
                let mut used = BTreeSet::new();
                for property in &pattern.props {
                    match property {
                        ObjectPatProp::Assign(property) => {
                            let key = property.key.id.sym.to_string();
                            let field_type = fields
                                .iter()
                                .find(|(name, _)| name == &key)
                                .map(|(_, ty)| ty.clone())
                                .ok_or_else(|| format!("object has no field `{key}`"))?;
                            used.insert(key.clone());
                            let mut field_value =
                                HirExpr::PropAccess(Box::new(value.clone()), ty.clone(), key);
                            let mut binding_type = field_type.clone();
                            if let Some(default) = &property.value {
                                let default = self.lower_expr(default)?;
                                field_value = self.lower_undefined_default(field_value, default)?;
                                binding_type = self.infer_expr_type(&field_value)?;
                            }
                            self.lower_assignment_pattern(
                                &Pat::Ident(property.key.clone()),
                                field_value,
                                &binding_type,
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
                if let HirType::Union(elements) = ty {
                    if elements
                        .iter()
                        .all(|element| matches!(element, HirType::Tuple(_)))
                    {
                        return self.lower_union_tuple_assignment_pattern(
                            pattern, value, elements, statements,
                        );
                    }
                }
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
            Pat::Assign(assign) => {
                let default = self.lower_expr(&assign.right)?;
                let default_type = self.infer_expr_type(&default)?;
                let value = self.lower_undefined_default(value, default)?;
                let value_type = self.infer_expr_type(&value)?;
                self.lower_assignment_pattern(&assign.left, value, &value_type, statements)?;
                if let Pat::Ident(binding) = assign.left.as_ref() {
                    self.destructuring_default_types
                        .insert(self.resolve_binding(binding.id.sym.as_ref()), default_type);
                }
                Ok(())
            }
            Pat::Rest(_) => Err("rest patterns are only valid inside object/array patterns".into()),
            _ => Err("unsupported destructuring assignment target".into()),
        }
    }

    fn lower_update(&mut self, update: &swc_ecma_ast::UpdateExpr) -> Result<HirExpr, String> {
        if self.unbound_this_context {
            if let Expr::Member(member) = update.arg.as_ref() {
                if matches!(member.obj.as_ref(), Expr::This(_)) {
                    let property = member_property_name(&member.prop)
                        .ok_or("unbound `this` update requires a statically known property")?;
                    return self.lower_unbound_this_member(&property);
                }
            }
        }
        if let Expr::Member(member) = update.arg.as_ref() {
            let static_class = match member.obj.as_ref() {
                Expr::Ident(receiver) => Some(receiver.sym.to_string()),
                Expr::This(_) if self.class_static_context => self.class_context.clone(),
                _ => None,
            };
            if let (Some(receiver), Some(property)) =
                (static_class, member_property_name(&member.prop))
            {
                let getter = class_getter_symbol(&receiver, &property, true);
                if let Some(getter_signature) = self.signatures.get(&getter).cloned() {
                    let setter = class_setter_symbol(&receiver, &property, true);
                    if !self.signatures.contains_key(&setter) {
                        return Err(format!(
                            "cannot update readonly static member `{}.{}`",
                            receiver, property
                        ));
                    }
                    if getter_signature.ret != HirType::F64 {
                        return Err(format!(
                            "cannot apply ++/-- to non-number static member `{}.{}`",
                            receiver, property
                        ));
                    }
                    let operator = match update.op {
                        UpdateOp::PlusPlus => BinOp::Add,
                        UpdateOp::MinusMinus => BinOp::Sub,
                    };
                    let current = HirExpr::Call(Box::new(HirExpr::Var(getter)), Vec::new());
                    let one = HirExpr::Lit(HirLit::F64(1.0));
                    if update.prefix {
                        let updated = HirExpr::BinOp(operator, Box::new(current), Box::new(one));
                        return Ok(HirExpr::Call(Box::new(HirExpr::Var(setter)), vec![updated]));
                    }
                    let old_name = format!("__thaw_static_update_old_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(old_name.clone(), HirType::F64);
                    let old = HirExpr::Var(old_name.clone());
                    let updated = HirExpr::BinOp(operator, Box::new(old.clone()), Box::new(one));
                    let result = HirExpr::Block(vec![
                        HirStmt::Expr(HirExpr::Call(Box::new(HirExpr::Var(setter)), vec![updated])),
                        HirStmt::Return(Some(old)),
                    ]);
                    return self
                        .wrap_call_argument_bindings(result, &[(old_name, HirType::F64, current)]);
                }
            }
        }
        let target = match update.arg.as_ref() {
            Expr::Ident(ident) => Target::Var(self.resolve_binding(ident.sym.as_ref())),
            Expr::Member(member) => {
                let static_this_target = (matches!(member.obj.as_ref(), Expr::This(_))
                    && self.class_static_context)
                    .then(|| {
                        let class = self
                            .class_context
                            .as_deref()
                            .expect("static update retains its class context");
                        member_property_name(&member.prop)
                            .map(|property| class_static_field_symbol(class, &property))
                    })
                    .flatten()
                    .filter(|symbol| self.scope.contains_key(symbol));
                if let Some(symbol) = static_this_target {
                    Target::Var(symbol)
                } else if let (Expr::Ident(class), Some(property)) =
                    (member.obj.as_ref(), member_property_name(&member.prop))
                {
                    let symbol = class_static_field_symbol(class.sym.as_ref(), &property);
                    if self.scope.contains_key(&symbol) {
                        Target::Var(symbol)
                    } else {
                        match &member.prop {
                            MemberProp::Computed(computed) => {
                                self.lower_computed_target(member, computed)?
                            }
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
                                    HirType::Dictionary(element)
                                        if element.as_ref() == &HirType::F64 =>
                                    {
                                        Target::Dictionary(
                                            object,
                                            Box::new(HirExpr::Lit(HirLit::Str(
                                                prop.sym.to_string(),
                                            ))),
                                            HirType::F64,
                                        )
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
                        }
                    }
                } else {
                    match &member.prop {
                        MemberProp::Computed(computed) => {
                            self.lower_computed_target(member, computed)?
                        }
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
                                HirType::Dictionary(element)
                                    if element.as_ref() == &HirType::F64 =>
                                {
                                    Target::Dictionary(
                                        object,
                                        Box::new(HirExpr::Lit(HirLit::Str(prop.sym.to_string()))),
                                        HirType::F64,
                                    )
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
                    }
                }
            }
            _ => return Err("unsupported ++/-- target".into()),
        };
        if let Target::Var(name) = &target {
            if self.immutable_bindings.contains(name) {
                return Err(format!("cannot update constant `{name}`"));
            }
        }

        let op = match update.op {
            UpdateOp::PlusPlus => BinOp::Add,
            UpdateOp::MinusMinus => BinOp::Sub,
        };
        let one = HirExpr::Lit(HirLit::F64(1.0));
        let current = target_to_read_expr(&target)?;
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
                bindings.push((index_name.clone(), HirType::F64, *index));
                Target::Index(HirExpr::Var(array_name), Box::new(HirExpr::Var(index_name)))
            }
            Target::Prop(object, object_type, field) => {
                let object_name = format!("__thaw_update_object_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(object_name.clone(), object_type.clone());
                bindings.push((object_name.clone(), object_type.clone(), object));
                Target::Prop(HirExpr::Var(object_name), object_type, field)
            }
            Target::Dictionary(object, key, element) => {
                let object_name = format!("__thaw_update_dictionary_{}", self.next_binding);
                self.next_binding += 1;
                let object_type = HirType::Dictionary(Box::new(element.clone()));
                self.scope.insert(object_name.clone(), object_type.clone());
                bindings.push((object_name.clone(), object_type, object));

                let key_name = format!("__thaw_update_key_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(key_name.clone(), HirType::Str);
                bindings.push((key_name.clone(), HirType::Str, *key));
                Target::Dictionary(
                    HirExpr::Var(object_name),
                    Box::new(HirExpr::Var(key_name)),
                    element,
                )
            }
        };
        let old_name = format!("__thaw_update_old_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(old_name.clone(), HirType::F64);
        bindings.push((
            old_name.clone(),
            HirType::F64,
            target_to_read_expr(&target)?,
        ));
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
                if matches!(&result, HirExpr::Block(_)) {
                    result
                } else {
                    HirExpr::Block(vec![HirStmt::Expr(result)])
                }
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

    fn lower_array_sort_comparator(
        &mut self,
        receiver: HirExpr,
        array_type: HirType,
        element_type: HirType,
        comparator: HirExpr,
        copy: bool,
    ) -> Result<HirExpr, String> {
        let receiver_name = format!("__thaw_sort_receiver_{}", self.next_binding);
        self.next_binding += 1;
        let comparator_name = format!("__thaw_sort_comparator_{}", self.next_binding);
        self.next_binding += 1;
        let comparator_type = HirType::Function(
            vec![element_type.clone(), element_type.clone()],
            Box::new(HirType::F64),
        );
        self.scope.insert(receiver_name.clone(), array_type.clone());
        self.scope
            .insert(comparator_name.clone(), comparator_type.clone());

        let array_name = format!("__thaw_sort_array_{}", self.next_binding);
        self.next_binding += 1;
        let length_name = format!("__thaw_sort_length_{}", self.next_binding);
        self.next_binding += 1;
        let outer_name = format!("__thaw_sort_outer_{}", self.next_binding);
        self.next_binding += 1;
        let inner_name = format!("__thaw_sort_inner_{}", self.next_binding);
        self.next_binding += 1;
        let left_name = format!("__thaw_sort_left_{}", self.next_binding);
        self.next_binding += 1;
        let right_name = format!("__thaw_sort_right_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(array_name.clone(), array_type.clone());
        self.scope.insert(length_name.clone(), HirType::F64);
        self.scope.insert(outer_name.clone(), HirType::F64);
        self.scope.insert(inner_name.clone(), HirType::F64);
        self.scope.insert(left_name.clone(), element_type.clone());
        self.scope.insert(right_name.clone(), element_type.clone());

        let number = |value| HirExpr::Lit(HirLit::F64(value));
        let variable = |name: &str| HirExpr::Var(name.to_string());
        let add_one =
            |value: HirExpr| HirExpr::BinOp(BinOp::Add, Box::new(value), Box::new(number(1.0)));
        let working_source = if copy {
            HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_array_slice".into())),
                vec![variable(&receiver_name), number(0.0), number(f64::INFINITY)],
            )
        } else {
            variable(&receiver_name)
        };
        let inner_index = variable(&inner_name);
        let next_index = add_one(inner_index.clone());
        let left_value = HirExpr::TypedIndex(
            Box::new(variable(&array_name)),
            Box::new(inner_index.clone()),
            element_type.clone(),
        );
        let right_value = HirExpr::TypedIndex(
            Box::new(variable(&array_name)),
            Box::new(next_index.clone()),
            element_type.clone(),
        );
        let compare = HirExpr::Call(
            Box::new(variable(&comparator_name)),
            vec![variable(&left_name), variable(&right_name)],
        );
        let should_swap = HirExpr::BinOp(BinOp::Gt, Box::new(compare), Box::new(number(0.0)));
        let inner_limit = HirExpr::BinOp(
            BinOp::Sub,
            Box::new(variable(&length_name)),
            Box::new(variable(&outer_name)),
        );
        let inner_condition = HirExpr::BinOp(
            BinOp::Lt,
            Box::new(add_one(variable(&inner_name))),
            Box::new(inner_limit),
        );
        let outer_condition = HirExpr::BinOp(
            BinOp::Lt,
            Box::new(variable(&outer_name)),
            Box::new(variable(&length_name)),
        );
        let body = HirExpr::Block(vec![
            HirStmt::Let(array_name.clone(), array_type.clone(), working_source),
            HirStmt::Let(
                length_name.clone(),
                HirType::F64,
                HirExpr::ArrayLen(Box::new(variable(&array_name))),
            ),
            HirStmt::Let(outer_name.clone(), HirType::F64, number(0.0)),
            HirStmt::While(
                outer_condition,
                vec![
                    HirStmt::Let(inner_name.clone(), HirType::F64, number(0.0)),
                    HirStmt::While(
                        inner_condition,
                        vec![
                            HirStmt::Let(left_name.clone(), element_type.clone(), left_value),
                            HirStmt::Let(right_name.clone(), element_type.clone(), right_value),
                            HirStmt::If(
                                should_swap,
                                vec![
                                    HirStmt::Expr(HirExpr::IndexAssign(
                                        Box::new(variable(&array_name)),
                                        Box::new(variable(&inner_name)),
                                        Box::new(variable(&right_name)),
                                    )),
                                    HirStmt::Expr(HirExpr::IndexAssign(
                                        Box::new(variable(&array_name)),
                                        Box::new(add_one(variable(&inner_name))),
                                        Box::new(variable(&left_name)),
                                    )),
                                ],
                                Vec::new(),
                            ),
                            HirStmt::Expr(HirExpr::Assign(
                                inner_name.clone(),
                                Box::new(add_one(variable(&inner_name))),
                            )),
                        ],
                    ),
                    HirStmt::Expr(HirExpr::Assign(
                        outer_name.clone(),
                        Box::new(add_one(variable(&outer_name))),
                    )),
                ],
            ),
            HirStmt::Return(Some(variable(&array_name))),
        ]);
        self.wrap_call_argument_bindings(
            body,
            &[
                (receiver_name, array_type, receiver),
                (comparator_name, comparator_type, comparator),
            ],
        )
    }

    fn lower_array_callback(
        &mut self,
        expr: &Expr,
        element_type: &HirType,
        array_type: &HirType,
        expected_return: &HirType,
    ) -> Result<HirExpr, String> {
        let arity = match expr {
            Expr::Arrow(arrow) => arrow.params.len(),
            Expr::Fn(function) => function.function.params.len(),
            Expr::Ident(ident) => {
                let name = self.resolve_binding(ident.sym.as_ref());
                self.scope
                    .get(&name)
                    .and_then(|ty| match ty {
                        HirType::Function(params, _) => Some(params.len()),
                        _ => None,
                    })
                    .or_else(|| {
                        self.generic_arrows
                            .get(&name)
                            .map(|arrow| arrow.params.len())
                    })
                    .or_else(|| {
                        self.generic_named_templates
                            .get(&name)
                            .and_then(|target| self.signatures.get(target))
                            .map(|signature| signature.params.len())
                    })
                    .or_else(|| {
                        self.signatures
                            .get(&name)
                            .map(|signature| signature.params.len())
                    })
                    .ok_or_else(|| format!("unknown array predicate `{name}`"))?
            }
            _ => return Err("array predicate must be an arrow or function value".into()),
        };
        if arity > 3 {
            return Err(format!(
                "array predicate accepts at most three parameters, got {arity}"
            ));
        }
        let available = [element_type.clone(), HirType::F64, array_type.clone()];
        self.lower_promise_callback(expr, &available[..arity], Some(expected_return))
    }

    fn lower_array_reducer_callback(
        &mut self,
        expr: &Expr,
        accumulator_type: &HirType,
        element_type: &HirType,
        array_type: &HirType,
    ) -> Result<HirExpr, String> {
        let arity = match expr {
            Expr::Arrow(arrow) => arrow.params.len(),
            Expr::Fn(function) => function.function.params.len(),
            Expr::Ident(ident) => {
                let name = self.resolve_binding(ident.sym.as_ref());
                self.scope
                    .get(&name)
                    .and_then(|ty| match ty {
                        HirType::Function(params, _) => Some(params.len()),
                        _ => None,
                    })
                    .or_else(|| {
                        self.generic_arrows
                            .get(&name)
                            .map(|arrow| arrow.params.len())
                    })
                    .or_else(|| {
                        self.generic_named_templates
                            .get(&name)
                            .and_then(|target| self.signatures.get(target))
                            .map(|signature| signature.params.len())
                    })
                    .or_else(|| {
                        self.signatures
                            .get(&name)
                            .map(|signature| signature.params.len())
                    })
                    .ok_or_else(|| format!("unknown array reducer `{name}`"))?
            }
            _ => return Err("array reducer must be an arrow or function value".into()),
        };
        if arity > 4 {
            return Err(format!(
                "array reducer accepts at most four parameters, got {arity}"
            ));
        }
        let available = [
            accumulator_type.clone(),
            element_type.clone(),
            HirType::F64,
            array_type.clone(),
        ];
        self.lower_promise_callback(expr, &available[..arity], Some(accumulator_type))
    }

    fn lower_array_mapping_callback(
        &mut self,
        expr: &Expr,
        element_type: &HirType,
        array_type: &HirType,
    ) -> Result<HirExpr, String> {
        let arity = match expr {
            Expr::Arrow(arrow) => arrow.params.len(),
            Expr::Fn(function) => function.function.params.len(),
            Expr::Ident(ident) => {
                let name = self.resolve_binding(ident.sym.as_ref());
                self.scope
                    .get(&name)
                    .and_then(|ty| match ty {
                        HirType::Function(params, _) => Some(params.len()),
                        _ => None,
                    })
                    .or_else(|| {
                        self.generic_arrows
                            .get(&name)
                            .map(|arrow| arrow.params.len())
                    })
                    .or_else(|| {
                        self.generic_named_templates
                            .get(&name)
                            .and_then(|target| self.signatures.get(target))
                            .map(|signature| signature.params.len())
                    })
                    .or_else(|| {
                        self.signatures
                            .get(&name)
                            .map(|signature| signature.params.len())
                    })
                    .ok_or_else(|| format!("unknown array mapper `{name}`"))?
            }
            _ => return Err("array mapper must be an arrow or function value".into()),
        };
        if arity > 3 {
            return Err(format!(
                "array mapper accepts at most three parameters, got {arity}"
            ));
        }
        let available = [element_type.clone(), HirType::F64, array_type.clone()];
        self.lower_promise_callback(expr, &available[..arity], None)
    }

    fn lower_array_from_callback(
        &mut self,
        expr: &Expr,
        element_type: &HirType,
    ) -> Result<HirExpr, String> {
        let arity = match expr {
            Expr::Arrow(arrow) => arrow.params.len(),
            Expr::Fn(function) => function.function.params.len(),
            Expr::Ident(ident) => {
                let name = self.resolve_binding(ident.sym.as_ref());
                self.scope
                    .get(&name)
                    .and_then(|ty| match ty {
                        HirType::Function(params, _) => Some(params.len()),
                        _ => None,
                    })
                    .or_else(|| {
                        self.generic_arrows
                            .get(&name)
                            .map(|arrow| arrow.params.len())
                    })
                    .or_else(|| {
                        self.generic_named_templates
                            .get(&name)
                            .and_then(|target| self.signatures.get(target))
                            .map(|signature| signature.params.len())
                    })
                    .or_else(|| {
                        self.signatures
                            .get(&name)
                            .map(|signature| signature.params.len())
                    })
                    .ok_or_else(|| format!("unknown Array.from mapper `{name}`"))?
            }
            _ => return Err("Array.from mapper must be an arrow or function value".into()),
        };
        if arity > 2 {
            return Err(format!(
                "Array.from mapper accepts at most two parameters, got {arity}"
            ));
        }
        let available = [element_type.clone(), HirType::F64];
        self.lower_promise_callback(expr, &available[..arity], None)
    }

    fn lower_array_map(
        &mut self,
        receiver: HirExpr,
        array_type: HirType,
        element_type: HirType,
        callback: HirExpr,
        this_arg: Option<HirExpr>,
    ) -> Result<HirExpr, String> {
        let receiver_name = format!("__thaw_map_receiver_{}", self.next_binding);
        self.next_binding += 1;
        let callback_name = format!("__thaw_map_callback_{}", self.next_binding);
        self.next_binding += 1;
        let callback_type = self.infer_expr_type(&callback)?;
        let HirType::Function(params, output_type) = &callback_type else {
            unreachable!("array mapper was validated as a function")
        };
        if **output_type == HirType::Void {
            return Err("array mapper must return a value".into());
        }
        let output_type = output_type.as_ref().clone();
        self.scope.insert(receiver_name.clone(), array_type.clone());
        self.scope
            .insert(callback_name.clone(), callback_type.clone());
        let length_name = format!("__thaw_map_length_{}", self.next_binding);
        self.next_binding += 1;
        let result_name = format!("__thaw_map_result_{}", self.next_binding);
        self.next_binding += 1;
        let index_name = format!("__thaw_map_index_{}", self.next_binding);
        self.next_binding += 1;
        let element_name = format!("__thaw_map_element_{}", self.next_binding);
        self.next_binding += 1;
        let result_type = HirType::Array(Box::new(output_type.clone()));
        self.scope.insert(length_name.clone(), HirType::F64);
        self.scope.insert(result_name.clone(), result_type.clone());
        self.scope.insert(index_name.clone(), HirType::F64);
        self.scope
            .insert(element_name.clone(), element_type.clone());
        let available = [
            HirExpr::Var(element_name.clone()),
            HirExpr::Var(index_name.clone()),
            HirExpr::Var(receiver_name.clone()),
        ];
        let callback_call = HirExpr::Call(
            Box::new(HirExpr::Var(callback_name.clone())),
            available[..params.len()].to_vec(),
        );
        let body = HirExpr::Block(vec![
            HirStmt::Let(
                length_name.clone(),
                HirType::F64,
                HirExpr::ArrayLen(Box::new(HirExpr::Var(receiver_name.clone()))),
            ),
            HirStmt::Let(
                result_name.clone(),
                result_type,
                HirExpr::ArrayAlloc(Box::new(HirExpr::Var(length_name.clone())), output_type),
            ),
            HirStmt::Let(
                index_name.clone(),
                HirType::F64,
                HirExpr::Lit(HirLit::F64(0.0)),
            ),
            HirStmt::While(
                HirExpr::BinOp(
                    BinOp::Lt,
                    Box::new(HirExpr::Var(index_name.clone())),
                    Box::new(HirExpr::Var(length_name)),
                ),
                vec![
                    HirStmt::Let(
                        element_name,
                        element_type.clone(),
                        HirExpr::TypedIndex(
                            Box::new(HirExpr::Var(receiver_name.clone())),
                            Box::new(HirExpr::Var(index_name.clone())),
                            element_type,
                        ),
                    ),
                    HirStmt::Expr(HirExpr::IndexAssign(
                        Box::new(HirExpr::Var(result_name.clone())),
                        Box::new(HirExpr::Var(index_name.clone())),
                        Box::new(callback_call),
                    )),
                    HirStmt::Expr(HirExpr::Assign(
                        index_name.clone(),
                        Box::new(HirExpr::BinOp(
                            BinOp::Add,
                            Box::new(HirExpr::Var(index_name)),
                            Box::new(HirExpr::Lit(HirLit::F64(1.0))),
                        )),
                    )),
                ],
            ),
            HirStmt::Return(Some(HirExpr::Var(result_name))),
        ]);
        let mut bindings = vec![
            (receiver_name, array_type, receiver),
            (callback_name, callback_type, callback),
        ];
        if let Some(this_arg) = this_arg {
            let ty = self.infer_expr_type(&this_arg)?;
            let name = format!("__thaw_map_this_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(name.clone(), ty.clone());
            bindings.push((name, ty, this_arg));
        }
        self.wrap_call_argument_bindings(body, &bindings)
    }

    fn lower_array_filter(
        &mut self,
        receiver: HirExpr,
        array_type: HirType,
        element_type: HirType,
        callback: HirExpr,
        this_arg: Option<HirExpr>,
    ) -> Result<HirExpr, String> {
        let receiver_name = format!("__thaw_filter_receiver_{}", self.next_binding);
        self.next_binding += 1;
        let callback_name = format!("__thaw_filter_callback_{}", self.next_binding);
        self.next_binding += 1;
        let callback_type = self.infer_expr_type(&callback)?;
        self.scope.insert(receiver_name.clone(), array_type.clone());
        self.scope
            .insert(callback_name.clone(), callback_type.clone());
        let length_name = format!("__thaw_filter_length_{}", self.next_binding);
        self.next_binding += 1;
        let result_name = format!("__thaw_filter_result_{}", self.next_binding);
        self.next_binding += 1;
        let index_name = format!("__thaw_filter_index_{}", self.next_binding);
        self.next_binding += 1;
        let output_index_name = format!("__thaw_filter_output_index_{}", self.next_binding);
        self.next_binding += 1;
        let element_name = format!("__thaw_filter_element_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(length_name.clone(), HirType::F64);
        self.scope.insert(result_name.clone(), array_type.clone());
        self.scope.insert(index_name.clone(), HirType::F64);
        self.scope.insert(output_index_name.clone(), HirType::F64);
        self.scope
            .insert(element_name.clone(), element_type.clone());
        let HirType::Function(params, _) = &callback_type else {
            unreachable!("array filter was validated as a function")
        };
        let available = [
            HirExpr::Var(element_name.clone()),
            HirExpr::Var(index_name.clone()),
            HirExpr::Var(receiver_name.clone()),
        ];
        let callback_call = HirExpr::Call(
            Box::new(HirExpr::Var(callback_name.clone())),
            available[..params.len()].to_vec(),
        );
        let increment = |name: &str| {
            HirStmt::Expr(HirExpr::Assign(
                name.into(),
                Box::new(HirExpr::BinOp(
                    BinOp::Add,
                    Box::new(HirExpr::Var(name.into())),
                    Box::new(HirExpr::Lit(HirLit::F64(1.0))),
                )),
            ))
        };
        let body = HirExpr::Block(vec![
            HirStmt::Let(
                length_name.clone(),
                HirType::F64,
                HirExpr::ArrayLen(Box::new(HirExpr::Var(receiver_name.clone()))),
            ),
            HirStmt::Let(
                result_name.clone(),
                array_type.clone(),
                HirExpr::ArrayAlloc(
                    Box::new(HirExpr::Var(length_name.clone())),
                    element_type.clone(),
                ),
            ),
            HirStmt::Let(
                index_name.clone(),
                HirType::F64,
                HirExpr::Lit(HirLit::F64(0.0)),
            ),
            HirStmt::Let(
                output_index_name.clone(),
                HirType::F64,
                HirExpr::Lit(HirLit::F64(0.0)),
            ),
            HirStmt::While(
                HirExpr::BinOp(
                    BinOp::Lt,
                    Box::new(HirExpr::Var(index_name.clone())),
                    Box::new(HirExpr::Var(length_name)),
                ),
                vec![
                    HirStmt::Let(
                        element_name.clone(),
                        element_type.clone(),
                        HirExpr::TypedIndex(
                            Box::new(HirExpr::Var(receiver_name.clone())),
                            Box::new(HirExpr::Var(index_name.clone())),
                            element_type.clone(),
                        ),
                    ),
                    HirStmt::If(
                        callback_call,
                        vec![
                            HirStmt::Expr(HirExpr::IndexAssign(
                                Box::new(HirExpr::Var(result_name.clone())),
                                Box::new(HirExpr::Var(output_index_name.clone())),
                                Box::new(HirExpr::Var(element_name)),
                            )),
                            increment(&output_index_name),
                        ],
                        Vec::new(),
                    ),
                    increment(&index_name),
                ],
            ),
            HirStmt::Return(Some(HirExpr::ArraySetLen(
                Box::new(HirExpr::Var(result_name)),
                Box::new(HirExpr::Var(output_index_name)),
                element_type,
            ))),
        ]);
        let mut bindings = vec![
            (receiver_name, array_type, receiver),
            (callback_name, callback_type, callback),
        ];
        if let Some(this_arg) = this_arg {
            let ty = self.infer_expr_type(&this_arg)?;
            let name = format!("__thaw_filter_this_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(name.clone(), ty.clone());
            bindings.push((name, ty, this_arg));
        }
        self.wrap_call_argument_bindings(body, &bindings)
    }

    fn lower_array_flat_one(
        &mut self,
        receiver: HirExpr,
        nested_array_type: HirType,
        element_type: HirType,
    ) -> Result<HirExpr, String> {
        let receiver_name = format!("__thaw_flat_receiver_{}", self.next_binding);
        self.next_binding += 1;
        self.scope
            .insert(receiver_name.clone(), nested_array_type.clone());
        let outer_length_name = format!("__thaw_flat_outer_length_{}", self.next_binding);
        self.next_binding += 1;
        let total_length_name = format!("__thaw_flat_total_length_{}", self.next_binding);
        self.next_binding += 1;
        let outer_index_name = format!("__thaw_flat_outer_index_{}", self.next_binding);
        self.next_binding += 1;
        let inner_array_name = format!("__thaw_flat_inner_array_{}", self.next_binding);
        self.next_binding += 1;
        let inner_length_name = format!("__thaw_flat_inner_length_{}", self.next_binding);
        self.next_binding += 1;
        let inner_index_name = format!("__thaw_flat_inner_index_{}", self.next_binding);
        self.next_binding += 1;
        let destination_name = format!("__thaw_flat_destination_{}", self.next_binding);
        self.next_binding += 1;
        let result_name = format!("__thaw_flat_result_{}", self.next_binding);
        self.next_binding += 1;
        for name in [
            &outer_length_name,
            &total_length_name,
            &outer_index_name,
            &inner_length_name,
            &inner_index_name,
            &destination_name,
        ] {
            self.scope.insert(name.clone(), HirType::F64);
        }
        let inner_array_type = HirType::Array(Box::new(element_type.clone()));
        let result_type = inner_array_type.clone();
        self.scope
            .insert(inner_array_name.clone(), inner_array_type.clone());
        self.scope.insert(result_name.clone(), result_type.clone());
        let number = |value| HirExpr::Lit(HirLit::F64(value));
        let var = |name: &str| HirExpr::Var(name.into());
        let add = |left, right| HirExpr::BinOp(BinOp::Add, Box::new(left), Box::new(right));
        let assign =
            |name: &str, value| HirStmt::Expr(HirExpr::Assign(name.into(), Box::new(value)));
        let increment = |name: &str| assign(name, add(var(name), number(1.0)));
        let load_inner = || {
            HirExpr::TypedIndex(
                Box::new(var(&receiver_name)),
                Box::new(var(&outer_index_name)),
                inner_array_type.clone(),
            )
        };
        let body = HirExpr::Block(vec![
            HirStmt::Let(
                outer_length_name.clone(),
                HirType::F64,
                HirExpr::ArrayLen(Box::new(var(&receiver_name))),
            ),
            HirStmt::Let(total_length_name.clone(), HirType::F64, number(0.0)),
            HirStmt::Let(outer_index_name.clone(), HirType::F64, number(0.0)),
            HirStmt::While(
                HirExpr::BinOp(
                    BinOp::Lt,
                    Box::new(var(&outer_index_name)),
                    Box::new(var(&outer_length_name)),
                ),
                vec![
                    HirStmt::Let(
                        inner_array_name.clone(),
                        inner_array_type.clone(),
                        load_inner(),
                    ),
                    assign(
                        &total_length_name,
                        add(
                            var(&total_length_name),
                            HirExpr::ArrayLen(Box::new(var(&inner_array_name))),
                        ),
                    ),
                    increment(&outer_index_name),
                ],
            ),
            HirStmt::Let(
                result_name.clone(),
                result_type,
                HirExpr::ArrayAlloc(Box::new(var(&total_length_name)), element_type.clone()),
            ),
            assign(&outer_index_name, number(0.0)),
            HirStmt::Let(destination_name.clone(), HirType::F64, number(0.0)),
            HirStmt::While(
                HirExpr::BinOp(
                    BinOp::Lt,
                    Box::new(var(&outer_index_name)),
                    Box::new(var(&outer_length_name)),
                ),
                vec![
                    HirStmt::Let(
                        inner_array_name.clone(),
                        inner_array_type.clone(),
                        load_inner(),
                    ),
                    HirStmt::Let(
                        inner_length_name.clone(),
                        HirType::F64,
                        HirExpr::ArrayLen(Box::new(var(&inner_array_name))),
                    ),
                    HirStmt::Let(inner_index_name.clone(), HirType::F64, number(0.0)),
                    HirStmt::While(
                        HirExpr::BinOp(
                            BinOp::Lt,
                            Box::new(var(&inner_index_name)),
                            Box::new(var(&inner_length_name)),
                        ),
                        vec![
                            HirStmt::Expr(HirExpr::IndexAssign(
                                Box::new(var(&result_name)),
                                Box::new(var(&destination_name)),
                                Box::new(HirExpr::TypedIndex(
                                    Box::new(var(&inner_array_name)),
                                    Box::new(var(&inner_index_name)),
                                    element_type.clone(),
                                )),
                            )),
                            increment(&inner_index_name),
                            increment(&destination_name),
                        ],
                    ),
                    increment(&outer_index_name),
                ],
            ),
            HirStmt::Return(Some(var(&result_name))),
        ]);
        self.wrap_call_argument_bindings(body, &[(receiver_name, nested_array_type, receiver)])
    }

    fn lower_array_with(
        &mut self,
        receiver: HirExpr,
        array_type: HirType,
        element_type: HirType,
        index: HirExpr,
        value: HirExpr,
    ) -> Result<HirExpr, String> {
        let receiver_name = format!("__thaw_with_receiver_{}", self.next_binding);
        self.next_binding += 1;
        let index_argument_name = format!("__thaw_with_index_argument_{}", self.next_binding);
        self.next_binding += 1;
        let value_name = format!("__thaw_with_value_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(receiver_name.clone(), array_type.clone());
        self.scope.insert(index_argument_name.clone(), HirType::F64);
        self.scope.insert(value_name.clone(), element_type.clone());
        let length_name = format!("__thaw_with_length_{}", self.next_binding);
        self.next_binding += 1;
        let actual_index_name = format!("__thaw_with_actual_index_{}", self.next_binding);
        self.next_binding += 1;
        let copy_index_name = format!("__thaw_with_copy_index_{}", self.next_binding);
        self.next_binding += 1;
        let result_name = format!("__thaw_with_result_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(length_name.clone(), HirType::F64);
        self.scope.insert(actual_index_name.clone(), HirType::F64);
        self.scope.insert(copy_index_name.clone(), HirType::F64);
        self.scope.insert(result_name.clone(), array_type.clone());
        let number = |value| HirExpr::Lit(HirLit::F64(value));
        let var = |name: &str| HirExpr::Var(name.into());
        let assign =
            |name: &str, value| HirStmt::Expr(HirExpr::Assign(name.into(), Box::new(value)));
        let range_error = || {
            HirStmt::Throw(HirExpr::Lit(HirLit::Str(
                "Invalid index for Array.prototype.with".into(),
            )))
        };
        let body = HirExpr::Block(vec![
            HirStmt::Let(
                length_name.clone(),
                HirType::F64,
                HirExpr::ArrayLen(Box::new(var(&receiver_name))),
            ),
            HirStmt::Let(
                actual_index_name.clone(),
                HirType::F64,
                var(&index_argument_name),
            ),
            // ToIntegerOrInfinity maps NaN to +0; finite fractional indices
            // are truncated by the typed element-address conversion.
            HirStmt::If(
                HirExpr::BinOp(
                    BinOp::EqEqEq,
                    Box::new(var(&actual_index_name)),
                    Box::new(var(&actual_index_name)),
                ),
                Vec::new(),
                vec![assign(&actual_index_name, number(0.0))],
            ),
            assign(
                &actual_index_name,
                HirExpr::Call(
                    Box::new(HirExpr::Var("__thaw_math_trunc".into())),
                    vec![var(&actual_index_name)],
                ),
            ),
            HirStmt::If(
                HirExpr::BinOp(
                    BinOp::Lt,
                    Box::new(var(&actual_index_name)),
                    Box::new(number(0.0)),
                ),
                vec![assign(
                    &actual_index_name,
                    HirExpr::BinOp(
                        BinOp::Add,
                        Box::new(var(&length_name)),
                        Box::new(var(&actual_index_name)),
                    ),
                )],
                Vec::new(),
            ),
            HirStmt::If(
                HirExpr::BinOp(
                    BinOp::Lt,
                    Box::new(var(&actual_index_name)),
                    Box::new(number(0.0)),
                ),
                vec![range_error()],
                Vec::new(),
            ),
            HirStmt::If(
                HirExpr::BinOp(
                    BinOp::GtEq,
                    Box::new(var(&actual_index_name)),
                    Box::new(var(&length_name)),
                ),
                vec![range_error()],
                Vec::new(),
            ),
            HirStmt::Let(
                result_name.clone(),
                array_type.clone(),
                HirExpr::ArrayAlloc(Box::new(var(&length_name)), element_type.clone()),
            ),
            HirStmt::Let(copy_index_name.clone(), HirType::F64, number(0.0)),
            HirStmt::While(
                HirExpr::BinOp(
                    BinOp::Lt,
                    Box::new(var(&copy_index_name)),
                    Box::new(var(&length_name)),
                ),
                vec![
                    HirStmt::If(
                        HirExpr::BinOp(
                            BinOp::EqEqEq,
                            Box::new(var(&copy_index_name)),
                            Box::new(var(&actual_index_name)),
                        ),
                        vec![HirStmt::Expr(HirExpr::IndexAssign(
                            Box::new(var(&result_name)),
                            Box::new(var(&copy_index_name)),
                            Box::new(var(&value_name)),
                        ))],
                        vec![HirStmt::Expr(HirExpr::IndexAssign(
                            Box::new(var(&result_name)),
                            Box::new(var(&copy_index_name)),
                            Box::new(HirExpr::TypedIndex(
                                Box::new(var(&receiver_name)),
                                Box::new(var(&copy_index_name)),
                                element_type.clone(),
                            )),
                        ))],
                    ),
                    assign(
                        &copy_index_name,
                        HirExpr::BinOp(
                            BinOp::Add,
                            Box::new(var(&copy_index_name)),
                            Box::new(number(1.0)),
                        ),
                    ),
                ],
            ),
            HirStmt::Return(Some(var(&result_name))),
        ]);
        self.wrap_call_argument_bindings(
            body,
            &[
                (receiver_name, array_type, receiver),
                (index_argument_name, HirType::F64, index),
                (value_name, element_type, value),
            ],
        )
    }

    fn lower_array_at(
        &mut self,
        receiver: HirExpr,
        array_type: HirType,
        element_type: HirType,
        index: HirExpr,
    ) -> Result<HirExpr, String> {
        let receiver_name = format!("__thaw_at_receiver_{}", self.next_binding);
        self.next_binding += 1;
        let index_argument_name = format!("__thaw_at_index_argument_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(receiver_name.clone(), array_type.clone());
        self.scope.insert(index_argument_name.clone(), HirType::F64);
        let length_name = format!("__thaw_at_length_{}", self.next_binding);
        self.next_binding += 1;
        let actual_index_name = format!("__thaw_at_actual_index_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(length_name.clone(), HirType::F64);
        self.scope.insert(actual_index_name.clone(), HirType::F64);
        let number = |value| HirExpr::Lit(HirLit::F64(value));
        let var = |name: &str| HirExpr::Var(name.into());
        let assign =
            |value| HirStmt::Expr(HirExpr::Assign(actual_index_name.clone(), Box::new(value)));
        let none = || HirStmt::Return(Some(HirExpr::OptionalNone(element_type.clone())));
        let body = HirExpr::Block(vec![
            HirStmt::Let(
                length_name.clone(),
                HirType::F64,
                HirExpr::ArrayLen(Box::new(var(&receiver_name))),
            ),
            HirStmt::Let(
                actual_index_name.clone(),
                HirType::F64,
                var(&index_argument_name),
            ),
            HirStmt::If(
                HirExpr::BinOp(
                    BinOp::EqEqEq,
                    Box::new(var(&actual_index_name)),
                    Box::new(var(&actual_index_name)),
                ),
                Vec::new(),
                vec![assign(number(0.0))],
            ),
            assign(HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_math_trunc".into())),
                vec![var(&actual_index_name)],
            )),
            HirStmt::If(
                HirExpr::BinOp(
                    BinOp::Lt,
                    Box::new(var(&actual_index_name)),
                    Box::new(number(0.0)),
                ),
                vec![assign(HirExpr::BinOp(
                    BinOp::Add,
                    Box::new(var(&length_name)),
                    Box::new(var(&actual_index_name)),
                ))],
                Vec::new(),
            ),
            HirStmt::If(
                HirExpr::BinOp(
                    BinOp::Lt,
                    Box::new(var(&actual_index_name)),
                    Box::new(number(0.0)),
                ),
                vec![none()],
                Vec::new(),
            ),
            HirStmt::If(
                HirExpr::BinOp(
                    BinOp::GtEq,
                    Box::new(var(&actual_index_name)),
                    Box::new(var(&length_name)),
                ),
                vec![none()],
                Vec::new(),
            ),
            HirStmt::Return(Some(HirExpr::OptionalSome(
                Box::new(HirExpr::TypedIndex(
                    Box::new(var(&receiver_name)),
                    Box::new(var(&actual_index_name)),
                    element_type.clone(),
                )),
                element_type,
            ))),
        ]);
        self.wrap_call_argument_bindings(
            body,
            &[
                (receiver_name, array_type, receiver),
                (index_argument_name, HirType::F64, index),
            ],
        )
    }

    fn lower_array_to_spliced(
        &mut self,
        receiver: HirExpr,
        array_type: HirType,
        element_type: HirType,
        arguments: Vec<HirExpr>,
    ) -> Result<HirExpr, String> {
        let receiver_name = format!("__thaw_to_spliced_receiver_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(receiver_name.clone(), array_type.clone());
        let mut bindings = vec![(receiver_name.clone(), array_type.clone(), receiver)];
        let mut argument_names = Vec::with_capacity(arguments.len());
        for (index, argument) in arguments.into_iter().enumerate() {
            let ty = if index < 2 {
                HirType::F64
            } else {
                element_type.clone()
            };
            let name = format!("__thaw_to_spliced_argument_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(name.clone(), ty.clone());
            argument_names.push(name.clone());
            bindings.push((name, ty, argument));
        }
        let length_name = format!("__thaw_to_spliced_length_{}", self.next_binding);
        self.next_binding += 1;
        let start_name = format!("__thaw_to_spliced_start_{}", self.next_binding);
        self.next_binding += 1;
        let delete_name = format!("__thaw_to_spliced_delete_{}", self.next_binding);
        self.next_binding += 1;
        let result_length_name = format!("__thaw_to_spliced_result_length_{}", self.next_binding);
        self.next_binding += 1;
        let result_name = format!("__thaw_to_spliced_result_{}", self.next_binding);
        self.next_binding += 1;
        let source_index_name = format!("__thaw_to_spliced_source_{}", self.next_binding);
        self.next_binding += 1;
        let destination_index_name = format!("__thaw_to_spliced_destination_{}", self.next_binding);
        self.next_binding += 1;
        for name in [
            &length_name,
            &start_name,
            &delete_name,
            &result_length_name,
            &source_index_name,
            &destination_index_name,
        ] {
            self.scope.insert(name.clone(), HirType::F64);
        }
        self.scope.insert(result_name.clone(), array_type.clone());
        let number = |value| HirExpr::Lit(HirLit::F64(value));
        let var = |name: &str| HirExpr::Var(name.into());
        let assign =
            |name: &str, value| HirStmt::Expr(HirExpr::Assign(name.into(), Box::new(value)));
        let add = |left, right| HirExpr::BinOp(BinOp::Add, Box::new(left), Box::new(right));
        let sub = |left, right| HirExpr::BinOp(BinOp::Sub, Box::new(left), Box::new(right));
        let increment = |name: &str| assign(name, add(var(name), number(1.0)));
        let trunc = |value| {
            HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_math_trunc".into())),
                vec![value],
            )
        };
        let initial_start = argument_names
            .first()
            .map(|name| var(name))
            .unwrap_or_else(|| number(0.0));
        let mut statements = vec![
            HirStmt::Let(
                length_name.clone(),
                HirType::F64,
                HirExpr::ArrayLen(Box::new(var(&receiver_name))),
            ),
            HirStmt::Let(start_name.clone(), HirType::F64, initial_start),
            HirStmt::If(
                HirExpr::BinOp(
                    BinOp::EqEqEq,
                    Box::new(var(&start_name)),
                    Box::new(var(&start_name)),
                ),
                Vec::new(),
                vec![assign(&start_name, number(0.0))],
            ),
            assign(&start_name, trunc(var(&start_name))),
            HirStmt::If(
                HirExpr::BinOp(BinOp::Lt, Box::new(var(&start_name)), Box::new(number(0.0))),
                vec![
                    assign(&start_name, add(var(&length_name), var(&start_name))),
                    HirStmt::If(
                        HirExpr::BinOp(
                            BinOp::Lt,
                            Box::new(var(&start_name)),
                            Box::new(number(0.0)),
                        ),
                        vec![assign(&start_name, number(0.0))],
                        Vec::new(),
                    ),
                ],
                vec![HirStmt::If(
                    HirExpr::BinOp(
                        BinOp::Gt,
                        Box::new(var(&start_name)),
                        Box::new(var(&length_name)),
                    ),
                    vec![assign(&start_name, var(&length_name))],
                    Vec::new(),
                )],
            ),
        ];
        let initial_delete = match argument_names.len() {
            0 => number(0.0),
            1 => sub(var(&length_name), var(&start_name)),
            _ => var(&argument_names[1]),
        };
        statements.extend([
            HirStmt::Let(delete_name.clone(), HirType::F64, initial_delete),
            HirStmt::If(
                HirExpr::BinOp(
                    BinOp::EqEqEq,
                    Box::new(var(&delete_name)),
                    Box::new(var(&delete_name)),
                ),
                Vec::new(),
                vec![assign(&delete_name, number(0.0))],
            ),
            assign(&delete_name, trunc(var(&delete_name))),
            HirStmt::If(
                HirExpr::BinOp(
                    BinOp::Lt,
                    Box::new(var(&delete_name)),
                    Box::new(number(0.0)),
                ),
                vec![assign(&delete_name, number(0.0))],
                Vec::new(),
            ),
            HirStmt::If(
                HirExpr::BinOp(
                    BinOp::Gt,
                    Box::new(var(&delete_name)),
                    Box::new(sub(var(&length_name), var(&start_name))),
                ),
                vec![assign(
                    &delete_name,
                    sub(var(&length_name), var(&start_name)),
                )],
                Vec::new(),
            ),
            HirStmt::Let(
                result_length_name.clone(),
                HirType::F64,
                add(
                    sub(var(&length_name), var(&delete_name)),
                    number(argument_names.len().saturating_sub(2) as f64),
                ),
            ),
            HirStmt::Let(
                result_name.clone(),
                array_type.clone(),
                HirExpr::ArrayAlloc(Box::new(var(&result_length_name)), element_type.clone()),
            ),
            HirStmt::Let(source_index_name.clone(), HirType::F64, number(0.0)),
            HirStmt::Let(destination_index_name.clone(), HirType::F64, number(0.0)),
            HirStmt::While(
                HirExpr::BinOp(
                    BinOp::Lt,
                    Box::new(var(&source_index_name)),
                    Box::new(var(&start_name)),
                ),
                vec![
                    HirStmt::Expr(HirExpr::IndexAssign(
                        Box::new(var(&result_name)),
                        Box::new(var(&destination_index_name)),
                        Box::new(HirExpr::TypedIndex(
                            Box::new(var(&receiver_name)),
                            Box::new(var(&source_index_name)),
                            element_type.clone(),
                        )),
                    )),
                    increment(&source_index_name),
                    increment(&destination_index_name),
                ],
            ),
        ]);
        for item_name in argument_names.iter().skip(2) {
            statements.push(HirStmt::Expr(HirExpr::IndexAssign(
                Box::new(var(&result_name)),
                Box::new(var(&destination_index_name)),
                Box::new(var(item_name)),
            )));
            statements.push(increment(&destination_index_name));
        }
        statements.extend([
            assign(&source_index_name, add(var(&start_name), var(&delete_name))),
            HirStmt::While(
                HirExpr::BinOp(
                    BinOp::Lt,
                    Box::new(var(&source_index_name)),
                    Box::new(var(&length_name)),
                ),
                vec![
                    HirStmt::Expr(HirExpr::IndexAssign(
                        Box::new(var(&result_name)),
                        Box::new(var(&destination_index_name)),
                        Box::new(HirExpr::TypedIndex(
                            Box::new(var(&receiver_name)),
                            Box::new(var(&source_index_name)),
                            element_type,
                        )),
                    )),
                    increment(&source_index_name),
                    increment(&destination_index_name),
                ],
            ),
            HirStmt::Return(Some(var(&result_name))),
        ]);
        self.wrap_call_argument_bindings(HirExpr::Block(statements), &bindings)
    }

    fn lower_array_reduce(
        &mut self,
        receiver: HirExpr,
        array_type: HirType,
        element_type: HirType,
        callback: HirExpr,
        initial: Option<(HirExpr, HirType)>,
        reverse: bool,
    ) -> Result<HirExpr, String> {
        let receiver_name = format!("__thaw_reduce_receiver_{}", self.next_binding);
        self.next_binding += 1;
        let callback_name = format!("__thaw_reduce_callback_{}", self.next_binding);
        self.next_binding += 1;
        let callback_type = self.infer_expr_type(&callback)?;
        let accumulator_type = initial
            .as_ref()
            .map(|(_, ty)| ty.clone())
            .unwrap_or_else(|| element_type.clone());
        self.scope.insert(receiver_name.clone(), array_type.clone());
        self.scope
            .insert(callback_name.clone(), callback_type.clone());
        let length_name = format!("__thaw_reduce_length_{}", self.next_binding);
        self.next_binding += 1;
        let index_name = format!("__thaw_reduce_index_{}", self.next_binding);
        self.next_binding += 1;
        let accumulator_name = format!("__thaw_reduce_accumulator_{}", self.next_binding);
        self.next_binding += 1;
        let element_name = format!("__thaw_reduce_element_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(length_name.clone(), HirType::F64);
        self.scope.insert(index_name.clone(), HirType::F64);
        self.scope
            .insert(accumulator_name.clone(), accumulator_type.clone());
        self.scope
            .insert(element_name.clone(), element_type.clone());

        let one = || HirExpr::Lit(HirLit::F64(1.0));
        let length = || HirExpr::Var(length_name.clone());
        let index = || HirExpr::Var(index_name.clone());
        let receiver_var = || HirExpr::Var(receiver_name.clone());
        let (initial_value, initial_index, empty_guard, initial_name) = if initial.is_some() {
            let initial_name = format!("__thaw_reduce_initial_{}", self.next_binding);
            self.next_binding += 1;
            self.scope
                .insert(initial_name.clone(), accumulator_type.clone());
            (
                HirExpr::Var(initial_name.clone()),
                if reverse {
                    HirExpr::BinOp(BinOp::Sub, Box::new(length()), Box::new(one()))
                } else {
                    HirExpr::Lit(HirLit::F64(0.0))
                },
                None,
                Some(initial_name),
            )
        } else {
            (
                HirExpr::TypedIndex(
                    Box::new(receiver_var()),
                    Box::new(if reverse {
                        HirExpr::BinOp(BinOp::Sub, Box::new(length()), Box::new(one()))
                    } else {
                        HirExpr::Lit(HirLit::F64(0.0))
                    }),
                    element_type.clone(),
                ),
                if reverse {
                    HirExpr::BinOp(
                        BinOp::Sub,
                        Box::new(length()),
                        Box::new(HirExpr::Lit(HirLit::F64(2.0))),
                    )
                } else {
                    one()
                },
                Some(HirStmt::If(
                    HirExpr::BinOp(
                        BinOp::EqEqEq,
                        Box::new(length()),
                        Box::new(HirExpr::Lit(HirLit::F64(0.0))),
                    ),
                    vec![HirStmt::Throw(HirExpr::Lit(HirLit::Str(
                        "Reduce of empty array with no initial value".into(),
                    )))],
                    Vec::new(),
                )),
                None,
            )
        };
        let HirType::Function(params, _) = &callback_type else {
            unreachable!("array reducer was validated as a function")
        };
        let available = [
            HirExpr::Var(accumulator_name.clone()),
            HirExpr::Var(element_name.clone()),
            index(),
            receiver_var(),
        ];
        let callback_call = HirExpr::Call(
            Box::new(HirExpr::Var(callback_name.clone())),
            available[..params.len()].to_vec(),
        );
        let mut statements = vec![HirStmt::Let(
            length_name.clone(),
            HirType::F64,
            HirExpr::ArrayLen(Box::new(receiver_var())),
        )];
        if let Some(empty_guard) = empty_guard {
            statements.push(empty_guard);
        }
        statements.extend([
            HirStmt::Let(accumulator_name.clone(), accumulator_type, initial_value),
            HirStmt::Let(index_name.clone(), HirType::F64, initial_index),
            HirStmt::While(
                HirExpr::BinOp(
                    if reverse { BinOp::GtEq } else { BinOp::Lt },
                    Box::new(index()),
                    Box::new(if reverse {
                        HirExpr::Lit(HirLit::F64(0.0))
                    } else {
                        length()
                    }),
                ),
                vec![
                    HirStmt::Let(
                        element_name,
                        element_type.clone(),
                        HirExpr::TypedIndex(
                            Box::new(receiver_var()),
                            Box::new(index()),
                            element_type,
                        ),
                    ),
                    HirStmt::Expr(HirExpr::Assign(
                        accumulator_name.clone(),
                        Box::new(callback_call),
                    )),
                    HirStmt::Expr(HirExpr::Assign(
                        index_name.clone(),
                        Box::new(HirExpr::BinOp(
                            if reverse { BinOp::Sub } else { BinOp::Add },
                            Box::new(index()),
                            Box::new(one()),
                        )),
                    )),
                ],
            ),
            HirStmt::Return(Some(HirExpr::Var(accumulator_name))),
        ]);
        let mut bindings = vec![
            (receiver_name, array_type, receiver),
            (callback_name, callback_type, callback),
        ];
        if let Some((initial, ty)) = initial {
            bindings.push((
                initial_name.expect("initial accumulator binding must be retained"),
                ty,
                initial,
            ));
        }
        self.wrap_call_argument_bindings(HirExpr::Block(statements), &bindings)
    }

    fn lower_array_predicate_method(
        &mut self,
        receiver: HirExpr,
        array_type: HirType,
        element_type: HirType,
        callback: HirExpr,
        this_arg: Option<HirExpr>,
        mode: ArrayPredicateMode,
    ) -> Result<HirExpr, String> {
        let receiver_name = format!("__thaw_predicate_receiver_{}", self.next_binding);
        self.next_binding += 1;
        let callback_name = format!("__thaw_predicate_callback_{}", self.next_binding);
        self.next_binding += 1;
        let callback_type = self.infer_expr_type(&callback)?;
        self.scope.insert(receiver_name.clone(), array_type.clone());
        self.scope
            .insert(callback_name.clone(), callback_type.clone());
        let length_name = format!("__thaw_predicate_length_{}", self.next_binding);
        self.next_binding += 1;
        let index_name = format!("__thaw_predicate_index_{}", self.next_binding);
        self.next_binding += 1;
        let element_name = format!("__thaw_predicate_element_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(length_name.clone(), HirType::F64);
        self.scope.insert(index_name.clone(), HirType::F64);
        self.scope
            .insert(element_name.clone(), element_type.clone());

        let HirType::Function(params, _) = &callback_type else {
            unreachable!("array predicate was validated as a function")
        };
        let available = [
            HirExpr::Var(element_name.clone()),
            HirExpr::Var(index_name.clone()),
            HirExpr::Var(receiver_name.clone()),
        ];
        let callback_call = HirExpr::Call(
            Box::new(HirExpr::Var(callback_name.clone())),
            available[..params.len()].to_vec(),
        );
        let stop_condition = if matches!(
            mode,
            ArrayPredicateMode::Some
                | ArrayPredicateMode::Find
                | ArrayPredicateMode::FindIndex
                | ArrayPredicateMode::FindLast
                | ArrayPredicateMode::FindLastIndex
        ) {
            callback_call
        } else {
            HirExpr::BinOp(
                BinOp::EqEqEq,
                Box::new(callback_call),
                Box::new(HirExpr::Lit(HirLit::Bool(false))),
            )
        };
        let stop_result = match mode {
            ArrayPredicateMode::Some => HirExpr::Lit(HirLit::Bool(true)),
            ArrayPredicateMode::Every => HirExpr::Lit(HirLit::Bool(false)),
            ArrayPredicateMode::Find | ArrayPredicateMode::FindLast => HirExpr::OptionalSome(
                Box::new(HirExpr::Var(element_name.clone())),
                element_type.clone(),
            ),
            ArrayPredicateMode::FindIndex | ArrayPredicateMode::FindLastIndex => {
                HirExpr::Var(index_name.clone())
            }
        };
        let final_result = match mode {
            ArrayPredicateMode::Some => HirExpr::Lit(HirLit::Bool(false)),
            ArrayPredicateMode::Every => HirExpr::Lit(HirLit::Bool(true)),
            ArrayPredicateMode::Find | ArrayPredicateMode::FindLast => {
                HirExpr::OptionalNone(element_type.clone())
            }
            ArrayPredicateMode::FindIndex | ArrayPredicateMode::FindLastIndex => {
                HirExpr::Lit(HirLit::F64(-1.0))
            }
        };
        let one = || HirExpr::Lit(HirLit::F64(1.0));
        let reverse = matches!(
            mode,
            ArrayPredicateMode::FindLast | ArrayPredicateMode::FindLastIndex
        );
        let body = HirExpr::Block(vec![
            HirStmt::Let(
                length_name.clone(),
                HirType::F64,
                HirExpr::ArrayLen(Box::new(HirExpr::Var(receiver_name.clone()))),
            ),
            HirStmt::Let(
                index_name.clone(),
                HirType::F64,
                if reverse {
                    HirExpr::BinOp(
                        BinOp::Sub,
                        Box::new(HirExpr::Var(length_name.clone())),
                        Box::new(one()),
                    )
                } else {
                    HirExpr::Lit(HirLit::F64(0.0))
                },
            ),
            HirStmt::While(
                HirExpr::BinOp(
                    if reverse { BinOp::GtEq } else { BinOp::Lt },
                    Box::new(HirExpr::Var(index_name.clone())),
                    Box::new(if reverse {
                        HirExpr::Lit(HirLit::F64(0.0))
                    } else {
                        HirExpr::Var(length_name)
                    }),
                ),
                vec![
                    HirStmt::Let(
                        element_name,
                        element_type.clone(),
                        HirExpr::TypedIndex(
                            Box::new(HirExpr::Var(receiver_name.clone())),
                            Box::new(HirExpr::Var(index_name.clone())),
                            element_type.clone(),
                        ),
                    ),
                    HirStmt::If(
                        stop_condition,
                        vec![HirStmt::Return(Some(stop_result))],
                        Vec::new(),
                    ),
                    HirStmt::Expr(HirExpr::Assign(
                        index_name.clone(),
                        Box::new(HirExpr::BinOp(
                            if reverse { BinOp::Sub } else { BinOp::Add },
                            Box::new(HirExpr::Var(index_name)),
                            Box::new(one()),
                        )),
                    )),
                ],
            ),
            HirStmt::Return(Some(final_result)),
        ]);
        let mut bindings = vec![
            (receiver_name, array_type, receiver),
            (callback_name, callback_type, callback),
        ];
        if let Some(this_arg) = this_arg {
            let ty = self.infer_expr_type(&this_arg)?;
            let name = format!("__thaw_predicate_this_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(name.clone(), ty.clone());
            bindings.push((name, ty, this_arg));
        }
        self.wrap_call_argument_bindings(body, &bindings)
    }

    fn lower_array_for_each(
        &mut self,
        receiver: HirExpr,
        array_type: HirType,
        element_type: HirType,
        callback: HirExpr,
        this_arg: Option<HirExpr>,
    ) -> Result<HirExpr, String> {
        let receiver_name = format!("__thaw_for_each_receiver_{}", self.next_binding);
        self.next_binding += 1;
        let callback_name = format!("__thaw_for_each_callback_{}", self.next_binding);
        self.next_binding += 1;
        let callback_type = self.infer_expr_type(&callback)?;
        self.scope.insert(receiver_name.clone(), array_type.clone());
        self.scope
            .insert(callback_name.clone(), callback_type.clone());
        let length_name = format!("__thaw_for_each_length_{}", self.next_binding);
        self.next_binding += 1;
        let index_name = format!("__thaw_for_each_index_{}", self.next_binding);
        self.next_binding += 1;
        let element_name = format!("__thaw_for_each_element_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(length_name.clone(), HirType::F64);
        self.scope.insert(index_name.clone(), HirType::F64);
        self.scope
            .insert(element_name.clone(), element_type.clone());
        let HirType::Function(params, _) = &callback_type else {
            unreachable!("array callback was validated as a function")
        };
        let available = [
            HirExpr::Var(element_name.clone()),
            HirExpr::Var(index_name.clone()),
            HirExpr::Var(receiver_name.clone()),
        ];
        let callback_call = HirExpr::Call(
            Box::new(HirExpr::Var(callback_name.clone())),
            available[..params.len()].to_vec(),
        );
        let body = HirExpr::Block(vec![
            HirStmt::Let(
                length_name.clone(),
                HirType::F64,
                HirExpr::ArrayLen(Box::new(HirExpr::Var(receiver_name.clone()))),
            ),
            HirStmt::Let(
                index_name.clone(),
                HirType::F64,
                HirExpr::Lit(HirLit::F64(0.0)),
            ),
            HirStmt::While(
                HirExpr::BinOp(
                    BinOp::Lt,
                    Box::new(HirExpr::Var(index_name.clone())),
                    Box::new(HirExpr::Var(length_name)),
                ),
                vec![
                    HirStmt::Let(
                        element_name,
                        element_type.clone(),
                        HirExpr::TypedIndex(
                            Box::new(HirExpr::Var(receiver_name.clone())),
                            Box::new(HirExpr::Var(index_name.clone())),
                            element_type,
                        ),
                    ),
                    HirStmt::Expr(callback_call),
                    HirStmt::Expr(HirExpr::Assign(
                        index_name.clone(),
                        Box::new(HirExpr::BinOp(
                            BinOp::Add,
                            Box::new(HirExpr::Var(index_name)),
                            Box::new(HirExpr::Lit(HirLit::F64(1.0))),
                        )),
                    )),
                ],
            ),
            HirStmt::Return(None),
        ]);
        let mut bindings = vec![
            (receiver_name, array_type, receiver),
            (callback_name, callback_type, callback),
        ];
        if let Some(this_arg) = this_arg {
            let ty = self.infer_expr_type(&this_arg)?;
            let name = format!("__thaw_for_each_this_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(name.clone(), ty.clone());
            bindings.push((name, ty, this_arg));
        }
        self.wrap_call_argument_bindings(body, &bindings)
    }

    fn lower_native_spread_values(
        &mut self,
        arguments: &[swc_ecma_ast::ExprOrSpread],
        label: &str,
    ) -> Result<(Vec<HirExpr>, Vec<LoweredBinding>), String> {
        let lowered = arguments
            .iter()
            .map(|argument| self.lower_expr(&argument.expr))
            .collect::<Result<Vec<_>, _>>()?;
        let preserve_order = arguments.iter().any(|argument| argument.spread.is_some())
            || lowered.iter().any(contains_await);
        let mut bindings = Vec::new();
        let mut values = Vec::new();
        for (argument, value) in arguments.iter().zip(lowered) {
            if !preserve_order {
                values.push(value);
                continue;
            }
            if argument.spread.is_none() {
                let ty = self.infer_expr_type(&value)?;
                let name = format!("__thaw_native_arg_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(name.clone(), ty.clone());
                bindings.push((name.clone(), ty, value));
                values.push(HirExpr::Var(name));
                continue;
            }
            if let HirExpr::ArrayLit(elements) = value {
                for element in elements {
                    let ty = self.infer_expr_type(&element)?;
                    let name = format!("__thaw_native_arg_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(name.clone(), ty.clone());
                    bindings.push((name.clone(), ty, element));
                    values.push(HirExpr::Var(name));
                }
                continue;
            }
            let source_type = self.infer_expr_type(&value)?;
            let HirType::Tuple(elements) = &source_type else {
                return Err(format!(
                    "{label} spread source must have statically known tuple length, got {source_type:?}"
                ));
            };
            let elements = elements.clone();
            let name = format!("__thaw_native_spread_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(name.clone(), source_type.clone());
            bindings.push((name.clone(), source_type, value));
            values.extend(elements.into_iter().enumerate().map(|(index, ty)| {
                HirExpr::TypedIndex(
                    Box::new(HirExpr::Var(name.clone())),
                    Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                    ty,
                )
            }));
        }
        Ok((values, bindings))
    }

    fn select_omitted_class_arguments(
        &mut self,
        symbol: &mut Symbol,
        signature: &mut FnSignature,
        mut arguments: Vec<HirExpr>,
        receiver_count: usize,
        label: &str,
    ) -> Result<Vec<HirExpr>, String> {
        let native_rest_values = signature.native_rest.clone().map(|element| {
            let fixed_argument_count = signature.params.len() - receiver_count - 1;
            let values = if arguments.len() > fixed_argument_count {
                arguments.split_off(fixed_argument_count)
            } else {
                Vec::new()
            };
            (element, values)
        });
        let logical_param_count =
            signature.params.len() - usize::from(signature.native_rest.is_some());
        if logical_param_count < usize::BITS as usize
            && arguments.len() + receiver_count <= logical_param_count
        {
            let mut omitted_mask = 0usize;
            for index in receiver_count..logical_param_count {
                let omitted = match arguments.get(index - receiver_count) {
                    None => true,
                    Some(value) => self.infer_expr_type(value)? == HirType::Undefined,
                };
                if omitted {
                    omitted_mask |= 1usize << index;
                }
            }
            let wrapper = omitted_parameter_symbol(symbol, omitted_mask);
            if omitted_mask != 0 {
                if let Some(wrapper_signature) = self.signatures.get(&wrapper).cloned() {
                    arguments = arguments
                        .into_iter()
                        .enumerate()
                        .filter_map(|(index, argument)| {
                            (omitted_mask & (1usize << (index + receiver_count)) == 0)
                                .then_some(argument)
                        })
                        .collect();
                    *symbol = wrapper;
                    *signature = wrapper_signature;
                }
            }
        }
        if signature.native_rest.is_none()
            && arguments.len() + receiver_count != signature.params.len()
        {
            let wrapper = default_arity_symbol(symbol, arguments.len() + receiver_count);
            if let Some(wrapper_signature) = self.signatures.get(&wrapper).cloned() {
                *symbol = wrapper;
                *signature = wrapper_signature;
            }
        }
        if let Some((element, values)) = native_rest_values {
            let values = values
                .into_iter()
                .map(|value| self.coerce_to_declared(&element, value))
                .collect::<Result<Vec<_>, _>>()?;
            arguments.push(native_rest_array(values, &element));
        }
        let expected = &signature.params[receiver_count..];
        if arguments.len() != expected.len() {
            return Err(format!(
                "{label} expects {} argument(s), got {}",
                expected.len(),
                arguments.len()
            ));
        }
        arguments
            .into_iter()
            .zip(expected)
            .map(|(value, expected)| self.coerce_to_declared(expected, value))
            .collect()
    }

    fn lower_call(&mut self, call: &CallExpr) -> Result<HirExpr, String> {
        if matches!(call.callee, Callee::Super(_)) {
            let (mut symbol, _base_type, _base_name) = self
                .super_initializer
                .clone()
                .ok_or("`super(...)` is only valid in a derived class constructor")?;
            let mut signature = self
                .signatures
                .get(&symbol)
                .cloned()
                .ok_or_else(|| format!("missing base class initializer `{symbol}`"))?;
            if call.type_args.is_some() {
                return Err("native `super(...)` does not support type arguments".into());
            }
            let this_name = self.resolve_binding("this");
            let mut args = vec![HirExpr::Var(this_name)];
            let (arguments, bindings) =
                self.lower_native_spread_values(&call.args, "base constructor")?;
            let arguments = self.select_omitted_class_arguments(
                &mut symbol,
                &mut signature,
                arguments,
                1,
                "base constructor",
            )?;
            args.extend(arguments);
            let result = HirExpr::Call(Box::new(HirExpr::Var(symbol)), args);
            return self.wrap_call_argument_bindings(result, &bindings);
        }
        if let Callee::Expr(callee) = &call.callee {
            if let Expr::SuperProp(member) = callee.as_ref() {
                let (_, _, base_name) = self
                    .super_initializer
                    .clone()
                    .ok_or("`super` member access is only valid in a derived class")?;
                let method_name = match &member.prop {
                    SuperProp::Ident(name) => name.sym.to_string(),
                    SuperProp::Computed(computed) => match computed.expr.as_ref() {
                        Expr::Lit(Lit::Str(name)) => name.value.to_string_lossy().into_owned(),
                        _ => {
                            return Err(
                                "computed super methods require a string literal name".into()
                            )
                        }
                    },
                };
                let mut symbol = if self.class_static_context {
                    class_static_method_symbol(&base_name, &method_name)
                } else {
                    class_method_symbol(&base_name, &method_name)
                };
                let mut signature = self.signatures.get(&symbol).cloned().ok_or_else(|| {
                    format!("base class `{base_name}` has no method `{method_name}`")
                })?;
                if call.type_args.is_some() {
                    return Err("native super methods do not support type arguments".into());
                }
                let receiver_count = usize::from(!self.class_static_context);
                let mut args = if self.class_static_context {
                    Vec::new()
                } else {
                    vec![HirExpr::Var(self.resolve_binding("this"))]
                };
                let label = format!("super method `{base_name}.{method_name}`");
                let (arguments, bindings) = self.lower_native_spread_values(&call.args, &label)?;
                let arguments = self.select_omitted_class_arguments(
                    &mut symbol,
                    &mut signature,
                    arguments,
                    receiver_count,
                    &label,
                )?;
                args.extend(arguments);
                let result = HirExpr::Call(Box::new(HirExpr::Var(symbol)), args);
                return self.wrap_call_argument_bindings(result, &bindings);
            }
        }
        let Callee::Expr(callee_expr) = &call.callee else {
            return Err("unsupported callee (super/import calls not supported)".into());
        };

        if let Some(invoked) = self.lower_saved_native_method_call_or_apply(call)? {
            return Ok(invoked);
        }
        if self.unbound_this_context {
            if let Expr::Member(member) = callee_expr.as_ref() {
                if matches!(member.obj.as_ref(), Expr::This(_)) {
                    let property = member_property_name(&member.prop)
                        .ok_or("unbound `this` method call requires a statically known property")?;
                    let HirType::Function(_, result) = self
                        .unbound_this_member_type(&property)
                        .ok_or_else(|| format!("class has no native method `{property}`"))?
                    else {
                        return Err(format!("native member `{property}` is not callable"));
                    };
                    return self.lower_unbound_this_error(&property, &result);
                }
            }
        }

        if let Some(invoked) = self.lower_immediately_invoked_class_bind(call)? {
            return Ok(invoked);
        }
        if let Some(invoked) = self.lower_immediately_invoked_function_bind(call)? {
            return Ok(invoked);
        }

        if let Some(bound) = self.lower_native_class_bind(call)? {
            return Ok(bound);
        }
        if let Some(bound) = self.lower_native_static_bind(call)? {
            return Ok(bound);
        }
        if let Some(invoked) = self.lower_native_class_call_or_apply(call)? {
            return Ok(invoked);
        }
        if let Some(invoked) = self.lower_native_static_call_or_apply(call)? {
            return Ok(invoked);
        }
        if let Some(bound) = self.lower_function_bind(call)? {
            return Ok(bound);
        }
        if let Some(invoked) = self.lower_function_call_or_apply(call)? {
            return Ok(invoked);
        }

        if let Expr::Member(member) = callee_expr.as_ref() {
            if let Some(property) = member_property_name(&member.prop) {
                if matches!(member.obj.as_ref(), Expr::This(_)) && self.class_static_context {
                    let class_name = self
                        .class_context
                        .as_deref()
                        .expect("static class lowering retains its class context");
                    let symbol = class_static_method_symbol(class_name, &property);
                    if self.signatures.contains_key(&symbol) {
                        if call.type_args.is_some() {
                            return Err(format!(
                                "native static method `{class_name}.{property}` is not generic"
                            ));
                        }
                        return self.lower_call(&CallExpr {
                            span: call.span,
                            ctxt: call.ctxt,
                            callee: Callee::Expr(Box::new(Expr::Ident(
                                swc_ecma_ast::Ident::new_no_ctxt(symbol.into(), call.span),
                            ))),
                            args: call.args.clone(),
                            type_args: None,
                        });
                    }
                }
                if let Expr::Ident(class) = member.obj.as_ref() {
                    let class_name = class.sym.as_ref();
                    let symbol = class_static_method_symbol(class_name, &property);
                    if self.signatures.contains_key(&symbol) {
                        if call.type_args.is_some() {
                            return Err(format!(
                                "native static method `{class_name}.{property}` is not generic"
                            ));
                        }
                        return self.lower_call(&CallExpr {
                            span: call.span,
                            ctxt: call.ctxt,
                            callee: Callee::Expr(Box::new(Expr::Ident(
                                swc_ecma_ast::Ident::new_no_ctxt(symbol.into(), call.span),
                            ))),
                            args: call.args.clone(),
                            type_args: None,
                        });
                    }
                }
                let receiver_type = match member.obj.as_ref() {
                    Expr::Ident(receiver) => {
                        let name = self.resolve_binding(receiver.sym.as_ref());
                        self.scope.get(&name).cloned()
                    }
                    Expr::New(construction) => construction
                        .callee
                        .as_ident()
                        .and_then(|class| self.interfaces.get(class.sym.as_ref()).cloned()),
                    Expr::This(_) => self.scope.get(&self.resolve_binding("this")).cloned(),
                    Expr::Member(_) | Expr::Paren(_) | Expr::TsAs(_) | Expr::TsTypeAssertion(_) => {
                        infer_generic_constructor_expr_type(
                            &member.obj,
                            self.interfaces,
                            self.generic_interfaces,
                            std::slice::from_ref(&self.scope),
                            &self.generic_call_returns,
                        )
                        .ok()
                    }
                    _ => None,
                };
                if let Some(receiver_type) =
                    receiver_type.filter(|ty| class_name_from_type(ty).is_some())
                {
                    let class_name = class_name_from_type(&receiver_type)
                        .expect("the receiver was classified as a native class");
                    let symbol = class_method_symbol(class_name, &property);
                    if self.signatures.contains_key(&symbol) {
                        if call.type_args.is_some() {
                            return Err(format!(
                                "native class method `{class_name}.{property}` is not generic"
                            ));
                        }
                        let mut args = Vec::with_capacity(call.args.len() + 1);
                        args.push(swc_ecma_ast::ExprOrSpread {
                            spread: None,
                            expr: member.obj.clone(),
                        });
                        args.extend(call.args.iter().cloned());
                        return self.lower_call(&CallExpr {
                            span: call.span,
                            ctxt: call.ctxt,
                            callee: Callee::Expr(Box::new(Expr::Ident(
                                swc_ecma_ast::Ident::new_no_ctxt(symbol.into(), call.span),
                            ))),
                            args,
                            type_args: None,
                        });
                    }
                    return Err(format!(
                        "class `{class_name}` has no native method `{property}`"
                    ));
                }
            }
        }

        if matches!(callee_expr.as_ref(), Expr::Ident(identifier) if identifier.sym == *"__thaw_object_rest")
        {
            if call.args.is_empty() || call.args.iter().any(|argument| argument.spread.is_some()) {
                return Err("object-rest lowering expects a source and static field names".into());
            }
            let source = self.lower_expr(&call.args[0].expr)?;
            let source_type = self.infer_expr_type(&source)?;
            let HirType::Object(fields) = &source_type else {
                return Err(format!(
                    "object rest requires a fixed-shape object, got {source_type:?}"
                ));
            };
            let omitted = call.args[1..]
                .iter()
                .map(|argument| match argument.expr.as_ref() {
                    Expr::Lit(Lit::Str(key)) => Ok(key.value.to_string_lossy().into_owned()),
                    _ => Err("object-rest field names must be string literals".to_string()),
                })
                .collect::<Result<HashSet<_>, _>>()?;
            return Ok(HirExpr::ObjectLit(
                fields
                    .iter()
                    .filter(|(name, _)| !omitted.contains(name))
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
                    .collect(),
            ));
        }

        if let Expr::Member(member) = callee_expr.as_ref() {
            if let MemberProp::Ident(property) = &member.prop {
                if let Expr::Ident(object) = member.obj.as_ref() {
                    if object.sym == *"Array" && property.sym == *"of" {
                        let explicit_type = if let Some(type_args) = &call.type_args {
                            let [element] = type_args.params.as_slice() else {
                                return Err("`Array.of` expects zero or one type argument".into());
                            };
                            Some(lower_ts_type(
                                element,
                                self.interfaces,
                                self.generic_interfaces,
                            )?)
                        } else {
                            None
                        };
                        let mut element_type = explicit_type;
                        let mut parts = Vec::new();
                        let mut pending = Vec::new();
                        for argument in &call.args {
                            let value = self.lower_expr(&argument.expr)?;
                            if argument.spread.is_some() {
                                if !pending.is_empty() {
                                    parts.push(HirExpr::ArrayLit(std::mem::take(&mut pending)));
                                }
                                let ty = self.infer_expr_type(&value)?;
                                let HirType::Array(element) = ty else {
                                    return Err(
                                        "`Array.of` spread requires a homogeneous array".into()
                                    );
                                };
                                if let Some(expected) = &element_type {
                                    if expected != element.as_ref() {
                                        return Err(format!(
                                            "`Array.of` spread element has type {element:?}, expected {expected:?}"
                                        ));
                                    }
                                } else {
                                    element_type = Some(element.as_ref().clone());
                                }
                                parts.push(value);
                            } else {
                                let ty = self.infer_expr_type(&value)?;
                                if let Some(expected) = &element_type {
                                    if expected != &ty {
                                        return Err(format!(
                                            "`Array.of` element has type {ty:?}, expected {expected:?}"
                                        ));
                                    }
                                } else {
                                    element_type = Some(ty);
                                }
                                pending.push(value);
                            }
                        }
                        if !pending.is_empty() {
                            parts.push(HirExpr::ArrayLit(pending));
                        }
                        let element_type = element_type.ok_or(
                            "empty `Array.of()` requires an explicit element type argument",
                        )?;
                        return Ok(match parts.len() {
                            0 => HirExpr::ArrayAlloc(
                                Box::new(HirExpr::Lit(HirLit::F64(0.0))),
                                element_type,
                            ),
                            1 => parts.pop().unwrap(),
                            _ => HirExpr::ArrayConcat(parts, element_type),
                        });
                    }
                    if object.sym == *"Array" && property.sym == *"from" {
                        if !(1..=3).contains(&call.args.len()) {
                            return Err(
                                "native `Array.from` expects a source, optional mapper and optional thisArg"
                                    .into(),
                            );
                        }
                        if call.args.iter().any(|argument| argument.spread.is_some()) {
                            return Err("Array.from spread arguments are not supported".into());
                        }
                        let explicit_types = call
                            .type_args
                            .as_ref()
                            .map(|type_args| {
                                type_args
                                    .params
                                    .iter()
                                    .map(|ty| {
                                        lower_ts_type(ty, self.interfaces, self.generic_interfaces)
                                    })
                                    .collect::<Result<Vec<_>, _>>()
                            })
                            .transpose()?
                            .unwrap_or_default();
                        if explicit_types.len() > 2 {
                            return Err("`Array.from` expects at most two type arguments".into());
                        }
                        let source = self.lower_expr(&call.args[0].expr)?;
                        let source_type = self.infer_expr_type(&source)?;
                        let (source, source_type, element_type) = match source_type {
                            HirType::Array(element) => {
                                let element_type = element.as_ref().clone();
                                (source, HirType::Array(element), element_type)
                            }
                            HirType::Str => (
                                HirExpr::Call(
                                    Box::new(HirExpr::Var("__thaw_string_to_array".into())),
                                    vec![source],
                                ),
                                HirType::Array(Box::new(HirType::Str)),
                                HirType::Str,
                            ),
                            other => {
                                return Err(format!(
                                    "native `Array.from` requires a homogeneous array or string, got {other:?}"
                                ))
                            }
                        };
                        if let Some(expected) = explicit_types.first() {
                            if expected != &element_type {
                                return Err(format!(
                                    "`Array.from` source element has type {element_type:?}, expected {expected:?}"
                                ));
                            }
                        }
                        let Some(mapper_argument) = call.args.get(1) else {
                            return Ok(HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_array_slice".into())),
                                vec![
                                    source,
                                    HirExpr::Lit(HirLit::F64(0.0)),
                                    HirExpr::Lit(HirLit::F64(f64::INFINITY)),
                                ],
                            ));
                        };
                        let callback =
                            self.lower_array_from_callback(&mapper_argument.expr, &element_type)?;
                        if let Some(expected) = explicit_types.get(1).or(explicit_types.first()) {
                            let HirType::Function(_, output) = self.infer_expr_type(&callback)?
                            else {
                                unreachable!("Array.from mapper is a function")
                            };
                            if output.as_ref() != expected {
                                return Err(format!(
                                    "`Array.from` mapper returns {output:?}, expected {expected:?}"
                                ));
                            }
                        }
                        let this_arg = call
                            .args
                            .get(2)
                            .map(|argument| self.lower_expr(&argument.expr))
                            .transpose()?;
                        return self.lower_array_map(
                            source,
                            source_type,
                            element_type,
                            callback,
                            this_arg,
                        );
                    }
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
                    if (object.sym == *"Object"
                        && matches!(property.sym.as_ref(), "keys" | "getOwnPropertyNames"))
                        || (object.sym == *"Reflect" && property.sym == *"ownKeys")
                    {
                        let label = format!("{}.{}", object.sym, property.sym);
                        let [argument] = call.args.as_slice() else {
                            return Err(format!("`{label}` expects exactly one argument"));
                        };
                        if argument.spread.is_some() {
                            return Err(format!("{label} spread is not supported"));
                        }
                        let value = self.lower_expr(&argument.expr)?;
                        let ty = self.infer_expr_type(&value)?;
                        let HirType::Object(fields) = &ty else {
                            return Err(format!(
                                "`{label}` currently requires a fixed object, got {ty:?}"
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
                    if object.sym == *"Object" && property.sym == *"values" {
                        let [argument] = call.args.as_slice() else {
                            return Err("`Object.values` expects exactly one argument".into());
                        };
                        if argument.spread.is_some() {
                            return Err("Object.values spread is not supported".into());
                        }
                        let value = self.lower_expr(&argument.expr)?;
                        let ty = self.infer_expr_type(&value)?;
                        let HirType::Object(fields) = &ty else {
                            return Err(format!(
                                "`Object.values` currently requires a fixed object, got {ty:?}"
                            ));
                        };
                        let field_names = fields
                            .iter()
                            .map(|(name, _)| name.clone())
                            .collect::<Vec<_>>();
                        let name = format!("__thaw_object_values_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(name.clone(), ty.clone());
                        let values = HirExpr::ArrayLit(
                            field_names
                                .into_iter()
                                .map(|field| {
                                    HirExpr::PropAccess(
                                        Box::new(HirExpr::Var(name.clone())),
                                        ty.clone(),
                                        field,
                                    )
                                })
                                .collect(),
                        );
                        return self.wrap_call_argument_bindings(values, &[(name, ty, value)]);
                    }
                    if object.sym == *"Object" && property.sym == *"entries" {
                        let [argument] = call.args.as_slice() else {
                            return Err("`Object.entries` expects exactly one argument".into());
                        };
                        if argument.spread.is_some() {
                            return Err("Object.entries spread is not supported".into());
                        }
                        let value = self.lower_expr(&argument.expr)?;
                        let ty = self.infer_expr_type(&value)?;
                        let HirType::Object(fields) = &ty else {
                            return Err(format!(
                                "`Object.entries` currently requires a fixed object, got {ty:?}"
                            ));
                        };
                        let entry_fields = fields
                            .iter()
                            .map(|(name, field_type)| (name.clone(), field_type.clone()))
                            .collect::<Vec<_>>();
                        let name = format!("__thaw_object_entries_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(name.clone(), ty.clone());
                        let entries = HirExpr::ArrayLit(
                            entry_fields
                                .into_iter()
                                .map(|(field, field_type)| {
                                    let entry_type = HirType::Tuple(vec![HirType::Str, field_type]);
                                    HirExpr::Call(
                                        Box::new(HirExpr::Lambda(
                                            vec![HirParam {
                                                name: name.clone(),
                                                ty: ty.clone(),
                                            }],
                                            Vec::new(),
                                            entry_type,
                                            Box::new(HirExpr::ArrayLit(vec![
                                                HirExpr::Lit(HirLit::Str(field.clone())),
                                                HirExpr::PropAccess(
                                                    Box::new(HirExpr::Var(name.clone())),
                                                    ty.clone(),
                                                    field,
                                                ),
                                            ])),
                                        )),
                                        Vec::new(),
                                    )
                                })
                                .collect(),
                        );
                        return self.wrap_call_argument_bindings(entries, &[(name, ty, value)]);
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
                    if object.sym == *"Object" && property.sym == *"is" {
                        let [left, right] = call.args.as_slice() else {
                            return Err("`Object.is` expects exactly two arguments".into());
                        };
                        if left.spread.is_some() || right.spread.is_some() {
                            return Err("Object.is spread is not supported".into());
                        }
                        let left_value = self.lower_expr(&left.expr)?;
                        let right_value = self.lower_expr(&right.expr)?;
                        let left_type = self.infer_expr_type(&left_value)?;
                        let right_type = self.infer_expr_type(&right_value)?;
                        if matches!(left_type, HirType::Json | HirType::Dynamic)
                            || matches!(right_type, HirType::Json | HirType::Dynamic)
                        {
                            return Err("`Object.is` requires statically native operands".into());
                        }
                        let left_name = format!("__thaw_object_is_left_{}", self.next_binding);
                        self.next_binding += 1;
                        let right_name = format!("__thaw_object_is_right_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(left_name.clone(), left_type.clone());
                        self.scope.insert(right_name.clone(), right_type.clone());
                        let result = if left_type != right_type {
                            HirExpr::Lit(HirLit::Bool(false))
                        } else if left_type == HirType::F64 {
                            HirExpr::Call(
                                Box::new(HirExpr::Var("__thaw_number_object_is".into())),
                                vec![
                                    HirExpr::Var(left_name.clone()),
                                    HirExpr::Var(right_name.clone()),
                                ],
                            )
                        } else {
                            HirExpr::BinOp(
                                BinOp::EqEqEq,
                                Box::new(HirExpr::Var(left_name.clone())),
                                Box::new(HirExpr::Var(right_name.clone())),
                            )
                        };
                        return self.wrap_call_argument_bindings(
                            result,
                            &[
                                (left_name, left_type, left_value),
                                (right_name, right_type, right_value),
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
                if property.sym == *"repeat" {
                    let [count] = call.args.as_slice() else {
                        return Err("native `.repeat()` expects exactly one count".into());
                    };
                    if count.spread.is_some() {
                        return Err("string repeat spread is not supported".into());
                    }
                    let receiver = self.lower_expr(&member.obj)?;
                    self.expect_type(&HirType::Str, &receiver, "string repeat receiver")?;
                    let count = self.lower_expr(&count.expr)?;
                    let count = self.coerce_primitive_to_number(count)?;
                    let receiver_name = format!("__thaw_repeat_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    let count_name = format!("__thaw_repeat_count_{}", self.next_binding);
                    self.next_binding += 1;
                    let normalized_name = format!("__thaw_repeat_normalized_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(receiver_name.clone(), HirType::Str);
                    self.scope.insert(count_name.clone(), HirType::F64);
                    self.scope.insert(normalized_name.clone(), HirType::F64);
                    let number = |value| HirExpr::Lit(HirLit::F64(value));
                    let var = |name: &str| HirExpr::Var(name.into());
                    let assign = |value| {
                        HirStmt::Expr(HirExpr::Assign(normalized_name.clone(), Box::new(value)))
                    };
                    let range_error = || {
                        HirStmt::Throw(HirExpr::Lit(HirLit::Str(
                            "Invalid count value for String.prototype.repeat".into(),
                        )))
                    };
                    let body = HirExpr::Block(vec![
                        HirStmt::Let(normalized_name.clone(), HirType::F64, var(&count_name)),
                        HirStmt::If(
                            HirExpr::BinOp(
                                BinOp::EqEqEq,
                                Box::new(var(&normalized_name)),
                                Box::new(var(&normalized_name)),
                            ),
                            Vec::new(),
                            vec![assign(number(0.0))],
                        ),
                        assign(HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_math_trunc".into())),
                            vec![var(&normalized_name)],
                        )),
                        HirStmt::If(
                            HirExpr::BinOp(
                                BinOp::Lt,
                                Box::new(var(&normalized_name)),
                                Box::new(number(0.0)),
                            ),
                            vec![range_error()],
                            Vec::new(),
                        ),
                        HirStmt::If(
                            HirExpr::BinOp(
                                BinOp::EqEqEq,
                                Box::new(var(&normalized_name)),
                                Box::new(number(f64::INFINITY)),
                            ),
                            vec![range_error()],
                            Vec::new(),
                        ),
                        HirStmt::Return(Some(HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_string_repeat".into())),
                            vec![var(&receiver_name), var(&normalized_name)],
                        ))),
                    ]);
                    return self.wrap_call_argument_bindings(
                        body,
                        &[
                            (receiver_name, HirType::Str, receiver),
                            (count_name, HirType::F64, count),
                        ],
                    );
                }
                if matches!(property.sym.as_ref(), "toLowerCase" | "toUpperCase") {
                    if !call.args.is_empty() {
                        return Err(format!("native `.{}()` expects no arguments", property.sym));
                    }
                    let receiver = self.lower_expr(&member.obj)?;
                    self.expect_type(&HirType::Str, &receiver, "string case receiver")?;
                    let suffix = if property.sym == *"toLowerCase" {
                        "to_lower_case"
                    } else {
                        "to_upper_case"
                    };
                    return Ok(HirExpr::Call(
                        Box::new(HirExpr::Var(format!("__thaw_string_{suffix}"))),
                        vec![receiver],
                    ));
                }
                if matches!(property.sym.as_ref(), "isWellFormed" | "toWellFormed") {
                    if !call.args.is_empty() {
                        return Err(format!("native `.{}()` expects no arguments", property.sym));
                    }
                    let receiver = self.lower_expr(&member.obj)?;
                    self.expect_type(
                        &HirType::Str,
                        &receiver,
                        &format!("string {} receiver", property.sym),
                    )?;
                    if property.sym == *"toWellFormed" {
                        return Ok(receiver);
                    }
                    let receiver_name =
                        format!("__thaw_well_formed_receiver_{}", self.next_binding);
                    self.next_binding += 1;
                    self.scope.insert(receiver_name.clone(), HirType::Str);
                    return self.wrap_call_argument_bindings(
                        HirExpr::Lit(HirLit::Bool(true)),
                        &[(receiver_name, HirType::Str, receiver)],
                    );
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
                if matches!(property.sym.as_ref(), "sort" | "toSorted") {
                    if call.args.len() > 1 {
                        return Err(format!(
                            "native `.{}()` expects zero or one comparator",
                            property.sym
                        ));
                    }
                    if call.args.iter().any(|argument| argument.spread.is_some()) {
                        return Err("array sort comparator spread is not supported".into());
                    }
                    let receiver = self.lower_expr(&member.obj)?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    let HirType::Array(element) = &receiver_type else {
                        return Err(format!(
                            "`.{}()` requires a homogeneous array, got {receiver_type:?}",
                            property.sym
                        ));
                    };
                    let element_type = element.as_ref().clone();
                    if let Some(argument) = call.args.first() {
                        let comparator = self.lower_promise_callback(
                            &argument.expr,
                            &[element_type.clone(), element_type.clone()],
                            Some(&HirType::F64),
                        )?;
                        return self.lower_array_sort_comparator(
                            receiver,
                            receiver_type,
                            element_type,
                            comparator,
                            property.sym == *"toSorted",
                        );
                    }
                    let prefix = match &element_type {
                        HirType::F64 => "number",
                        HirType::Str => "string",
                        HirType::Bool => "bool",
                        HirType::Object(_) => "object",
                        other => {
                            return Err(format!(
                                "default array sort does not support element type {other:?}"
                            ))
                        }
                    };
                    let suffix = if property.sym == *"sort" {
                        "sort"
                    } else {
                        "to_sorted"
                    };
                    return Ok(HirExpr::Call(
                        Box::new(HirExpr::Var(format!("__thaw_{prefix}_array_{suffix}"))),
                        vec![receiver],
                    ));
                }
                if matches!(
                    property.sym.as_ref(),
                    "some" | "every" | "find" | "findIndex" | "findLast" | "findLastIndex"
                ) {
                    if !(1..=2).contains(&call.args.len()) {
                        return Err(format!(
                            "native `.{}()` expects a predicate and optional thisArg",
                            property.sym
                        ));
                    }
                    if call.args.iter().any(|argument| argument.spread.is_some()) {
                        return Err("array predicate spread is not supported".into());
                    }
                    let receiver = self.lower_expr(&member.obj)?;
                    let array_type = self.infer_expr_type(&receiver)?;
                    let HirType::Array(element) = &array_type else {
                        return Err(format!(
                            "`.{}()` requires a homogeneous array, got {array_type:?}",
                            property.sym
                        ));
                    };
                    let element_type = element.as_ref().clone();
                    let callback = self.lower_array_callback(
                        &call.args[0].expr,
                        &element_type,
                        &array_type,
                        &HirType::Bool,
                    )?;
                    let this_arg = call
                        .args
                        .get(1)
                        .map(|argument| self.lower_expr(&argument.expr))
                        .transpose()?;
                    return self.lower_array_predicate_method(
                        receiver,
                        array_type,
                        element_type,
                        callback,
                        this_arg,
                        match property.sym.as_ref() {
                            "some" => ArrayPredicateMode::Some,
                            "every" => ArrayPredicateMode::Every,
                            "find" => ArrayPredicateMode::Find,
                            "findIndex" => ArrayPredicateMode::FindIndex,
                            "findLast" => ArrayPredicateMode::FindLast,
                            "findLastIndex" => ArrayPredicateMode::FindLastIndex,
                            _ => unreachable!(),
                        },
                    );
                }
                if matches!(property.sym.as_ref(), "reduce" | "reduceRight") {
                    if !(1..=2).contains(&call.args.len()) {
                        return Err(format!(
                            "native `.{}()` expects a reducer and optional initial value",
                            property.sym
                        ));
                    }
                    if call.args.iter().any(|argument| argument.spread.is_some()) {
                        return Err("array reducer spread is not supported".into());
                    }
                    let receiver = self.lower_expr(&member.obj)?;
                    let array_type = self.infer_expr_type(&receiver)?;
                    let HirType::Array(element) = &array_type else {
                        return Err(format!(
                            "`.{}()` requires a homogeneous array, got {array_type:?}",
                            property.sym
                        ));
                    };
                    let element_type = element.as_ref().clone();
                    let initial = call
                        .args
                        .get(1)
                        .map(|argument| {
                            let value = self.lower_expr(&argument.expr)?;
                            let ty = self.infer_expr_type(&value)?;
                            Ok::<_, String>((value, ty))
                        })
                        .transpose()?;
                    let accumulator_type =
                        initial.as_ref().map(|(_, ty)| ty).unwrap_or(&element_type);
                    let callback = self.lower_array_reducer_callback(
                        &call.args[0].expr,
                        accumulator_type,
                        &element_type,
                        &array_type,
                    )?;
                    return self.lower_array_reduce(
                        receiver,
                        array_type,
                        element_type,
                        callback,
                        initial,
                        property.sym == *"reduceRight",
                    );
                }
                if property.sym == *"toSpliced" {
                    if call.args.iter().any(|argument| argument.spread.is_some()) {
                        return Err("array toSpliced spread arguments are not supported".into());
                    }
                    let receiver = self.lower_expr(&member.obj)?;
                    let array_type = self.infer_expr_type(&receiver)?;
                    let HirType::Array(element) = &array_type else {
                        return Err(format!(
                            "`.toSpliced()` requires a homogeneous array, got {array_type:?}"
                        ));
                    };
                    let element_type = element.as_ref().clone();
                    let mut arguments = Vec::with_capacity(call.args.len());
                    for (index, argument) in call.args.iter().enumerate() {
                        let value = self.lower_expr(&argument.expr)?;
                        let expected = if index < 2 {
                            &HirType::F64
                        } else {
                            &element_type
                        };
                        self.expect_type(expected, &value, "array toSpliced argument")?;
                        arguments.push(value);
                    }
                    return self.lower_array_to_spliced(
                        receiver,
                        array_type,
                        element_type,
                        arguments,
                    );
                }
                if property.sym == *"at" {
                    let [index] = call.args.as_slice() else {
                        return Err("native array `.at()` expects exactly one index".into());
                    };
                    if index.spread.is_some() {
                        return Err("array at spread is not supported".into());
                    }
                    let receiver = self.lower_expr(&member.obj)?;
                    let array_type = self.infer_expr_type(&receiver)?;
                    let HirType::Array(element) = &array_type else {
                        return Err(format!(
                            "array `.at()` requires a homogeneous array, got {array_type:?}"
                        ));
                    };
                    let element_type = element.as_ref().clone();
                    let index = self.lower_expr(&index.expr)?;
                    let index = self.coerce_primitive_to_number(index)?;
                    return self.lower_array_at(receiver, array_type, element_type, index);
                }
                if property.sym == *"with" {
                    let [index, value] = call.args.as_slice() else {
                        return Err("native `.with()` expects an index and value".into());
                    };
                    if index.spread.is_some() || value.spread.is_some() {
                        return Err("array with spread is not supported".into());
                    }
                    let receiver = self.lower_expr(&member.obj)?;
                    let array_type = self.infer_expr_type(&receiver)?;
                    let HirType::Array(element) = &array_type else {
                        return Err(format!(
                            "`.with()` requires a homogeneous array, got {array_type:?}"
                        ));
                    };
                    let element_type = element.as_ref().clone();
                    let index = self.lower_expr(&index.expr)?;
                    self.expect_type(&HirType::F64, &index, "array with index")?;
                    let value = self.lower_expr(&value.expr)?;
                    self.expect_type(&element_type, &value, "array with value")?;
                    return self.lower_array_with(receiver, array_type, element_type, index, value);
                }
                if property.sym == *"flat" {
                    if call.args.len() > 1 {
                        return Err("native `.flat()` expects zero or one depth".into());
                    }
                    if call.args.iter().any(|argument| argument.spread.is_some()) {
                        return Err("array flat spread is not supported".into());
                    }
                    let receiver = self.lower_expr(&member.obj)?;
                    let mut current_type = self.infer_expr_type(&receiver)?;
                    if !matches!(current_type, HirType::Array(_)) {
                        return Err(format!(
                            "`.flat()` requires a homogeneous array, got {current_type:?}"
                        ));
                    }
                    let depth = if let Some(argument) = call.args.first() {
                        let value = self.lower_expr(&argument.expr)?;
                        self.expect_type(&HirType::F64, &value, "array flat depth")?;
                        let constant = match value {
                            HirExpr::Lit(HirLit::F64(value)) => Some(value),
                            HirExpr::BinOp(BinOp::Sub, left, right) => match (*left, *right) {
                                (HirExpr::Lit(HirLit::F64(0.0)), HirExpr::Lit(HirLit::F64(value))) => {
                                    Some(-value)
                                }
                                _ => None,
                            },
                            HirExpr::Call(callee, arguments)
                                if matches!(
                                    callee.as_ref(),
                                    HirExpr::Var(name) if name == "__thaw_number_neg"
                                ) => match arguments.as_slice() {
                                    [HirExpr::Lit(HirLit::F64(value))] => Some(-value),
                                    _ => None,
                                },
                            _ => None,
                        }
                        .ok_or(
                            "native `.flat()` depth must be a numeric literal so its result layout is static",
                        )?;
                        if constant.is_nan() || constant <= 0.0 {
                            0usize
                        } else if constant.is_infinite() {
                            usize::MAX
                        } else {
                            constant.trunc() as usize
                        }
                    } else {
                        1
                    };
                    let mut result = receiver;
                    let mut flattened = false;
                    for _ in 0..depth {
                        let HirType::Array(element) = &current_type else {
                            unreachable!()
                        };
                        let HirType::Array(inner) = element.as_ref() else {
                            break;
                        };
                        let inner = inner.as_ref().clone();
                        result = self.lower_array_flat_one(result, current_type, inner.clone())?;
                        current_type = HirType::Array(Box::new(inner));
                        flattened = true;
                    }
                    if !flattened {
                        result = HirExpr::Call(
                            Box::new(HirExpr::Var("__thaw_array_slice".into())),
                            vec![
                                result,
                                HirExpr::Lit(HirLit::F64(0.0)),
                                HirExpr::Lit(HirLit::F64(f64::INFINITY)),
                            ],
                        );
                    }
                    return Ok(result);
                }
                if property.sym == *"flatMap" {
                    if !(1..=2).contains(&call.args.len()) {
                        return Err(
                            "native `.flatMap()` expects a callback and optional thisArg".into(),
                        );
                    }
                    if call.args.iter().any(|argument| argument.spread.is_some()) {
                        return Err("array flatMap spread is not supported".into());
                    }
                    let receiver = self.lower_expr(&member.obj)?;
                    let array_type = self.infer_expr_type(&receiver)?;
                    let HirType::Array(element) = &array_type else {
                        return Err(format!(
                            "`.flatMap()` requires a homogeneous array, got {array_type:?}"
                        ));
                    };
                    let element_type = element.as_ref().clone();
                    let callback = self.lower_array_mapping_callback(
                        &call.args[0].expr,
                        &element_type,
                        &array_type,
                    )?;
                    let this_arg = call
                        .args
                        .get(1)
                        .map(|argument| self.lower_expr(&argument.expr))
                        .transpose()?;
                    let mapped = self.lower_array_map(
                        receiver,
                        array_type,
                        element_type,
                        callback,
                        this_arg,
                    )?;
                    let mapped_type = self.infer_expr_type(&mapped)?;
                    let HirType::Array(mapped_element) = &mapped_type else {
                        unreachable!("array map always returns an array")
                    };
                    let HirType::Array(flat_element) = mapped_element.as_ref() else {
                        return Err(format!(
                            "native `.flatMap()` callback must return a homogeneous array, got {mapped_element:?}"
                        ));
                    };
                    let flat_element = flat_element.as_ref().clone();
                    return self.lower_array_flat_one(mapped, mapped_type, flat_element);
                }
                if property.sym == *"map" {
                    if !(1..=2).contains(&call.args.len()) {
                        return Err(
                            "native `.map()` expects a callback and optional thisArg".into()
                        );
                    }
                    if call.args.iter().any(|argument| argument.spread.is_some()) {
                        return Err("array mapper spread is not supported".into());
                    }
                    let receiver = self.lower_expr(&member.obj)?;
                    let array_type = self.infer_expr_type(&receiver)?;
                    let HirType::Array(element) = &array_type else {
                        return Err(format!(
                            "`.map()` requires a homogeneous array, got {array_type:?}"
                        ));
                    };
                    let element_type = element.as_ref().clone();
                    let callback = self.lower_array_mapping_callback(
                        &call.args[0].expr,
                        &element_type,
                        &array_type,
                    )?;
                    let this_arg = call
                        .args
                        .get(1)
                        .map(|argument| self.lower_expr(&argument.expr))
                        .transpose()?;
                    return self.lower_array_map(
                        receiver,
                        array_type,
                        element_type,
                        callback,
                        this_arg,
                    );
                }
                if property.sym == *"filter" {
                    if !(1..=2).contains(&call.args.len()) {
                        return Err(
                            "native `.filter()` expects a predicate and optional thisArg".into(),
                        );
                    }
                    if call.args.iter().any(|argument| argument.spread.is_some()) {
                        return Err("array filter spread is not supported".into());
                    }
                    let receiver = self.lower_expr(&member.obj)?;
                    let array_type = self.infer_expr_type(&receiver)?;
                    let HirType::Array(element) = &array_type else {
                        return Err(format!(
                            "`.filter()` requires a homogeneous array, got {array_type:?}"
                        ));
                    };
                    let element_type = element.as_ref().clone();
                    let callback = self.lower_array_callback(
                        &call.args[0].expr,
                        &element_type,
                        &array_type,
                        &HirType::Bool,
                    )?;
                    let this_arg = call
                        .args
                        .get(1)
                        .map(|argument| self.lower_expr(&argument.expr))
                        .transpose()?;
                    return self.lower_array_filter(
                        receiver,
                        array_type,
                        element_type,
                        callback,
                        this_arg,
                    );
                }
                if property.sym == *"forEach" {
                    if !(1..=2).contains(&call.args.len()) {
                        return Err(
                            "native `.forEach()` expects a callback and optional thisArg".into(),
                        );
                    }
                    if call.args.iter().any(|argument| argument.spread.is_some()) {
                        return Err("array forEach spread is not supported".into());
                    }
                    let receiver = self.lower_expr(&member.obj)?;
                    let array_type = self.infer_expr_type(&receiver)?;
                    let HirType::Array(element) = &array_type else {
                        return Err(format!(
                            "`.forEach()` requires a homogeneous array, got {array_type:?}"
                        ));
                    };
                    let element_type = element.as_ref().clone();
                    let callback = self.lower_array_callback(
                        &call.args[0].expr,
                        &element_type,
                        &array_type,
                        &HirType::Void,
                    )?;
                    let this_arg = call
                        .args
                        .get(1)
                        .map(|argument| self.lower_expr(&argument.expr))
                        .transpose()?;
                    return self.lower_array_for_each(
                        receiver,
                        array_type,
                        element_type,
                        callback,
                        this_arg,
                    );
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
                        HirType::Str | HirType::Array(_) | HirType::Object(_) => {
                            "__thaw_pointer_array_fill"
                        }
                        _ => "__thaw_array_fill",
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
                if property.sym == *"valueOf" {
                    if !call.args.is_empty() {
                        return Err("native `.valueOf()` expects no arguments".into());
                    }
                    let receiver = self.lower_expr(&member.obj)?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    if !matches!(receiver_type, HirType::F64 | HirType::Str | HirType::Bool) {
                        return Err(format!(
                            "native `.valueOf()` requires a number, string or boolean receiver, got {receiver_type:?}"
                        ));
                    }
                    return Ok(receiver);
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
            let property = member_property_name(&member.prop);
            if let Some(property) = property {
                let lowered_object = if let Expr::Ident(object) = member.obj.as_ref() {
                    let object_name = self.resolve_binding(object.sym.as_ref());
                    let object_ty = self
                        .narrowings
                        .get(&object_name)
                        .cloned()
                        .or_else(|| self.nullable_narrowings.get(&object_name).cloned())
                        .or_else(|| self.nullish_narrowings.get(&object_name).cloned())
                        .or_else(|| self.scope.get(&object_name).cloned());
                    object_ty
                        .map(|object_ty| {
                            self.lower_expr(&member.obj)
                                .map(|object_expr| (object_expr, object_ty, object.sym.to_string()))
                        })
                        .transpose()?
                } else {
                    let object_expr = self.lower_expr(&member.obj)?;
                    let object_ty = self.infer_expr_type(&object_expr)?;
                    Some((object_expr, object_ty, "<expression>".to_string()))
                };
                if let Some((object_expr, object_ty, object_label)) = lowered_object {
                    let requested_property = property.as_str();
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
                    let callable = match &object_ty {
                        HirType::Object(fields) => fields
                            .iter()
                            .find(|(name, _)| name == resolved_property)
                            .and_then(|(_, ty)| match ty {
                                HirType::Function(params, ret) => Some((
                                    params.clone(),
                                    ret.as_ref().clone(),
                                    None,
                                    HirOptionalMask::default(),
                                )),
                                HirType::CallableFunction(params, optional, rest, ret) => Some((
                                    params.clone(),
                                    ret.as_ref().clone(),
                                    rest.as_deref().cloned(),
                                    optional.clone(),
                                )),
                                _ => None,
                            }),
                        _ => None,
                    };
                    if let Some((params, _, rest, optional)) = callable {
                        let callee = HirExpr::PropAccess(
                            Box::new(object_expr),
                            object_ty,
                            resolved_property.to_string(),
                        );
                        let label = format!("method `{object_label}.{property}`");
                        let (mut args, bindings) =
                            self.lower_native_spread_values(&call.args, &label)?;
                        if args.len() < params.len()
                            && (args.len()..params.len())
                                .any(|index| !is_optional_parameter(&optional, index))
                        {
                            return Err(format!(
                                "method `{}.{}` expects at least {} argument(s), got {}",
                                object_label,
                                property,
                                params.len(),
                                args.len()
                            ));
                        }
                        let rest_values = rest.as_ref().map(|element| {
                            let values = args.split_off(params.len());
                            (element, values)
                        });
                        for parameter in params.iter().skip(args.len()) {
                            args.push(omitted_parameter_value(parameter)?);
                        }
                        let mut args = args
                            .into_iter()
                            .zip(&params)
                            .enumerate()
                            .map(|(index, (value, expected))| {
                                self.coerce_to_declared(expected, value).map_err(|error| {
                                    format!(
                                        "argument {} of `{}.{}` is invalid: {error}",
                                        index + 1,
                                        object_label,
                                        property
                                    )
                                })
                            })
                            .collect::<Result<Vec<_>, _>>()?;
                        if let Some((element, values)) = rest_values {
                            let values = values
                                .into_iter()
                                .map(|value| self.coerce_to_declared(element, value))
                                .collect::<Result<Vec<_>, _>>()?;
                            args.push(native_rest_array(values, element));
                        }
                        return self.wrap_call_argument_bindings(
                            HirExpr::Call(Box::new(callee), args),
                            &bindings,
                        );
                    }
                }
            }
        }

        let mut callee_name = match callee_expr.as_ref() {
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

        if let Some(arrow) = self.generic_arrows.get(&callee_name).cloned() {
            return self.lower_generic_arrow_call(&callee_name, &arrow, call);
        }
        if let Some(target) = self.generic_named_templates.get(&callee_name).cloned() {
            let mut forwarded = call.clone();
            forwarded.callee = Callee::Expr(Box::new(Expr::Ident(
                swc_ecma_ast::Ident::new_no_ctxt(target.into(), call.span),
            )));
            return self.lower_call(&forwarded);
        }

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

        let mut signature = self.signatures.get(&callee_name).cloned();
        let local_function = self.scope.get(&callee_name).and_then(|ty| match ty {
            HirType::Function(params, ret) => Some((
                params.clone(),
                ret.as_ref().clone(),
                None,
                HirOptionalMask::default(),
            )),
            HirType::CallableFunction(params, optional, rest, ret) => {
                let mut abi_params = params.clone();
                if let Some(rest) = rest {
                    abi_params.push(HirType::Array(rest.clone()));
                }
                Some((
                    abi_params,
                    ret.as_ref().clone(),
                    rest.as_deref().cloned(),
                    optional.clone(),
                ))
            }
            _ => None,
        });
        let mut param_types = signature
            .as_ref()
            .map(|sig| sig.params.clone())
            .or_else(|| {
                local_function
                    .as_ref()
                    .map(|(params, _, _, _)| params.clone())
            });

        let mut argument_bindings = Vec::new();
        let mut lowered_arguments = Vec::new();
        let mut lowered = Vec::with_capacity(call.args.len());
        for (index, argument) in call.args.iter().enumerate() {
            let contextual_function = if argument.spread.is_none() {
                param_types
                    .as_ref()
                    .and_then(|params| params.get(index))
                    .and_then(|expected| match expected {
                        HirType::Function(params, ret) => {
                            Some((params.clone(), ret.as_ref().clone()))
                        }
                        HirType::CallableFunction(params, _, rest, ret) => {
                            let mut abi_params = params.clone();
                            if let Some(rest) = rest {
                                abi_params.push(HirType::Array(rest.clone()));
                            }
                            Some((abi_params, ret.as_ref().clone()))
                        }
                        _ => None,
                    })
            } else {
                None
            };
            let value = if let Some((params, ret)) = contextual_function {
                if matches!(
                    argument.expr.as_ref(),
                    Expr::Arrow(_) | Expr::Fn(_) | Expr::Ident(_)
                ) {
                    self.lower_promise_callback(&argument.expr, &params, Some(&ret))?
                } else {
                    self.lower_expr(&argument.expr)?
                }
            } else {
                self.lower_expr(&argument.expr)?
            };
            lowered.push(value);
        }
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

        let native_rest_values = signature.as_ref().and_then(|signature| {
            signature.native_rest.clone().map(|element| {
                let fixed_count = signature.params.len() - 1;
                let values = if lowered_arguments.len() > fixed_count {
                    lowered_arguments.split_off(fixed_count)
                } else {
                    Vec::new()
                };
                (element, values)
            })
        });
        let local_rest_values = local_function
            .as_ref()
            .and_then(|(_, _, rest, _)| rest.clone())
            .map(|element| {
                let fixed_count = param_types
                    .as_ref()
                    .map_or(0, |params| params.len().saturating_sub(1));
                let values = if lowered_arguments.len() > fixed_count {
                    lowered_arguments.split_off(fixed_count)
                } else {
                    Vec::new()
                };
                (element, values)
            });

        if let Some((fixed, _, _, optional)) = &local_function {
            let fixed_count = fixed.len() - usize::from(local_rest_values.is_some());
            if lowered_arguments.len() < fixed_count {
                for (index, parameter) in fixed
                    .iter()
                    .enumerate()
                    .take(fixed_count)
                    .skip(lowered_arguments.len())
                {
                    if !is_optional_parameter(optional, index) {
                        return Err(format!(
                            "function `{callee_name}` expects argument {}, but it was omitted",
                            index + 1
                        ));
                    }
                    lowered_arguments.push(omitted_parameter_value(parameter)?);
                }
            }
        }

        if let Some(full_signature) = signature.clone() {
            let logical_param_count =
                full_signature.params.len() - usize::from(full_signature.native_rest.is_some());
            if full_signature.variadic.is_none()
                && logical_param_count < usize::BITS as usize
                && lowered_arguments.len() <= logical_param_count
            {
                let mut omitted_mask = 0usize;
                for index in 0..logical_param_count {
                    let omitted = match lowered_arguments.get(index) {
                        None => true,
                        Some(value) => self.infer_expr_type(value)? == HirType::Undefined,
                    };
                    if omitted {
                        omitted_mask |= 1usize << index;
                    }
                }
                if omitted_mask != 0 {
                    let wrapper = omitted_parameter_symbol(&callee_name, omitted_mask);
                    if let Some(wrapper_signature) = self.signatures.get(&wrapper).cloned() {
                        lowered_arguments = lowered_arguments
                            .into_iter()
                            .enumerate()
                            .filter_map(|(index, argument)| {
                                (omitted_mask & (1usize << index) == 0).then_some(argument)
                            })
                            .collect();
                        callee_name = wrapper;
                        param_types = Some(wrapper_signature.params.clone());
                        signature = Some(wrapper_signature);
                    }
                }
            }
        }

        if let Some((element, values)) = native_rest_values {
            let values = values
                .into_iter()
                .map(|value| self.coerce_to_declared(&element, value))
                .collect::<Result<Vec<_>, _>>()?;
            lowered_arguments.push(native_rest_array(values, &element));
            param_types = signature.as_ref().map(|signature| signature.params.clone());
        }
        if let Some((element, values)) = local_rest_values {
            let values = values
                .into_iter()
                .map(|value| self.coerce_to_declared(&element, value))
                .collect::<Result<Vec<_>, _>>()?;
            lowered_arguments.push(native_rest_array(values, &element));
        }

        if signature.as_ref().is_some_and(|signature| {
            signature.variadic.is_none()
                && signature.native_rest.is_none()
                && lowered_arguments.len() != signature.params.len()
        }) {
            let wrapper = default_arity_symbol(&callee_name, lowered_arguments.len());
            if let Some(wrapper_signature) = self.signatures.get(&wrapper).cloned() {
                callee_name = wrapper;
                param_types = Some(wrapper_signature.params.clone());
                signature = Some(wrapper_signature);
            }
        }

        if let Some(params) = &param_types {
            let variadic = signature.as_ref().and_then(|sig| sig.variadic.as_ref());
            let wrong_count = if variadic.is_some() {
                lowered_arguments.len() < params.len()
            } else {
                lowered_arguments.len() != params.len()
            };
            if wrong_count {
                return Err(format!(
                    "function `{callee_name}` expects {}{} argument(s), got {}",
                    if variadic.is_some() { "at least " } else { "" },
                    params.len(),
                    lowered_arguments.len()
                ));
            }
        }

        let args = lowered_arguments
            .into_iter()
            .enumerate()
            .map(|(i, value)| {
                match param_types
                    .as_ref()
                    .and_then(|p| p.get(i))
                    .or_else(|| signature.as_ref().and_then(|sig| sig.variadic.as_ref()))
                {
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
            let types = if let Some(type_args) = &call.type_args {
                resolve_explicit_generic_type_tuple(
                    signature,
                    &type_args.params,
                    &actual,
                    self.interfaces,
                    self.generic_interfaces,
                )
            } else {
                infer_generic_type_tuple(
                    signature,
                    &actual,
                    self.interfaces,
                    self.generic_interfaces,
                )
            }
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
            if call.type_args.is_some() {
                return Err(format!(
                    "non-generic function `{callee_name}` does not accept type arguments"
                ));
            }
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
                variadic: sig.variadic,
                variadic_abi: crate::FfiVariadicAbi::Native,
                ret: sig.ret,
                error_abi: FfiErrorAbi::Direct,
                return_ownership: FfiOwnership::Borrowed,
                error_ownership: FfiOwnership::Borrowed,
                param_string_abis: vec![FfiStringAbi::NullTerminated; param_count],
                return_string_abi: FfiStringAbi::NullTerminated,
                calling_convention: FfiCallingConvention::C,
                aggregate_return_abi: FfiAggregateAbi::Internal,
                aggregate_return_layout: None,
            };
            let result = HirExpr::FfiCall(Box::new(ffi_signature), args);
            return self.wrap_call_argument_bindings(result, &argument_bindings);
        }

        let lowered_name = if let Some(types) = generic_types
            .as_ref()
            .filter(|types| !types.contains(&HirType::Dynamic))
        {
            let param_types = args
                .iter()
                .map(|arg| self.infer_expr_type(arg))
                .collect::<Result<Vec<_>, _>>()?;
            let signature = signature
                .as_ref()
                .expect("generic types require a generic signature");
            let lowered_name =
                specialized_generic_function_name(&callee_name, &param_types, signature, types);
            let substitution = signature
                .generic_type_params
                .iter()
                .cloned()
                .zip(types.iter().cloned())
                .collect::<HashMap<_, _>>();
            let return_type = resolve_ts_type_with_substitution(
                signature
                    .generic_return_type
                    .as_ref()
                    .expect("generic return type"),
                &substitution,
                self.interfaces,
                self.generic_interfaces,
                &mut Vec::new(),
            )?;
            self.generic_call_returns
                .insert(lowered_name.clone(), return_type);
            lowered_name
        } else {
            callee_name
        };
        let result = HirExpr::Call(Box::new(HirExpr::Var(lowered_name)), args);
        self.wrap_call_argument_bindings(result, &argument_bindings)
    }

    fn lower_generic_arrow_call(
        &mut self,
        name: &str,
        arrow: &swc_ecma_ast::ArrowExpr,
        call: &CallExpr,
    ) -> Result<HirExpr, String> {
        let lowered = call
            .args
            .iter()
            .map(|argument| self.lower_expr(&argument.expr))
            .collect::<Result<Vec<_>, _>>()?;
        let preserve_order = call.args.iter().any(|argument| argument.spread.is_some())
            || lowered.iter().any(contains_await);
        let mut bindings = Vec::new();
        let mut arguments = Vec::new();
        for (source, value) in call.args.iter().zip(lowered) {
            if source.spread.is_none() && !preserve_order {
                arguments.push(value);
                continue;
            }
            if source.spread.is_some() {
                if let HirExpr::ArrayLit(elements) = value {
                    for element in elements {
                        let ty = self.infer_expr_type(&element)?;
                        let temporary = format!("__thaw_generic_arrow_arg_{}", self.next_binding);
                        self.next_binding += 1;
                        self.scope.insert(temporary.clone(), ty.clone());
                        bindings.push((temporary.clone(), ty, element));
                        arguments.push(HirExpr::Var(temporary));
                    }
                    continue;
                }
                let source_type = self.infer_expr_type(&value)?;
                let HirType::Tuple(elements) = &source_type else {
                    return Err(format!(
                        "generic arrow `{name}` spread source must have a statically known tuple length, got {source_type:?}"
                    ));
                };
                let elements = elements.clone();
                let temporary = format!("__thaw_generic_arrow_spread_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(temporary.clone(), source_type.clone());
                bindings.push((temporary.clone(), source_type, value));
                arguments.extend(elements.into_iter().enumerate().map(|(index, ty)| {
                    HirExpr::TypedIndex(
                        Box::new(HirExpr::Var(temporary.clone())),
                        Box::new(HirExpr::Lit(HirLit::F64(index as f64))),
                        ty,
                    )
                }));
                continue;
            }
            let ty = self.infer_expr_type(&value)?;
            let temporary = format!("__thaw_generic_arrow_arg_{}", self.next_binding);
            self.next_binding += 1;
            self.scope.insert(temporary.clone(), ty.clone());
            bindings.push((temporary.clone(), ty, value));
            arguments.push(HirExpr::Var(temporary));
        }
        if arrow.params.len() != arguments.len() {
            return Err(format!(
                "generic arrow `{name}` expects {} argument(s), got {} after spread expansion",
                arrow.params.len(),
                arguments.len()
            ));
        }
        let parameter_types = arguments
            .iter()
            .map(|argument| self.infer_expr_type(argument))
            .collect::<Result<Vec<_>, _>>()?;
        let signature = self.generic_arrow_signature(arrow)?;
        let concrete_types = if let Some(type_args) = &call.type_args {
            resolve_explicit_generic_type_tuple(
                &signature,
                &type_args.params,
                &parameter_types,
                self.interfaces,
                self.generic_interfaces,
            )
            .map_err(|error| {
                format!("cannot explicitly specialize generic arrow `{name}`: {error}")
            })?
        } else {
            infer_generic_type_tuple(
                &signature,
                &parameter_types,
                self.interfaces,
                self.generic_interfaces,
            )
            .map_err(|error| format!("cannot specialize generic arrow `{name}`: {error}"))?
        };
        let recursive = self.generic_arrow_self_names.get(name).cloned();
        let mut recursive_state = None;
        if let Some(internal) = recursive {
            let return_type = signature.generic_return_type.as_ref().ok_or_else(|| {
                format!("recursive generic local function `{name}` needs a return annotation")
            })?;
            let substitution = signature
                .generic_type_params
                .iter()
                .cloned()
                .zip(concrete_types.iter().cloned())
                .collect::<HashMap<_, _>>();
            let return_type = resolve_ts_type_with_substitution(
                return_type,
                &substitution,
                self.interfaces,
                self.generic_interfaces,
                &mut Vec::new(),
            )?;
            let self_type = HirType::Function(parameter_types.clone(), Box::new(return_type));
            let self_name = format!("__thaw_recursive_generic_{}", self.next_binding);
            self.next_binding += 1;
            let saved_binding = self.bindings.get(&internal).cloned();
            self.scope.insert(self_name.clone(), self_type.clone());
            self.bindings
                .entry(internal.clone())
                .or_default()
                .push(self_name.clone());
            recursive_state = Some((internal, self_name, self_type, saved_binding));
        }
        let lowered = self.lower_contextual_arrow(arrow, &parameter_types, None);
        if let Some((internal, self_name, _, saved_binding)) = &recursive_state {
            self.scope.remove(self_name);
            if let Some(saved) = saved_binding {
                self.bindings.insert(internal.clone(), saved.clone());
            } else {
                self.bindings.remove(internal);
            }
        }
        let mut lambda = lowered
            .map_err(|error| format!("cannot specialize generic arrow `{name}`: {error}"))?;
        if let Some((_, self_name, self_type, _)) = recursive_state {
            lambda = HirExpr::RecursiveClosure(self_name, self_type, Box::new(lambda));
        }
        let result = HirExpr::Call(Box::new(lambda), arguments);
        self.wrap_call_argument_bindings(result, &bindings)
    }

    fn generic_arrow_signature(
        &self,
        arrow: &swc_ecma_ast::ArrowExpr,
    ) -> Result<FnSignature, String> {
        let type_params = arrow.type_params.as_ref().ok_or("arrow is not generic")?;
        validate_trailing_type_parameter_defaults(
            "generic arrow function",
            "<anonymous>",
            type_params,
        )?;
        let generic_type_params = type_params
            .params
            .iter()
            .map(|parameter| parameter.name.sym.to_string())
            .collect::<Vec<_>>();
        let substitutions = generic_type_params
            .iter()
            .map(|name| (name.clone(), GenericTypePattern::Variable(name.clone())))
            .collect::<HashMap<_, _>>();
        let generic_param_patterns = arrow
            .params
            .iter()
            .map(|parameter| {
                let Pat::Ident(binding) = parameter else {
                    return Err("generic arrows require identifier parameters".into());
                };
                let annotation = binding
                    .type_ann
                    .as_ref()
                    .ok_or("generic arrow parameters need type annotations")?;
                generic_type_pattern(
                    &annotation.type_ann,
                    &substitutions,
                    self.interfaces,
                    self.generic_interfaces,
                    &mut Vec::new(),
                )
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(FnSignature {
            params: Vec::new(),
            variadic: None,
            native_rest: None,
            abstract_class_constructor: false,
            ret: HirType::Dynamic,
            is_async: false,
            uses_this: false,
            is_extern: false,
            source_range: (arrow.span.lo.0, arrow.span.hi.0),
            generic_type_params,
            generic_type_constraints: type_params
                .params
                .iter()
                .map(|parameter| parameter.constraint.clone())
                .collect(),
            generic_type_defaults: type_params
                .params
                .iter()
                .map(|parameter| parameter.default.clone())
                .collect(),
            generic_param_patterns,
            generic_param_optional: arrow
                .params
                .iter()
                .map(|parameter| matches!(parameter, Pat::Ident(binding) if binding.id.optional))
                .collect(),
            generic_return_type: arrow
                .return_type
                .as_ref()
                .map(|annotation| annotation.type_ann.clone())
                .or_else(|| inferred_generic_arrow_return_type(arrow)),
        })
    }

    fn generic_callable_annotation_signature(
        &self,
        ty: &TsType,
    ) -> Result<Option<(FnSignature, Symbol)>, String> {
        let TsType::TsTypeRef(reference) = strip_parenthesized_ts_type(ty) else {
            return Ok(None);
        };
        let swc_ecma_ast::TsEntityName::Ident(identifier) = &reference.type_name else {
            return Ok(None);
        };
        if reference.type_params.is_some() {
            return Ok(None);
        }
        let callable_name = identifier.sym.to_string();
        let mut target = callable_name.clone();
        let mut seen = BTreeSet::new();
        loop {
            if let Some(alias) = self.generic_interfaces.function_aliases.get(&target) {
                return Ok(Some((
                    self.generic_function_alias_signature(alias)?,
                    callable_name,
                )));
            }
            if let Some(interface) = self.generic_interfaces.function_interfaces.get(&target) {
                return Ok(Some((
                    self.generic_function_interface_signature(interface)?,
                    callable_name,
                )));
            }
            let Some(next) = self
                .generic_interfaces
                .function_alias_chains
                .get(&target)
                .or_else(|| {
                    self.generic_interfaces
                        .function_interface_chains
                        .get(&target)
                })
                .cloned()
            else {
                return Ok(None);
            };
            if !seen.insert(target.clone()) {
                return Err(format!(
                    "cyclic generic callable type alias `{callable_name}`"
                ));
            }
            target = next;
        }
    }

    fn generic_function_interface_signature(
        &self,
        interface: &TsInterfaceDecl,
    ) -> Result<FnSignature, String> {
        let [TsTypeElement::TsCallSignatureDecl(call)] = interface.body.body.as_slice() else {
            return Err(format!(
                "callable interface `{}` must contain exactly one call signature",
                interface.id.sym
            ));
        };
        let type_params = call.type_params.as_ref().ok_or_else(|| {
            format!(
                "callable interface `{}` does not have a generic call signature",
                interface.id.sym
            )
        })?;
        validate_trailing_type_parameter_defaults(
            "generic callable interface",
            interface.id.sym.as_ref(),
            type_params,
        )?;
        let generic_type_params = type_params
            .params
            .iter()
            .map(|parameter| parameter.name.sym.to_string())
            .collect::<Vec<_>>();
        let substitutions = generic_type_params
            .iter()
            .map(|name| (name.clone(), GenericTypePattern::Variable(name.clone())))
            .collect::<HashMap<_, _>>();
        let generic_param_patterns = call
            .params
            .iter()
            .map(|parameter| {
                let TsFnParam::Ident(parameter) = parameter else {
                    return Err(format!(
                        "generic callable interface `{}` requires identifier parameters",
                        interface.id.sym
                    ));
                };
                let annotation = parameter.type_ann.as_ref().ok_or_else(|| {
                    format!(
                        "generic callable interface `{}` parameter `{}` needs an annotation",
                        interface.id.sym, parameter.id.sym
                    )
                })?;
                generic_type_pattern(
                    &annotation.type_ann,
                    &substitutions,
                    self.interfaces,
                    self.generic_interfaces,
                    &mut Vec::new(),
                )
            })
            .collect::<Result<Vec<_>, String>>()?;
        let return_type = call.type_ann.as_ref().ok_or_else(|| {
            format!(
                "generic callable interface `{}` needs a return type",
                interface.id.sym
            )
        })?;
        Ok(FnSignature {
            params: Vec::new(),
            variadic: None,
            native_rest: None,
            abstract_class_constructor: false,
            ret: HirType::Dynamic,
            is_async: false,
            uses_this: false,
            is_extern: false,
            source_range: (interface.span.lo.0, interface.span.hi.0),
            generic_type_params,
            generic_type_constraints: type_params
                .params
                .iter()
                .map(|parameter| parameter.constraint.clone())
                .collect(),
            generic_type_defaults: type_params
                .params
                .iter()
                .map(|parameter| parameter.default.clone())
                .collect(),
            generic_param_patterns,
            generic_param_optional: call
                .params
                .iter()
                .map(|parameter| {
                    matches!(parameter, TsFnParam::Ident(binding) if binding.id.optional)
                })
                .collect(),
            generic_return_type: Some(return_type.type_ann.clone()),
        })
    }

    fn generic_function_alias_signature(
        &self,
        alias: &swc_ecma_ast::TsTypeAliasDecl,
    ) -> Result<FnSignature, String> {
        let (type_params, params, return_type) = match strip_parenthesized_ts_type(&alias.type_ann)
        {
            TsType::TsFnOrConstructorType(TsFnOrConstructorType::TsFnType(function)) => (
                function.type_params.as_ref(),
                function.params.as_slice(),
                Some(function.type_ann.type_ann.as_ref()),
            ),
            TsType::TsTypeLit(literal) => {
                let [TsTypeElement::TsCallSignatureDecl(call)] = literal.members.as_slice() else {
                    return Err(format!(
                        "type alias `{}` is not a generic callable type",
                        alias.id.sym
                    ));
                };
                (
                    call.type_params.as_ref(),
                    call.params.as_slice(),
                    call.type_ann
                        .as_ref()
                        .map(|annotation| annotation.type_ann.as_ref()),
                )
            }
            _ => {
                return Err(format!(
                    "type alias `{}` is not a generic callable type",
                    alias.id.sym
                ))
            }
        };
        let type_params = type_params
            .as_ref()
            .ok_or_else(|| format!("callable type alias `{}` is not generic", alias.id.sym))?;
        validate_trailing_type_parameter_defaults(
            "generic function type alias",
            alias.id.sym.as_ref(),
            type_params,
        )?;
        let generic_type_params = type_params
            .params
            .iter()
            .map(|parameter| parameter.name.sym.to_string())
            .collect::<Vec<_>>();
        let substitutions = generic_type_params
            .iter()
            .map(|name| (name.clone(), GenericTypePattern::Variable(name.clone())))
            .collect::<HashMap<_, _>>();
        let generic_param_patterns = params
            .iter()
            .map(|parameter| {
                let TsFnParam::Ident(parameter) = parameter else {
                    return Err(format!(
                        "generic function type alias `{}` requires identifier parameters",
                        alias.id.sym
                    ));
                };
                let annotation = parameter.type_ann.as_ref().ok_or_else(|| {
                    format!(
                        "generic function type alias `{}` parameter `{}` needs an annotation",
                        alias.id.sym, parameter.id.sym
                    )
                })?;
                generic_type_pattern(
                    &annotation.type_ann,
                    &substitutions,
                    self.interfaces,
                    self.generic_interfaces,
                    &mut Vec::new(),
                )
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(FnSignature {
            params: Vec::new(),
            variadic: None,
            native_rest: None,
            abstract_class_constructor: false,
            ret: HirType::Dynamic,
            is_async: false,
            uses_this: false,
            is_extern: false,
            source_range: (alias.span.lo.0, alias.span.hi.0),
            generic_type_params,
            generic_type_constraints: type_params
                .params
                .iter()
                .map(|parameter| parameter.constraint.clone())
                .collect(),
            generic_type_defaults: type_params
                .params
                .iter()
                .map(|parameter| parameter.default.clone())
                .collect(),
            generic_param_patterns,
            generic_param_optional: params
                .iter()
                .map(|parameter| {
                    matches!(parameter, TsFnParam::Ident(binding) if binding.id.optional)
                })
                .collect(),
            generic_return_type: Some(Box::new(
                return_type
                    .ok_or_else(|| {
                        format!(
                            "generic callable type alias `{}` needs a return type",
                            alias.id.sym
                        )
                    })?
                    .clone(),
            )),
        })
    }

    fn validate_generic_callable_shape(
        &self,
        expected: &FnSignature,
        actual: &FnSignature,
        alias_name: &str,
    ) -> Result<(), String> {
        if actual.generic_return_type.is_none() {
            return Err(format!(
                "generic callable assigned to function type alias `{alias_name}` needs an explicit return type"
            ));
        }
        if expected.generic_type_params.len() != actual.generic_type_params.len()
            || expected.generic_param_patterns.len() != actual.generic_param_patterns.len()
        {
            return Err(format!(
                "generic arrow does not match function type alias `{alias_name}` arity"
            ));
        }
        if expected.generic_param_optional != actual.generic_param_optional {
            return Err(format!(
                "generic callable optional parameters do not match function type alias `{alias_name}`"
            ));
        }
        let canonical = (0..expected.generic_type_params.len())
            .map(|index| {
                HirType::Object(vec![(format!("__generic_parameter_{index}"), HirType::F64)])
            })
            .collect::<Vec<_>>();
        let expected_substitution = expected
            .generic_type_params
            .iter()
            .cloned()
            .zip(canonical.iter().cloned())
            .collect::<HashMap<_, _>>();
        let actual_substitution = actual
            .generic_type_params
            .iter()
            .cloned()
            .zip(canonical.iter().cloned())
            .collect::<HashMap<_, _>>();
        let expected_params = expected
            .generic_param_patterns
            .iter()
            .map(|pattern| instantiate_generic_pattern(pattern, &expected_substitution))
            .collect::<Result<Vec<_>, _>>()?;
        let actual_params = actual
            .generic_param_patterns
            .iter()
            .map(|pattern| instantiate_generic_pattern(pattern, &actual_substitution))
            .collect::<Result<Vec<_>, _>>()?;
        let expected_return = resolve_ts_type_with_substitution(
            expected
                .generic_return_type
                .as_ref()
                .expect("generic alias return type"),
            &expected_substitution,
            self.interfaces,
            self.generic_interfaces,
            &mut Vec::new(),
        )?;
        let mut actual_return = resolve_ts_type_with_substitution(
            actual
                .generic_return_type
                .as_ref()
                .expect("generic callable return type was validated"),
            &actual_substitution,
            self.interfaces,
            self.generic_interfaces,
            &mut Vec::new(),
        )?;
        if actual.is_async && !matches!(actual_return, HirType::Promise(_)) {
            actual_return = HirType::Promise(Box::new(actual_return));
        }
        if expected_params != actual_params || expected_return != actual_return {
            return Err(format!(
                "generic arrow has signature {actual_params:?} -> {actual_return:?}, incompatible with function type alias `{alias_name}` {expected_params:?} -> {expected_return:?}"
            ));
        }
        for (expected_constraint, actual_constraint) in expected
            .generic_type_constraints
            .iter()
            .zip(&actual.generic_type_constraints)
        {
            let expected_constraint = expected_constraint
                .as_ref()
                .map(|constraint| {
                    resolve_ts_type_with_substitution(
                        constraint,
                        &expected_substitution,
                        self.interfaces,
                        self.generic_interfaces,
                        &mut Vec::new(),
                    )
                })
                .transpose()?;
            let actual_constraint = actual_constraint
                .as_ref()
                .map(|constraint| {
                    resolve_ts_type_with_substitution(
                        constraint,
                        &actual_substitution,
                        self.interfaces,
                        self.generic_interfaces,
                        &mut Vec::new(),
                    )
                })
                .transpose()?;
            if expected_constraint != actual_constraint {
                return Err(format!(
                    "generic arrow constraints do not match function type alias `{alias_name}`"
                ));
            }
        }
        for (expected_default, actual_default) in expected
            .generic_type_defaults
            .iter()
            .zip(&actual.generic_type_defaults)
        {
            let expected_default = expected_default
                .as_ref()
                .map(|default| {
                    resolve_ts_type_with_substitution(
                        default,
                        &expected_substitution,
                        self.interfaces,
                        self.generic_interfaces,
                        &mut Vec::new(),
                    )
                })
                .transpose()?;
            let actual_default = actual_default
                .as_ref()
                .map(|default| {
                    resolve_ts_type_with_substitution(
                        default,
                        &actual_substitution,
                        self.interfaces,
                        self.generic_interfaces,
                        &mut Vec::new(),
                    )
                })
                .transpose()?;
            if expected_default != actual_default {
                return Err(format!(
                    "generic arrow defaults do not match function type alias `{alias_name}`"
                ));
            }
        }
        Ok(())
    }

    fn lower_optional_call(&mut self, call: &swc_ecma_ast::OptCall) -> Result<HirExpr, String> {
        let optional_member = match call.callee.as_ref() {
            Expr::Member(member) => Some(member),
            Expr::OptChain(chain) => match chain.base.as_ref() {
                OptChainBase::Member(member) => Some(member),
                OptChainBase::Call(_) => None,
            },
            _ => None,
        };
        if let Some(member) = optional_member {
            let receiver = self.lower_expr(&member.obj)?;
            let receiver_type = self.infer_expr_type(&receiver)?;
            if let Some((payload, absence_kind)) = match receiver_type.clone() {
                HirType::Optional(payload) => Some((payload, 0)),
                HirType::Nullable(payload) => Some((payload, 1)),
                HirType::Nullish(payload) => Some((payload, 2)),
                _ => None,
            } {
                let name = format!("__thaw_optional_method_receiver_{}", self.next_binding);
                self.next_binding += 1;
                self.scope.insert(name.clone(), receiver_type.clone());
                match absence_kind {
                    0 => {
                        self.narrowings
                            .insert(name.clone(), payload.as_ref().clone());
                    }
                    1 => {
                        self.nullable_narrowings
                            .insert(name.clone(), payload.as_ref().clone());
                    }
                    2 => {
                        self.nullish_narrowings
                            .insert(name.clone(), payload.as_ref().clone());
                    }
                    _ => unreachable!(),
                }

                let mut rebound = member.clone();
                rebound.obj = Box::new(Expr::Ident(swc_ecma_ast::Ident::new_no_ctxt(
                    name.clone().into(),
                    member.span,
                )));
                let mut ordinary = CallExpr::from(call.clone());
                ordinary.callee = Callee::Expr(Box::new(Expr::Member(rebound)));
                let invoked = self.lower_call(&ordinary);
                self.narrowings.remove(&name);
                self.nullable_narrowings.remove(&name);
                self.nullish_narrowings.remove(&name);
                let invoked = invoked?;
                let return_type = self.infer_expr_type(&invoked)?;
                let bound = HirExpr::Var(name.clone());
                let is_none = |bound: HirExpr| match absence_kind {
                    0 => HirExpr::OptionalIsNone(Box::new(bound), payload.as_ref().clone()),
                    1 => HirExpr::NullableIsNone(Box::new(bound), payload.as_ref().clone()),
                    2 => HirExpr::NullishIsNone(Box::new(bound), payload.as_ref().clone()),
                    _ => unreachable!(),
                };
                let result = if return_type == HirType::Void {
                    HirExpr::Block(vec![HirStmt::If(
                        is_none(bound),
                        vec![HirStmt::Return(Some(HirExpr::Lit(HirLit::Undefined)))],
                        vec![
                            HirStmt::Expr(invoked),
                            HirStmt::Return(Some(HirExpr::Lit(HirLit::Undefined))),
                        ],
                    )])
                } else {
                    let (result_payload, present) = match &return_type {
                        HirType::Optional(inner) => (inner.as_ref().clone(), invoked),
                        output => (
                            output.clone(),
                            HirExpr::OptionalSome(Box::new(invoked), output.clone()),
                        ),
                    };
                    HirExpr::Block(vec![HirStmt::If(
                        is_none(bound),
                        vec![HirStmt::Return(Some(HirExpr::OptionalNone(result_payload)))],
                        vec![HirStmt::Return(Some(present))],
                    )])
                };
                return self
                    .wrap_call_argument_bindings(result, &[(name, receiver_type, receiver)]);
            }
            let mut ordinary = CallExpr::from(call.clone());
            ordinary.callee = Callee::Expr(Box::new(Expr::Member(member.clone())));
            return self.lower_call(&ordinary);
        }

        let callee = self.lower_expr(&call.callee)?;
        let callee_type = self.infer_expr_type(&callee)?;
        let (payload, absence_kind) = match callee_type.clone() {
            HirType::Optional(payload) => (payload, 0),
            HirType::Nullable(payload) => (payload, 1),
            HirType::Nullish(payload) => (payload, 2),
            _ => return self.lower_call(&CallExpr::from(call.clone())),
        };
        let (params, optional, rest, return_type) = match payload.as_ref() {
            HirType::Function(params, ret) => (
                params.clone(),
                HirOptionalMask::default(),
                None,
                ret.as_ref().clone(),
            ),
            HirType::CallableFunction(params, optional, rest, ret) => (
                params.clone(),
                optional.clone(),
                rest.as_deref().cloned(),
                ret.as_ref().clone(),
            ),
            _ => {
                return Err(format!(
                    "optional call requires a function payload, got {payload:?}"
                ))
            }
        };
        if call.type_args.is_some() {
            return Err("optional native calls do not accept type arguments".into());
        }
        if call.args.iter().any(|argument| argument.spread.is_some()) {
            return Err("optional native call spread arguments are not supported".into());
        }
        if call.args.len() > params.len() && rest.is_none() {
            return Err(format!(
                "optional function accepts {} argument(s), got {}",
                params.len(),
                call.args.len()
            ));
        }
        if call.args.len() < params.len()
            && (call.args.len()..params.len()).any(|index| !is_optional_parameter(&optional, index))
        {
            return Err(format!(
                "optional function requires at least {} argument(s), got {}",
                optional.first_at_or_after(0).unwrap_or(params.len()),
                call.args.len()
            ));
        }
        let mut arguments = call
            .args
            .iter()
            .map(|argument| self.lower_expr(&argument.expr))
            .collect::<Result<Vec<_>, String>>()?;
        let rest_values = rest.as_ref().map(|element| {
            let values = if arguments.len() > params.len() {
                arguments.split_off(params.len())
            } else {
                Vec::new()
            };
            (element, values)
        });
        for parameter in params.iter().skip(arguments.len()) {
            arguments.push(omitted_parameter_value(parameter)?);
        }
        let mut arguments = arguments
            .into_iter()
            .zip(&params)
            .map(|(value, expected)| self.coerce_to_declared(expected, value))
            .collect::<Result<Vec<_>, String>>()?;
        if let Some((element, values)) = rest_values {
            let values = values
                .into_iter()
                .map(|value| self.coerce_to_declared(element, value))
                .collect::<Result<Vec<_>, _>>()?;
            arguments.push(native_rest_array(values, element));
        }

        let name = format!("__thaw_optional_callee_{}", self.next_binding);
        self.next_binding += 1;
        self.scope.insert(name.clone(), callee_type.clone());
        let bound = HirExpr::Var(name.clone());
        let function = match absence_kind {
            0 => HirExpr::OptionalValue(Box::new(bound.clone()), payload.as_ref().clone()),
            1 => HirExpr::NullableValue(Box::new(bound.clone()), payload.as_ref().clone()),
            2 => HirExpr::NullishValue(Box::new(bound.clone()), payload.as_ref().clone()),
            _ => unreachable!(),
        };
        let invoked = HirExpr::Call(Box::new(function), arguments);
        let is_none = |bound: HirExpr| match absence_kind {
            0 => HirExpr::OptionalIsNone(Box::new(bound), payload.as_ref().clone()),
            1 => HirExpr::NullableIsNone(Box::new(bound), payload.as_ref().clone()),
            2 => HirExpr::NullishIsNone(Box::new(bound), payload.as_ref().clone()),
            _ => unreachable!(),
        };
        let result = if return_type == HirType::Void {
            HirExpr::Block(vec![HirStmt::If(
                is_none(bound),
                vec![HirStmt::Return(Some(HirExpr::Lit(HirLit::Undefined)))],
                vec![
                    HirStmt::Expr(invoked),
                    HirStmt::Return(Some(HirExpr::Lit(HirLit::Undefined))),
                ],
            )])
        } else {
            let (result_payload, present) = match &return_type {
                HirType::Optional(inner) => (inner.as_ref().clone(), invoked),
                output => (
                    output.clone(),
                    HirExpr::OptionalSome(Box::new(invoked), output.clone()),
                ),
            };
            HirExpr::Block(vec![HirStmt::If(
                is_none(bound),
                vec![HirStmt::Return(Some(HirExpr::OptionalNone(result_payload)))],
                vec![HirStmt::Return(Some(present))],
            )])
        };
        self.wrap_call_argument_bindings(result, &[(name, callee_type, callee)])
    }
}

#[cfg(test)]
mod tests;
