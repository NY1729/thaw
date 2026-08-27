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

include!("lower/classes/normalization.rs");
include!("lower/classes/generic_classes.rs");
include!("lower/classes/generic_methods.rs");

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

impl<'a> FnLowerer<'a> {}

include!("lower/expressions.rs");

impl<'a> FnLowerer<'a> {}

include!("lower/promises.rs");

impl<'a> FnLowerer<'a> {}

include!("lower/objects.rs");

impl<'a> FnLowerer<'a> {}

include!("lower/assignments.rs");

impl<'a> FnLowerer<'a> {}

include!("lower/arrays.rs");

impl<'a> FnLowerer<'a> {}

include!("lower/invocations.rs");

impl<'a> FnLowerer<'a> {}

include!("lower/generic_calls.rs");

#[cfg(test)]
mod tests;
