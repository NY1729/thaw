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
    ClassMember, ComputedPropName, Decl, Expr, FnDecl, ForHead, KeyValueProp, Lit, MemberExpr,
    MemberProp, MethodKind, Module, ModuleDecl, ModuleItem, ObjectLit as SwcObjectLit,
    ObjectPatProp, OptChainBase, ParamOrTsParamProp, Pat, Prop, PropName, PropOrSpread,
    SimpleAssignTarget, Stmt, SuperProp, TsFnOrConstructorType, TsFnParam, TsInterfaceDecl,
    TsKeywordTypeKind, TsParamPropParam, TsType, TsTypeElement, TsUnionOrIntersectionType, UnaryOp,
    UpdateOp, VarDecl, VarDeclOrExpr,
};
use swc_ecma_visit::{Visit, VisitWith};

use crate::{
    BinOp, DynamicBackend, DynamicSignature, FfiAggregateAbi, FfiCallingConvention, FfiErrorAbi,
    FfiOwnership, FfiSignature, FfiStringAbi, HirExpr, HirFunction, HirInitStep, HirLit, HirParam,
    HirProgram, HirStmt, HirType, Symbol,
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
    variadic: Option<HirType>,
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
    generic_type_constraints: Vec<Option<Box<TsType>>>,
    generic_type_defaults: Vec<Option<Box<TsType>>>,
    generic_param_patterns: Vec<GenericTypePattern>,
    generic_param_optional: Vec<bool>,
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

type EnumValues = HashMap<(Symbol, Symbol), HirLit>;
type EnumReverseValues = HashMap<Symbol, Vec<(f64, Symbol)>>;

fn enum_member_name(id: &swc_ecma_ast::TsEnumMemberId) -> Symbol {
    match id {
        swc_ecma_ast::TsEnumMemberId::Ident(id) => id.sym.to_string(),
        swc_ecma_ast::TsEnumMemberId::Str(value) => value.value.to_string_lossy().into_owned(),
    }
}

fn eval_enum_initializer(
    expr: &Expr,
    enum_name: &str,
    values: &EnumValues,
) -> Result<HirLit, String> {
    match expr {
        Expr::Lit(Lit::Num(value)) => Ok(HirLit::F64(value.value)),
        Expr::Lit(Lit::Str(value)) => Ok(HirLit::Str(value.value.to_string_lossy().into_owned())),
        Expr::Paren(value) => eval_enum_initializer(&value.expr, enum_name, values),
        Expr::Unary(unary) => {
            let HirLit::F64(value) = eval_enum_initializer(&unary.arg, enum_name, values)? else {
                return Err(format!(
                    "enum `{enum_name}` unary initializer requires a numeric operand"
                ));
            };
            match unary.op {
                UnaryOp::Plus => Ok(HirLit::F64(value)),
                UnaryOp::Minus => Ok(HirLit::F64(-value)),
                UnaryOp::Tilde => Ok(HirLit::F64((!(value as i32)) as f64)),
                other => Err(format!(
                    "enum `{enum_name}` has unsupported unary initializer {other:?}"
                )),
            }
        }
        Expr::Ident(member) => values
            .get(&(enum_name.to_string(), member.sym.to_string()))
            .cloned()
            .ok_or_else(|| {
                format!(
                    "enum `{enum_name}` initializer references unknown preceding member `{}`",
                    member.sym
                )
            }),
        Expr::Member(member) => {
            let Expr::Ident(target_enum) = member.obj.as_ref() else {
                return Err(format!(
                    "enum `{enum_name}` initializer has unsupported member reference"
                ));
            };
            let member_name = match &member.prop {
                MemberProp::Ident(member) => member.sym.to_string(),
                MemberProp::Computed(computed) => match computed.expr.as_ref() {
                    Expr::Lit(Lit::Str(member)) => member.value.to_string_lossy().into_owned(),
                    _ => {
                        return Err(format!(
                        "enum `{enum_name}` computed initializer member must be a string literal"
                    ))
                    }
                },
                _ => {
                    return Err(format!(
                        "enum `{enum_name}` has unsupported initializer member"
                    ))
                }
            };
            values
                .get(&(target_enum.sym.to_string(), member_name.clone()))
                .cloned()
                .ok_or_else(|| {
                    format!(
                        "enum `{enum_name}` initializer references unknown member `{}.{member_name}`",
                        target_enum.sym
                    )
                })
        }
        Expr::Bin(binary) => {
            let HirLit::F64(left) = eval_enum_initializer(&binary.left, enum_name, values)? else {
                return Err(format!(
                    "enum `{enum_name}` binary initializer requires numeric operands"
                ));
            };
            let HirLit::F64(right) = eval_enum_initializer(&binary.right, enum_name, values)?
            else {
                return Err(format!(
                    "enum `{enum_name}` binary initializer requires numeric operands"
                ));
            };
            let value = match binary.op {
                BinaryOp::Add => left + right,
                BinaryOp::Sub => left - right,
                BinaryOp::Mul => left * right,
                BinaryOp::Div => left / right,
                BinaryOp::Mod => left % right,
                BinaryOp::Exp => left.powf(right),
                BinaryOp::BitOr => ((left as i32) | (right as i32)) as f64,
                BinaryOp::BitXor => ((left as i32) ^ (right as i32)) as f64,
                BinaryOp::BitAnd => ((left as i32) & (right as i32)) as f64,
                BinaryOp::LShift => ((left as i32) << ((right as u32) & 31)) as f64,
                BinaryOp::RShift => ((left as i32) >> ((right as u32) & 31)) as f64,
                BinaryOp::ZeroFillRShift => ((left as u32) >> ((right as u32) & 31)) as f64,
                other => {
                    return Err(format!(
                        "enum `{enum_name}` has unsupported binary initializer {other:?}"
                    ))
                }
            };
            Ok(HirLit::F64(value))
        }
        other => Err(format!(
            "enum `{enum_name}` has unsupported initializer expression {other:?}"
        )),
    }
}

fn collect_enums(
    module: &Module,
) -> Result<(EnumValues, EnumReverseValues, HashMap<Symbol, HirType>), String> {
    let mut values = EnumValues::new();
    let mut reverse_values = EnumReverseValues::new();
    let mut types = HashMap::new();
    let mut enum_names = BTreeSet::new();
    for item in &module.body {
        let ModuleItem::Stmt(Stmt::Decl(Decl::TsEnum(declaration))) = item else {
            continue;
        };
        let name = declaration.id.sym.to_string();
        enum_names.insert(name.clone());
        let mut enum_type = types.get(&name).cloned();
        let mut next_number = Some(0.0);
        for member in &declaration.members {
            let member_name = enum_member_name(&member.id);
            let value = if let Some(initializer) = &member.init {
                eval_enum_initializer(initializer, &name, &values)?
            } else {
                HirLit::F64(next_number.ok_or_else(|| {
                    format!(
                        "enum `{name}` member `{member_name}` needs an initializer after a string member"
                    )
                })?)
            };
            let ty = match &value {
                HirLit::F64(number) => {
                    next_number = Some(number + 1.0);
                    let reverse = reverse_values.entry(name.clone()).or_default();
                    reverse.retain(|(existing, _)| existing != number);
                    reverse.push((*number, member_name.clone()));
                    HirType::F64
                }
                HirLit::Str(_) => {
                    next_number = None;
                    HirType::Str
                }
                _ => unreachable!("enum evaluator only produces number or string literals"),
            };
            if enum_type.as_ref().is_some_and(|existing| existing != &ty) {
                return Err(format!(
                    "enum `{name}` mixes numeric and string members, which has no single native layout"
                ));
            }
            enum_type.get_or_insert(ty);
            if values
                .insert((name.clone(), member_name.clone()), value)
                .is_some()
            {
                return Err(format!(
                    "enum `{name}` contains duplicate member `{member_name}`"
                ));
            }
        }
        if let Some(ty) = enum_type {
            types.insert(name, ty);
        }
    }
    for name in enum_names {
        if !types.contains_key(&name) {
            return Err(format!(
                "enum `{name}` must contain at least one member across its declarations"
            ));
        }
    }
    Ok((values, reverse_values, types))
}

pub fn normalize_top_level_destructuring(module: &Module) -> Result<Module, String> {
    fn binding_annotation(pattern: &Pat) -> Option<Box<swc_ecma_ast::TsTypeAnn>> {
        match pattern {
            Pat::Ident(binding) => binding.type_ann.clone(),
            Pat::Array(pattern) => pattern.type_ann.clone(),
            Pat::Object(pattern) => pattern.type_ann.clone(),
            _ => None,
        }
    }

    fn member(object: Expr, key: &PropName, span: swc_common::Span) -> Result<Expr, String> {
        let prop = match key {
            PropName::Ident(identifier) => MemberProp::Ident(identifier.clone()),
            PropName::Str(string) => MemberProp::Computed(ComputedPropName {
                span,
                expr: Box::new(Expr::Lit(Lit::Str(string.clone()))),
            }),
            PropName::Computed(computed) => match computed.expr.as_ref() {
                Expr::Lit(Lit::Str(_)) | Expr::Lit(Lit::Num(_)) => {
                    MemberProp::Computed(computed.clone())
                }
                _ => {
                    return Err(
                        "top-level destructuring keys must be static string/number literals".into(),
                    )
                }
            },
            _ => return Err("unsupported top-level destructuring property key".into()),
        };
        Ok(Expr::Member(MemberExpr {
            span,
            obj: Box::new(object),
            prop,
        }))
    }

    fn property_name(key: &PropName) -> Result<String, String> {
        match key {
            PropName::Ident(identifier) => Ok(identifier.sym.to_string()),
            PropName::Str(string) => Ok(string.value.to_string_lossy().into_owned()),
            PropName::Computed(computed) => match computed.expr.as_ref() {
                Expr::Lit(Lit::Str(string)) => Ok(string.value.to_string_lossy().into_owned()),
                _ => Err("top-level object rest requires static string keys".into()),
            },
            _ => Err("top-level object rest requires static string keys".into()),
        }
    }

    fn expand_pattern(
        pattern: &Pat,
        value: Expr,
        kind: swc_ecma_ast::VarDeclKind,
        span: swc_common::Span,
        counter: &mut usize,
        used: &mut HashSet<String>,
        out: &mut Vec<swc_ecma_ast::VarDecl>,
    ) -> Result<(), String> {
        if let Pat::Assign(assign) = pattern {
            return expand_pattern(
                &assign.left,
                Expr::Bin(swc_ecma_ast::BinExpr {
                    span,
                    op: BinaryOp::NullishCoalescing,
                    left: Box::new(value),
                    right: assign.right.clone(),
                }),
                kind,
                span,
                counter,
                used,
                out,
            );
        }
        if let Pat::Ident(binding) = pattern {
            out.push(swc_ecma_ast::VarDecl {
                span,
                ctxt: Default::default(),
                kind,
                declare: false,
                decls: vec![swc_ecma_ast::VarDeclarator {
                    span,
                    name: Pat::Ident(binding.clone()),
                    init: Some(Box::new(value)),
                    definite: false,
                }],
            });
            return Ok(());
        }

        let temporary = loop {
            let candidate = format!("__thaw_top_destructure_{}", *counter);
            *counter += 1;
            if used.insert(candidate.clone()) {
                break candidate;
            }
        };
        let temporary_ident = swc_ecma_ast::Ident::new_no_ctxt(temporary.clone().into(), span);
        out.push(swc_ecma_ast::VarDecl {
            span,
            ctxt: Default::default(),
            kind: swc_ecma_ast::VarDeclKind::Const,
            declare: false,
            decls: vec![swc_ecma_ast::VarDeclarator {
                span,
                name: Pat::Ident(swc_ecma_ast::BindingIdent {
                    id: temporary_ident.clone(),
                    type_ann: binding_annotation(pattern),
                }),
                init: Some(Box::new(value)),
                definite: false,
            }],
        });
        let temporary_expr = || Expr::Ident(temporary_ident.clone());

        match pattern {
            Pat::Object(object) => {
                let mut used_keys = Vec::new();
                for property in &object.props {
                    match property {
                        ObjectPatProp::Assign(property) => {
                            let key = PropName::Ident(swc_ecma_ast::IdentName::new(
                                property.key.id.sym.clone(),
                                property.key.id.span,
                            ));
                            used_keys.push(property.key.id.sym.to_string());
                            let mut value = member(temporary_expr(), &key, span)?;
                            if let Some(default) = &property.value {
                                value = Expr::Bin(swc_ecma_ast::BinExpr {
                                    span,
                                    op: BinaryOp::NullishCoalescing,
                                    left: Box::new(value),
                                    right: default.clone(),
                                });
                            }
                            expand_pattern(
                                &Pat::Ident(property.key.clone()),
                                value,
                                kind,
                                span,
                                counter,
                                used,
                                out,
                            )?;
                        }
                        ObjectPatProp::KeyValue(property) => {
                            used_keys.push(property_name(&property.key)?);
                            expand_pattern(
                                &property.value,
                                member(temporary_expr(), &property.key, span)?,
                                kind,
                                span,
                                counter,
                                used,
                                out,
                            )?;
                        }
                        ObjectPatProp::Rest(rest) => {
                            let mut args = vec![swc_ecma_ast::ExprOrSpread {
                                spread: None,
                                expr: Box::new(temporary_expr()),
                            }];
                            args.extend(used_keys.iter().map(|key| swc_ecma_ast::ExprOrSpread {
                                spread: None,
                                expr: Box::new(Expr::Lit(Lit::Str(swc_ecma_ast::Str {
                                    span,
                                    value: key.clone().into(),
                                    raw: None,
                                }))),
                            }));
                            expand_pattern(
                                &rest.arg,
                                Expr::Call(CallExpr {
                                    span,
                                    ctxt: Default::default(),
                                    callee: Callee::Expr(Box::new(Expr::Ident(
                                        swc_ecma_ast::Ident::new_no_ctxt(
                                            "__thaw_object_rest".into(),
                                            span,
                                        ),
                                    ))),
                                    args,
                                    type_args: None,
                                }),
                                kind,
                                span,
                                counter,
                                used,
                                out,
                            )?;
                        }
                    }
                }
            }
            Pat::Array(array) => {
                for (index, element) in array.elems.iter().enumerate() {
                    let Some(element) = element else { continue };
                    if let Pat::Rest(rest) = element {
                        let slice = Expr::Call(CallExpr {
                            span,
                            ctxt: Default::default(),
                            callee: Callee::Expr(Box::new(Expr::Member(MemberExpr {
                                span,
                                obj: Box::new(temporary_expr()),
                                prop: MemberProp::Ident(swc_ecma_ast::IdentName::new(
                                    "slice".into(),
                                    span,
                                )),
                            }))),
                            args: vec![swc_ecma_ast::ExprOrSpread {
                                spread: None,
                                expr: Box::new(Expr::Lit(Lit::Num(swc_ecma_ast::Number {
                                    span,
                                    value: index as f64,
                                    raw: None,
                                }))),
                            }],
                            type_args: None,
                        });
                        expand_pattern(&rest.arg, slice, kind, span, counter, used, out)?;
                        continue;
                    }
                    let value = Expr::Member(MemberExpr {
                        span,
                        obj: Box::new(temporary_expr()),
                        prop: MemberProp::Computed(ComputedPropName {
                            span,
                            expr: Box::new(Expr::Lit(Lit::Num(swc_ecma_ast::Number {
                                span,
                                value: index as f64,
                                raw: None,
                            }))),
                        }),
                    });
                    expand_pattern(element, value, kind, span, counter, used, out)?;
                }
            }
            _ => return Err("unsupported top-level destructuring pattern".into()),
        }
        Ok(())
    }

    fn expand_declaration(
        declaration: &VarDecl,
        exported: bool,
        counter: &mut usize,
        used: &mut HashSet<String>,
    ) -> Result<Vec<ModuleItem>, String> {
        let mut items = Vec::new();
        for declarator in &declaration.decls {
            let init = declarator
                .init
                .as_deref()
                .ok_or("top-level destructuring declarations need an initializer")?
                .clone();
            let mut declarations = Vec::new();
            expand_pattern(
                &declarator.name,
                init,
                declaration.kind,
                declaration.span,
                counter,
                used,
                &mut declarations,
            )?;
            for declaration in declarations {
                let is_temporary = declaration.decls.first().is_some_and(|declarator| {
                    matches!(&declarator.name, Pat::Ident(binding) if binding.id.sym.starts_with("__thaw_top_destructure_"))
                });
                if exported && !is_temporary {
                    items.push(ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(
                        swc_ecma_ast::ExportDecl {
                            span: declaration.span,
                            decl: Decl::Var(Box::new(declaration)),
                        },
                    )));
                } else {
                    items.push(ModuleItem::Stmt(Stmt::Decl(Decl::Var(Box::new(
                        declaration,
                    )))));
                }
            }
        }
        Ok(items)
    }

    let mut used = HashSet::new();
    for item in &module.body {
        match item {
            ModuleItem::Stmt(Stmt::Decl(declaration)) => {
                used.extend(declaration_names_for_normalization(declaration));
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => {
                used.extend(declaration_names_for_normalization(&export.decl));
            }
            _ => {}
        }
    }
    let mut counter = 0;
    let mut body = Vec::new();
    for item in &module.body {
        match item {
            ModuleItem::Stmt(Stmt::Decl(Decl::Var(declaration)))
                if declaration
                    .decls
                    .iter()
                    .any(|declarator| !matches!(declarator.name, Pat::Ident(_))) =>
            {
                body.extend(expand_declaration(
                    declaration.as_ref(),
                    false,
                    &mut counter,
                    &mut used,
                )?);
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) if matches!(&export.decl, Decl::Var(declaration) if declaration.decls.iter().any(|declarator| !matches!(declarator.name, Pat::Ident(_)))) =>
            {
                let Decl::Var(declaration) = &export.decl else {
                    unreachable!()
                };
                body.extend(expand_declaration(
                    declaration.as_ref(),
                    true,
                    &mut counter,
                    &mut used,
                )?);
            }
            _ => body.push(item.clone()),
        }
    }
    let mut normalized = module.clone();
    normalized.body = body;
    Ok(normalized)
}

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

fn class_constructor_symbol(name: &str) -> Symbol {
    format!("__thaw_class_{name}_constructor")
}

fn class_initializer_symbol(name: &str) -> Symbol {
    format!("__thaw_class_{name}_initialize")
}

fn class_method_symbol(class: &str, method: &str) -> Symbol {
    format!("__thaw_class_{class}_method_{method}")
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
        let mut fields = vec![(format!("__thaw_class_identity_{name}"), HirType::Bool)];
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
            fields.extend(base_fields.iter().skip(1).cloned());
        }
        for member in &declaration.class.body {
            let ClassMember::ClassProp(property) = member else {
                continue;
            };
            if property.is_static {
                continue;
            }
            if property.is_abstract || property.declare {
                return Err(format!(
                    "class `{name}` field `{}` cannot be abstract or ambient",
                    class_property_name(&property.key)?
                ));
            }
            let field_name = class_property_name(&property.key)?;
            if fields.iter().any(|(existing, _)| existing == &field_name) {
                return Err(format!(
                    "class `{name}` field `{field_name}` collides with an inherited or local field"
                ));
            }
            let annotation = property.type_ann.as_ref().ok_or_else(|| {
                format!("class `{name}` field `{field_name}` needs a type annotation")
            })?;
            fields.push((
                field_name,
                lower_ts_type(&annotation.type_ann, interfaces, generic_interfaces)?,
            ));
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
            fields.push((
                field_name,
                lower_ts_type(&annotation.type_ann, interfaces, generic_interfaces)?,
            ));
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
        if declaration.class.is_abstract || declaration.class.type_params.is_some() {
            return Err(format!(
                "class `{name}` currently requires a concrete, non-generic class"
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
    Ok(classes)
}

pub fn lower_module(module: &Module) -> Result<HirProgram, String> {
    let normalized = normalize_top_level_destructuring(module)?;
    lower_normalized_module(&normalized)
}

fn lower_normalized_module(module: &Module) -> Result<HirProgram, String> {
    let (mut interfaces, generic_interfaces) = resolve_interfaces(module)?;
    let (enum_values, enum_reverse_values, enum_types) = collect_enums(module)?;
    for (name, ty) in enum_types {
        if interfaces.insert(name.clone(), ty).is_some() {
            return Err(format!(
                "enum `{name}` conflicts with an interface declaration"
            ));
        }
    }
    let class_decls = collect_native_classes(module, &mut interfaces, &generic_interfaces)?;

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
                signatures.insert(
                    name,
                    FnSignature {
                        params,
                        variadic,
                        ret,
                        is_async: func.is_async,
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
                        ret: instance_type.clone(),
                        is_async: false,
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
                        ret: instance_type.clone(),
                        is_async: false,
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
                            ret,
                            is_async: method.function.is_async,
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
            let constructor = signatures
                .get_mut(&class_constructor_symbol(derived_name))
                .expect("derived constructor signature");
            if constructor.params != base_params {
                constructor.params = base_params.clone();
                changed = true;
            }
            let mut initializer_params = vec![interfaces[derived_name].clone()];
            initializer_params.extend(base_params);
            signatures
                .get_mut(&class_initializer_symbol(derived_name))
                .expect("derived initializer signature")
                .params = initializer_params;
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
                if !seen.insert(key) {
                    continue;
                }
                let base_symbol = class_member_symbol(&current_name, method)?;
                let derived_symbol = class_member_symbol(&derived_name, method)?;
                let mut signature = signatures[&base_symbol].clone();
                if !method.is_static {
                    signature.params[0] = derived_type.clone();
                }
                signatures.insert(derived_symbol.clone(), signature.clone());
                let params = signature
                    .params
                    .iter()
                    .enumerate()
                    .map(|(index, ty)| HirParam {
                        name: if !method.is_static && index == 0 {
                            "__thaw_this".into()
                        } else {
                            format!("__thaw_inherited_arg_{index}")
                        },
                        ty: ty.clone(),
                    })
                    .collect::<Vec<_>>();
                let call = HirExpr::Call(
                    Box::new(HirExpr::Var(base_symbol)),
                    params
                        .iter()
                        .map(|parameter| HirExpr::Var(parameter.name.clone()))
                        .collect(),
                );
                let body = if signature.ret == HirType::Void {
                    vec![HirStmt::Expr(call), HirStmt::Return(None)]
                } else {
                    vec![HirStmt::Return(Some(call))]
                };
                inherited_class_functions.push(HirFunction {
                    name: derived_symbol,
                    params,
                    ret: signature.ret,
                    is_async: signature.is_async,
                    body,
                });
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
            if property.value.is_none() {
                return Err(format!(
                    "class `{class_name}` static field `{field}` needs an initializer"
                ));
            }
            let symbol = class_static_field_symbol(class_name, &field);
            let ty = lower_ts_type(&annotation.type_ann, &interfaces, &generic_interfaces)?;
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
                        ret: ty.clone(),
                        is_async: false,
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
                            ret: ty.clone(),
                            is_async: false,
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

    let functions = fn_decls
        .into_iter()
        .map(|fn_decl| {
            lower_fn_decl(
                fn_decl,
                &signatures,
                &interfaces,
                &generic_interfaces,
                &enum_values,
                &enum_reverse_values,
                &global_types,
                &immutable_globals,
                None,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut specialized = functions
        .into_iter()
        .filter(|function| signatures[&function.name].generic_type_params.is_empty())
        .collect::<Vec<_>>();
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
                    let init = lowerer.lower_expr(
                        property
                            .value
                            .as_deref()
                            .expect("static fields were validated above"),
                    )?;
                    let init = lowerer.coerce_to_declared(&expected, init)?;
                    steps.push(HirInitStep::StoreGlobal(symbol, init));
                }
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
            let init = lowerer.lower_expr(
                property
                    .value
                    .as_deref()
                    .expect("static fields were validated above"),
            )?;
            let init = lowerer.coerce_to_declared(&ty, init)?;
            globals.push(crate::HirGlobal {
                name: symbol.clone(),
                ty,
                init,
                mutable: !immutable_globals.contains(&symbol),
            });
        }
    }
    Ok(globals)
}

fn hir_type_contains_dynamic(ty: &HirType) -> bool {
    match ty {
        HirType::Dynamic => true,
        HirType::Promise(inner)
        | HirType::Array(inner)
        | HirType::Optional(inner)
        | HirType::Nullable(inner)
        | HirType::Nullish(inner) => hir_type_contains_dynamic(inner),
        HirType::Tuple(elements) | HirType::Union(elements) => {
            elements.iter().any(hir_type_contains_dynamic)
        }
        HirType::Object(fields) => fields
            .iter()
            .any(|(_, field)| hir_type_contains_dynamic(field)),
        HirType::Function(parameters, result) => {
            parameters.iter().any(hir_type_contains_dynamic) || hir_type_contains_dynamic(result)
        }
        _ => false,
    }
}

fn supports_ffi_variadic_element(ty: &HirType) -> bool {
    match ty {
        HirType::F64 | HirType::Bool | HirType::Str | HirType::JsValue => true,
        HirType::Array(element) => matches!(
            element.as_ref(),
            HirType::F64 | HirType::Bool | HirType::Str | HirType::JsValue
        ),
        HirType::Optional(payload) | HirType::Nullable(payload) | HirType::Nullish(payload) => {
            supports_ffi_variadic_element(payload)
        }
        HirType::Object(fields) => fields
            .iter()
            .all(|(_, field)| supports_ffi_variadic_element(field)),
        _ => false,
    }
}

fn validate_generic_function(fn_decl: &FnDecl) -> Result<Vec<Symbol>, String> {
    let Some(type_params) = &fn_decl.function.type_params else {
        return Ok(Vec::new());
    };
    let name = fn_decl.ident.sym.as_str();
    validate_trailing_type_parameter_defaults("generic function", name, type_params)?;
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

fn validate_trailing_type_parameter_defaults(
    declaration_kind: &str,
    name: &str,
    parameters: &swc_ecma_ast::TsTypeParamDecl,
) -> Result<(), String> {
    let mut saw_default = false;
    for parameter in &parameters.params {
        if parameter.default.is_some() {
            saw_default = true;
        } else if saw_default {
            return Err(format!(
                "{declaration_kind} `{name}` has required type parameter `{}` after an optional type parameter",
                parameter.name.sym
            ));
        }
    }
    Ok(())
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

fn generic_pattern_contains_variable(pattern: &GenericTypePattern, variable: &str) -> bool {
    match pattern {
        GenericTypePattern::Variable(name) => name == variable,
        GenericTypePattern::Array(inner) | GenericTypePattern::Promise(inner) => {
            generic_pattern_contains_variable(inner, variable)
        }
        GenericTypePattern::Object(fields) => fields
            .iter()
            .any(|(_, field)| generic_pattern_contains_variable(field, variable)),
        GenericTypePattern::Concrete(_) => false,
    }
}

fn instantiate_generic_pattern(
    pattern: &GenericTypePattern,
    substitution: &HashMap<Symbol, HirType>,
) -> Result<HirType, String> {
    match pattern {
        GenericTypePattern::Variable(name) => substitution
            .get(name)
            .cloned()
            .ok_or_else(|| format!("missing concrete type for `{name}`")),
        GenericTypePattern::Concrete(ty) => Ok(ty.clone()),
        GenericTypePattern::Array(inner) => Ok(HirType::Array(Box::new(
            instantiate_generic_pattern(inner, substitution)?,
        ))),
        GenericTypePattern::Promise(inner) => Ok(HirType::Promise(Box::new(
            instantiate_generic_pattern(inner, substitution)?,
        ))),
        GenericTypePattern::Object(fields) => Ok(HirType::Object(
            fields
                .iter()
                .map(|(name, ty)| {
                    Ok((name.clone(), instantiate_generic_pattern(ty, substitution)?))
                })
                .collect::<Result<Vec<_>, String>>()?,
        )),
    }
}

fn specialized_generic_function_name(
    name: &str,
    concrete_params: &[HirType],
    signature: &FnSignature,
    generic_types: &[HirType],
) -> Symbol {
    let base = specialized_generic_name(name, concrete_params);
    let hidden = signature
        .generic_type_params
        .iter()
        .zip(generic_types)
        .filter_map(|(parameter, ty)| {
            (!signature
                .generic_param_patterns
                .iter()
                .any(|pattern| generic_pattern_contains_variable(pattern, parameter)))
            .then_some(ty.clone())
        })
        .collect::<Vec<_>>();
    if hidden.is_empty() {
        base
    } else {
        format!("{base}__generic{}", specialized_generic_name("", &hidden))
    }
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
            if let Some(interface) = generic_interfaces.interfaces.get(name) {
                if in_progress.iter().any(|active| active == name) {
                    return Err(format!("generic interface `{name}` is self-referential"));
                }
                let parameters = &interface
                    .type_params
                    .as_ref()
                    .expect("generic interface type parameters")
                    .params;
                let arguments = reference
                    .type_params
                    .as_ref()
                    .map(|params| params.params.as_slice())
                    .unwrap_or_default();
                let required = parameters
                    .iter()
                    .take_while(|parameter| parameter.default.is_none())
                    .count();
                if arguments.len() < required || arguments.len() > parameters.len() {
                    return Err(format!(
                        "generic interface `{name}` expects {required}..={} type argument(s), got {}",
                        parameters.len(),
                        arguments.len()
                    ));
                }
                let mut nested_substitutions = HashMap::new();
                for (index, parameter) in parameters.iter().enumerate() {
                    let argument = arguments
                        .get(index)
                        .map(|argument| argument.as_ref())
                        .or_else(|| parameter.default.as_ref().map(|default| default.as_ref()))
                        .expect("validated generic interface arity requires a default");
                    let pattern = generic_type_pattern(
                        argument,
                        if index < arguments.len() {
                            substitutions
                        } else {
                            &nested_substitutions
                        },
                        interfaces,
                        generic_interfaces,
                        in_progress,
                    )?;
                    nested_substitutions.insert(parameter.name.sym.to_string(), pattern);
                }
                in_progress.push(name.to_string());
                let mut fields = Vec::new();
                for base in &interface.extends {
                    let Expr::Ident(base_ident) = base.expr.as_ref() else {
                        return Err(format!(
                            "generic interface `{name}` has an unsupported `extends` target"
                        ));
                    };
                    let base_reference = TsType::TsTypeRef(swc_ecma_ast::TsTypeRef {
                        span: base.span,
                        type_name: swc_ecma_ast::TsEntityName::Ident(base_ident.clone()),
                        type_params: base.type_args.clone(),
                    });
                    let base_pattern = generic_type_pattern(
                        &base_reference,
                        &nested_substitutions,
                        interfaces,
                        generic_interfaces,
                        in_progress,
                    )?;
                    let base_fields = match base_pattern {
                        GenericTypePattern::Object(fields) => fields,
                        GenericTypePattern::Concrete(HirType::Object(fields)) => fields
                            .into_iter()
                            .map(|(field, ty)| (field, GenericTypePattern::Concrete(ty)))
                            .collect(),
                        _ => {
                            return Err(format!(
                            "generic interface `{name}` can only extend an object-shaped interface"
                        ))
                        }
                    };
                    for (field_name, field_ty) in base_fields {
                        if fields.iter().any(|(existing, _)| existing == &field_name) {
                            return Err(format!(
                                "generic interface `{name}` inherits duplicate field `{field_name}`"
                            ));
                        }
                        fields.push((field_name, field_ty));
                    }
                }
                let own_fields = interface
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
                    .collect::<Result<Vec<_>, String>>()?;
                for (field_name, field_ty) in own_fields {
                    if fields.iter().any(|(existing, _)| existing == &field_name) {
                        return Err(format!(
                            "generic interface `{name}` declares inherited field `{field_name}` again"
                        ));
                    }
                    fields.push((field_name, field_ty));
                }
                in_progress.pop();
                return Ok(GenericTypePattern::Object(fields));
            }
            if let Some(alias) = generic_interfaces.aliases.get(name) {
                if in_progress.iter().any(|active| active == name) {
                    return Err(format!("generic type alias `{name}` is self-referential"));
                }
                let parameters = &alias
                    .type_params
                    .as_ref()
                    .expect("generic alias type parameters")
                    .params;
                let arguments = reference
                    .type_params
                    .as_ref()
                    .map(|parameters| parameters.params.as_slice())
                    .unwrap_or_default();
                let required = parameters
                    .iter()
                    .take_while(|parameter| parameter.default.is_none())
                    .count();
                if arguments.len() < required || arguments.len() > parameters.len() {
                    return Err(format!(
                        "generic type alias `{name}` expects {required}..={} type argument(s), got {}",
                        parameters.len(),
                        arguments.len()
                    ));
                }
                let mut nested = HashMap::new();
                for (index, parameter) in parameters.iter().enumerate() {
                    let argument = arguments
                        .get(index)
                        .map(|argument| argument.as_ref())
                        .or_else(|| parameter.default.as_ref().map(|default| default.as_ref()))
                        .expect("validated generic alias arity requires a default");
                    let pattern = generic_type_pattern(
                        argument,
                        if index < arguments.len() {
                            substitutions
                        } else {
                            &nested
                        },
                        interfaces,
                        generic_interfaces,
                        in_progress,
                    )?;
                    nested.insert(parameter.name.sym.to_string(), pattern);
                }
                in_progress.push(name.to_string());
                let result = generic_type_pattern(
                    &alias.type_ann,
                    &nested,
                    interfaces,
                    generic_interfaces,
                    in_progress,
                );
                in_progress.pop();
                return result;
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
    interfaces: &HashMap<Symbol, HirType>,
    generic_interfaces: &GenericInterfaces,
) -> Result<Vec<HirType>, String> {
    let mut inferred = HashMap::new();
    for (pattern, actual) in signature.generic_param_patterns.iter().zip(actual_params) {
        match_generic_pattern(pattern, actual, &mut inferred)?;
    }
    let mut types = Vec::with_capacity(signature.generic_type_params.len());
    let mut substitution = HashMap::new();
    for ((name, default), index) in signature
        .generic_type_params
        .iter()
        .zip(&signature.generic_type_defaults)
        .zip(0..)
    {
        let concrete = if let Some(inferred) = inferred.get(name) {
            inferred.clone()
        } else if let Some(default) = default {
            resolve_ts_type_with_substitution(
                default,
                &substitution,
                interfaces,
                generic_interfaces,
                &mut Vec::new(),
            )?
        } else {
            return Err(format!(
                "cannot infer generic type parameter `{name}` from this call (parameter {})",
                index + 1
            ));
        };
        substitution.insert(name.clone(), concrete.clone());
        types.push(concrete);
    }
    for ((name, actual), constraint) in signature
        .generic_type_params
        .iter()
        .zip(&types)
        .zip(&signature.generic_type_constraints)
    {
        let Some(constraint) = constraint else {
            continue;
        };
        let constraint = resolve_ts_type_with_substitution(
            constraint,
            &substitution,
            interfaces,
            generic_interfaces,
            &mut Vec::new(),
        )?;
        if !type_satisfies_constraint(actual, &constraint) {
            return Err(format!(
                "inferred type {actual:?} does not satisfy constraint {constraint:?} for `{name}`"
            ));
        }
    }
    Ok(types)
}

fn resolve_explicit_generic_type_tuple(
    signature: &FnSignature,
    arguments: &[Box<TsType>],
    actual_params: &[HirType],
    interfaces: &HashMap<Symbol, HirType>,
    generic_interfaces: &GenericInterfaces,
) -> Result<Vec<HirType>, String> {
    let required = signature
        .generic_type_defaults
        .iter()
        .filter(|default| default.is_none())
        .count();
    if arguments.len() < required || arguments.len() > signature.generic_type_params.len() {
        let expected = if required == signature.generic_type_params.len() {
            required.to_string()
        } else {
            format!("{required}..={}", signature.generic_type_params.len())
        };
        return Err(format!(
            "expects {expected} explicit type argument(s), got {}",
            arguments.len()
        ));
    }

    let mut types = Vec::with_capacity(signature.generic_type_params.len());
    let mut substitution = HashMap::new();
    for (index, name) in signature.generic_type_params.iter().enumerate() {
        let concrete = if let Some(argument) = arguments.get(index) {
            lower_ts_type(argument, interfaces, generic_interfaces)?
        } else {
            resolve_ts_type_with_substitution(
                signature.generic_type_defaults[index]
                    .as_ref()
                    .expect("validated explicit generic arity requires a default"),
                &substitution,
                interfaces,
                generic_interfaces,
                &mut Vec::new(),
            )?
        };
        substitution.insert(name.clone(), concrete.clone());
        types.push(concrete);
    }

    for ((name, actual), constraint) in signature
        .generic_type_params
        .iter()
        .zip(&types)
        .zip(&signature.generic_type_constraints)
    {
        let Some(constraint) = constraint else {
            continue;
        };
        let constraint = resolve_ts_type_with_substitution(
            constraint,
            &substitution,
            interfaces,
            generic_interfaces,
            &mut Vec::new(),
        )?;
        if !type_satisfies_constraint(actual, &constraint) {
            return Err(format!(
                "explicit type {actual:?} does not satisfy constraint {constraint:?} for `{name}`"
            ));
        }
    }

    let mut inferred = HashMap::new();
    for (pattern, actual) in signature.generic_param_patterns.iter().zip(actual_params) {
        match_generic_pattern(pattern, actual, &mut inferred)?;
    }
    for (name, inferred) in inferred {
        let explicit = &substitution[&name];
        if &inferred != explicit {
            return Err(format!(
                "argument infers {name} as {inferred:?}, but explicit type is {explicit:?}"
            ));
        }
    }
    Ok(types)
}

#[allow(clippy::too_many_arguments)]
fn lower_generic_instance(
    fn_decl: &FnDecl,
    types: &[HirType],
    signatures: &HashMap<Symbol, FnSignature>,
    interfaces: &HashMap<Symbol, HirType>,
    generic_interfaces: &GenericInterfaces,
    enum_values: &EnumValues,
    enum_reverse_values: &EnumReverseValues,
    global_types: &HashMap<Symbol, HirType>,
    immutable_globals: &HashSet<Symbol>,
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
        enum_values,
        enum_reverse_values,
        ret.clone(),
        call_constraints,
    );
    seed_global_scope(&mut lowerer, global_types, immutable_globals);
    for param in &params {
        lowerer.immutable_bindings.remove(&param.name);
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
        name: specialized_generic_function_name(
            &base_name,
            &params
                .iter()
                .map(|param| param.ty.clone())
                .collect::<Vec<_>>(),
            signature,
            types,
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
/// The eager declaration pass also consults the generic map, allowing a
/// non-generic interface to extend a concrete generic base or contain a
/// concretely instantiated generic interface field, including forward
/// references to the generic declaration and its non-generic bases.
#[derive(Default)]
struct GenericInterfaces<'a> {
    interfaces: HashMap<Symbol, &'a TsInterfaceDecl>,
    aliases: HashMap<Symbol, &'a swc_ecma_ast::TsTypeAliasDecl>,
    function_aliases: HashMap<Symbol, &'a swc_ecma_ast::TsTypeAliasDecl>,
    function_alias_chains: HashMap<Symbol, Symbol>,
    function_interfaces: HashMap<Symbol, &'a TsInterfaceDecl>,
    function_interface_chains: HashMap<Symbol, Symbol>,
}

impl GenericInterfaces<'_> {
    fn new() -> Self {
        Self::default()
    }
}

fn strip_parenthesized_ts_type(mut ty: &TsType) -> &TsType {
    while let TsType::TsParenthesizedType(parenthesized) = ty {
        ty = &parenthesized.type_ann;
    }
    ty
}

fn function_expression_as_arrow(
    expression: &swc_ecma_ast::FnExpr,
) -> Result<swc_ecma_ast::ArrowExpr, String> {
    let function = expression.function.as_ref();
    if function.this_param.is_some()
        || !function.decorators.is_empty()
        || function
            .params
            .iter()
            .any(|parameter| !parameter.decorators.is_empty())
    {
        return Err(
            "function expressions with this parameters or decorators are not supported".into(),
        );
    }
    let body = function
        .body
        .as_ref()
        .ok_or("function expression needs a body")?;
    Ok(swc_ecma_ast::ArrowExpr {
        span: function.span,
        ctxt: function.ctxt,
        params: function
            .params
            .iter()
            .map(|parameter| parameter.pat.clone())
            .collect(),
        body: Box::new(ArrowFunctionBody::FunctionBody(body.clone())),
        is_async: function.is_async,
        is_generator: function.is_generator,
        type_params: function.type_params.clone(),
        return_type: function.return_type.clone(),
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InferredGenericReturn {
    Parameter(usize),
    Keyword(TsKeywordTypeKind),
}

fn expression_is_definitely_string(expr: &Expr) -> bool {
    match expr {
        Expr::Lit(Lit::Str(_)) | Expr::Tpl(_) => true,
        Expr::Paren(parenthesized) => expression_is_definitely_string(&parenthesized.expr),
        Expr::TsAs(assertion) => expression_is_definitely_string(&assertion.expr),
        Expr::TsTypeAssertion(assertion) => expression_is_definitely_string(&assertion.expr),
        Expr::Call(call) => matches!(
            &call.callee,
            Callee::Expr(callee)
                if matches!(callee.as_ref(), Expr::Ident(identifier) if identifier.sym == *"String")
        ),
        _ => false,
    }
}

fn inferred_generic_return(expr: &Expr, params: &[Pat]) -> Option<InferredGenericReturn> {
    let returned = match expr {
        Expr::Paren(parenthesized) => parenthesized.expr.as_ref(),
        Expr::TsAs(assertion) => assertion.expr.as_ref(),
        Expr::TsTypeAssertion(assertion) => assertion.expr.as_ref(),
        expression => expression,
    };
    if let Expr::Cond(conditional) = returned {
        let consequent = inferred_generic_return(&conditional.cons, params)?;
        let alternate = inferred_generic_return(&conditional.alt, params)?;
        return (consequent == alternate).then_some(consequent);
    }
    if let Expr::Bin(binary) = returned {
        if matches!(
            binary.op,
            BinaryOp::LogicalAnd | BinaryOp::LogicalOr | BinaryOp::NullishCoalescing
        ) {
            let left = inferred_generic_return(&binary.left, params)?;
            let right = inferred_generic_return(&binary.right, params)?;
            return (left == right).then_some(left);
        }
    }
    let keyword = match returned {
        Expr::Lit(Lit::Num(_)) => Some(TsKeywordTypeKind::TsNumberKeyword),
        Expr::Lit(Lit::Str(_)) => Some(TsKeywordTypeKind::TsStringKeyword),
        Expr::Lit(Lit::Bool(_)) => Some(TsKeywordTypeKind::TsBooleanKeyword),
        Expr::Lit(Lit::Null(_)) => Some(TsKeywordTypeKind::TsNullKeyword),
        Expr::Tpl(_) => Some(TsKeywordTypeKind::TsStringKeyword),
        Expr::Unary(unary) => match unary.op {
            UnaryOp::Bang => Some(TsKeywordTypeKind::TsBooleanKeyword),
            UnaryOp::TypeOf => Some(TsKeywordTypeKind::TsStringKeyword),
            UnaryOp::Void => Some(TsKeywordTypeKind::TsUndefinedKeyword),
            UnaryOp::Plus | UnaryOp::Minus | UnaryOp::Tilde => {
                Some(TsKeywordTypeKind::TsNumberKeyword)
            }
            _ => None,
        },
        Expr::Bin(binary)
            if matches!(
                binary.op,
                BinaryOp::EqEq
                    | BinaryOp::NotEq
                    | BinaryOp::EqEqEq
                    | BinaryOp::NotEqEq
                    | BinaryOp::Lt
                    | BinaryOp::LtEq
                    | BinaryOp::Gt
                    | BinaryOp::GtEq
                    | BinaryOp::In
                    | BinaryOp::InstanceOf
            ) =>
        {
            Some(TsKeywordTypeKind::TsBooleanKeyword)
        }
        Expr::Bin(binary)
            if matches!(
                binary.op,
                BinaryOp::Sub
                    | BinaryOp::Mul
                    | BinaryOp::Div
                    | BinaryOp::Mod
                    | BinaryOp::Exp
                    | BinaryOp::BitOr
                    | BinaryOp::BitXor
                    | BinaryOp::BitAnd
                    | BinaryOp::LShift
                    | BinaryOp::RShift
                    | BinaryOp::ZeroFillRShift
            ) =>
        {
            Some(TsKeywordTypeKind::TsNumberKeyword)
        }
        Expr::Bin(binary)
            if binary.op == BinaryOp::Add
                && (expression_is_definitely_string(&binary.left)
                    || expression_is_definitely_string(&binary.right)) =>
        {
            Some(TsKeywordTypeKind::TsStringKeyword)
        }
        Expr::Call(call) => match &call.callee {
            Callee::Expr(callee) => match callee.as_ref() {
                Expr::Ident(identifier) if identifier.sym == *"String" => {
                    Some(TsKeywordTypeKind::TsStringKeyword)
                }
                Expr::Ident(identifier) if identifier.sym == *"Number" => {
                    Some(TsKeywordTypeKind::TsNumberKeyword)
                }
                Expr::Ident(identifier) if identifier.sym == *"Boolean" => {
                    Some(TsKeywordTypeKind::TsBooleanKeyword)
                }
                _ => None,
            },
            _ => None,
        },
        _ => None,
    };
    if let Some(keyword) = keyword {
        return Some(InferredGenericReturn::Keyword(keyword));
    }
    let Expr::Ident(returned) = returned else {
        return None;
    };
    let parameter = params.iter().enumerate().find_map(|(index, parameter)| {
        let Pat::Ident(binding) = parameter else {
            return None;
        };
        if binding.id.sym != returned.sym {
            return None;
        }
        let keyword = binding.type_ann.as_ref().and_then(|annotation| {
            let TsType::TsKeywordType(keyword) = strip_parenthesized_ts_type(&annotation.type_ann)
            else {
                return None;
            };
            Some(keyword.kind)
        });
        Some(
            keyword
                .map(InferredGenericReturn::Keyword)
                .unwrap_or(InferredGenericReturn::Parameter(index)),
        )
    });
    parameter.or_else(|| {
        (returned.sym == *"undefined").then_some(InferredGenericReturn::Keyword(
            TsKeywordTypeKind::TsUndefinedKeyword,
        ))
    })
}

fn collect_generic_return_parameters(
    statement: &Stmt,
    params: &[Pat],
    returned: &mut Vec<InferredGenericReturn>,
) -> bool {
    match statement {
        Stmt::Return(return_statement) => return_statement
            .arg
            .as_deref()
            .and_then(|expr| inferred_generic_return(expr, params))
            .map(|index| returned.push(index))
            .is_some(),
        Stmt::If(if_statement) => {
            collect_generic_return_parameters(&if_statement.cons, params, returned)
                && if_statement.alt.as_deref().is_none_or(|alternate| {
                    collect_generic_return_parameters(alternate, params, returned)
                })
        }
        Stmt::Block(block) => block
            .stmts
            .iter()
            .all(|statement| collect_generic_return_parameters(statement, params, returned)),
        Stmt::Empty(_) => true,
        _ => false,
    }
}

fn inferred_generic_arrow_return_type(arrow: &swc_ecma_ast::ArrowExpr) -> Option<Box<TsType>> {
    let inferred = match arrow.body.as_ref() {
        ArrowFunctionBody::Expr(expression) => inferred_generic_return(expression, &arrow.params)?,
        ArrowFunctionBody::FunctionBody(body) => {
            let mut returned = Vec::new();
            if !body.stmts.iter().all(|statement| {
                collect_generic_return_parameters(statement, &arrow.params, &mut returned)
            }) {
                return None;
            }
            let first = *returned.first()?;
            returned
                .iter()
                .all(|index| *index == first)
                .then_some(first)?
        }
    };
    match inferred {
        InferredGenericReturn::Parameter(index) => {
            let Pat::Ident(binding) = &arrow.params[index] else {
                return None;
            };
            binding
                .type_ann
                .as_ref()
                .map(|annotation| annotation.type_ann.clone())
        }
        InferredGenericReturn::Keyword(kind) => Some(Box::new(TsType::TsKeywordType(
            swc_ecma_ast::TsKeywordType {
                span: swc_common::DUMMY_SP,
                kind,
            },
        ))),
    }
}

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
    let mut aliases = HashMap::new();
    let mut generic = GenericInterfaces::new();
    for item in &module.body {
        match item {
            ModuleItem::Stmt(Stmt::Decl(Decl::TsInterface(iface))) => {
                let name = iface.id.sym.to_string();
                if let Some(parameters) = &iface.type_params {
                    validate_trailing_type_parameter_defaults(
                        "generic interface",
                        &name,
                        parameters,
                    )?;
                    generic.interfaces.insert(name, iface.as_ref());
                } else if matches!(
                    iface.body.body.as_slice(),
                    [TsTypeElement::TsCallSignatureDecl(call)] if call.type_params.is_some()
                ) {
                    generic.function_interfaces.insert(name, iface.as_ref());
                } else {
                    raw.insert(name, iface.as_ref());
                }
            }
            ModuleItem::Stmt(Stmt::Decl(Decl::TsTypeAlias(alias))) => {
                let name = alias.id.sym.to_string();
                if matches!(
                    strip_parenthesized_ts_type(&alias.type_ann),
                    TsType::TsFnOrConstructorType(TsFnOrConstructorType::TsFnType(function))
                        if function.type_params.is_some()
                ) || matches!(
                    strip_parenthesized_ts_type(&alias.type_ann),
                    TsType::TsTypeLit(literal)
                        if matches!(literal.members.as_slice(),
                            [TsTypeElement::TsCallSignatureDecl(call)]
                                if call.type_params.is_some())
                ) {
                    generic.function_aliases.insert(name, alias.as_ref());
                } else if let Some(parameters) = &alias.type_params {
                    validate_trailing_type_parameter_defaults(
                        "generic type alias",
                        &name,
                        parameters,
                    )?;
                    generic.aliases.insert(name, alias.as_ref());
                } else {
                    aliases.insert(name, alias.as_ref());
                }
            }
            _ => {}
        }
    }

    loop {
        let callable_aliases = aliases
            .iter()
            .filter_map(|(name, alias)| {
                let TsType::TsTypeRef(reference) = strip_parenthesized_ts_type(&alias.type_ann)
                else {
                    return None;
                };
                let swc_ecma_ast::TsEntityName::Ident(target) = &reference.type_name else {
                    return None;
                };
                if reference.type_params.is_none()
                    && (generic.function_aliases.contains_key(target.sym.as_ref())
                        || generic
                            .function_interfaces
                            .contains_key(target.sym.as_ref())
                        || generic
                            .function_alias_chains
                            .contains_key(target.sym.as_ref())
                        || generic
                            .function_interface_chains
                            .contains_key(target.sym.as_ref()))
                {
                    Some((name.clone(), target.sym.to_string()))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        let callable_interfaces = raw
            .iter()
            .filter_map(|(name, interface)| {
                let [base] = interface.extends.as_slice() else {
                    return None;
                };
                if !interface.body.body.is_empty() || base.type_args.is_some() {
                    return None;
                }
                let Expr::Ident(target) = base.expr.as_ref() else {
                    return None;
                };
                (generic.function_aliases.contains_key(target.sym.as_ref())
                    || generic
                        .function_interfaces
                        .contains_key(target.sym.as_ref())
                    || generic
                        .function_alias_chains
                        .contains_key(target.sym.as_ref())
                    || generic
                        .function_interface_chains
                        .contains_key(target.sym.as_ref()))
                .then(|| (name.clone(), target.sym.to_string()))
            })
            .collect::<Vec<_>>();
        if callable_aliases.is_empty() && callable_interfaces.is_empty() {
            break;
        }
        for (name, target) in callable_aliases {
            aliases.remove(&name);
            generic.function_alias_chains.insert(name, target);
        }
        for (name, target) in callable_interfaces {
            raw.remove(&name);
            generic.function_interface_chains.insert(name, target);
        }
    }

    if let Some(name) = raw.keys().find(|name| aliases.contains_key(*name)) {
        return Err(format!(
            "type alias `{name}` conflicts with an interface declaration"
        ));
    }
    if let Some(name) = generic.aliases.keys().find(|name| {
        raw.contains_key(*name)
            || aliases.contains_key(*name)
            || generic.interfaces.contains_key(*name)
    }) {
        return Err(format!(
            "generic type alias `{name}` conflicts with another type declaration"
        ));
    }
    if let Some(name) = aliases
        .keys()
        .find(|name| generic.interfaces.contains_key(*name))
    {
        return Err(format!(
            "type alias `{name}` conflicts with a generic interface declaration"
        ));
    }
    if let Some(name) = generic.function_aliases.keys().find(|name| {
        raw.contains_key(*name)
            || aliases.contains_key(*name)
            || generic.interfaces.contains_key(*name)
            || generic.aliases.contains_key(*name)
            || generic.function_alias_chains.contains_key(*name)
            || generic.function_interface_chains.contains_key(*name)
    }) {
        return Err(format!(
            "generic function type alias `{name}` conflicts with another type declaration"
        ));
    }
    if let Some(name) = generic.function_interfaces.keys().find(|name| {
        raw.contains_key(*name)
            || aliases.contains_key(*name)
            || generic.interfaces.contains_key(*name)
            || generic.aliases.contains_key(*name)
            || generic.function_aliases.contains_key(*name)
            || generic.function_alias_chains.contains_key(*name)
            || generic.function_interface_chains.contains_key(*name)
    }) {
        return Err(format!(
            "generic callable interface `{name}` conflicts with another type declaration"
        ));
    }
    if let Some(name) = generic.function_alias_chains.keys().find(|name| {
        raw.contains_key(*name)
            || generic.interfaces.contains_key(*name)
            || generic.aliases.contains_key(*name)
            || generic.function_interface_chains.contains_key(*name)
    }) {
        return Err(format!(
            "callable type alias `{name}` conflicts with another type declaration"
        ));
    }
    if let Some(name) = generic.function_interface_chains.keys().find(|name| {
        aliases.contains_key(*name)
            || generic.interfaces.contains_key(*name)
            || generic.aliases.contains_key(*name)
    }) {
        return Err(format!(
            "callable interface `{name}` conflicts with another type declaration"
        ));
    }

    let mut resolved = HashMap::new();
    let names = raw
        .keys()
        .chain(aliases.keys())
        .cloned()
        .collect::<Vec<_>>();
    for name in names {
        resolve_named_type(
            &name,
            &raw,
            &aliases,
            &generic,
            &mut resolved,
            &mut Vec::new(),
        )?;
    }
    Ok((resolved, generic))
}

fn resolve_named_type(
    name: &str,
    interfaces: &HashMap<Symbol, &TsInterfaceDecl>,
    aliases: &HashMap<Symbol, &swc_ecma_ast::TsTypeAliasDecl>,
    generic: &GenericInterfaces,
    resolved: &mut HashMap<Symbol, HirType>,
    in_progress: &mut Vec<Symbol>,
) -> Result<HirType, String> {
    if let Some(ty) = resolved.get(name) {
        return Ok(ty.clone());
    }
    if in_progress.iter().any(|active| active == name) {
        if interfaces.contains_key(name) {
            return resolve_interface(name, interfaces, aliases, generic, resolved, in_progress);
        }
        let mut cycle = in_progress.clone();
        cycle.push(name.to_string());
        return Err(format!(
            "type declaration cycle `{}` cannot use Thaw's fixed-size native layout",
            cycle.join(" -> ")
        ));
    }
    if interfaces.contains_key(name) {
        resolve_interface(name, interfaces, aliases, generic, resolved, in_progress)
    } else if let Some(alias) = aliases.get(name) {
        in_progress.push(name.to_string());
        let ty = resolve_type_with_interfaces(
            &alias.type_ann,
            interfaces,
            aliases,
            generic,
            resolved,
            in_progress,
        )?;
        in_progress.pop();
        resolved.insert(name.to_string(), ty.clone());
        Ok(ty)
    } else {
        Err(format!("unknown type declaration `{name}`"))
    }
}

fn resolve_interface(
    name: &str,
    raw: &HashMap<Symbol, &TsInterfaceDecl>,
    aliases: &HashMap<Symbol, &swc_ecma_ast::TsTypeAliasDecl>,
    generic: &GenericInterfaces,
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
        let Expr::Ident(base_ident) = base.expr.as_ref() else {
            return Err(format!(
                "interface `{name}` has an unsupported `extends` target (only a plain interface name is supported)"
            ));
        };
        let base_name = base_ident.sym.to_string();
        let base_ty = if let Some(base_decl) = generic.interfaces.get(&base_name) {
            resolve_generic_interface_dependencies(
                &base_name,
                base_decl,
                raw,
                aliases,
                generic,
                resolved,
                in_progress,
                &mut Vec::new(),
            )?;
            let reference = swc_ecma_ast::TsTypeRef {
                span: base.span,
                type_name: swc_ecma_ast::TsEntityName::Ident(base_ident.clone()),
                type_params: base.type_args.clone(),
            };
            resolve_generic_interface(
                &base_name,
                base_decl,
                &reference,
                resolved,
                generic,
                None,
                in_progress,
            )?
        } else if let Some(base_decl) = generic.aliases.get(&base_name) {
            resolve_type_dependencies(
                &base_decl.type_ann,
                raw,
                aliases,
                generic,
                resolved,
                in_progress,
            )?;
            let reference = swc_ecma_ast::TsTypeRef {
                span: base.span,
                type_name: swc_ecma_ast::TsEntityName::Ident(base_ident.clone()),
                type_params: base.type_args.clone(),
            };
            resolve_generic_alias(
                &base_name,
                base_decl,
                &reference,
                resolved,
                generic,
                None,
                in_progress,
            )?
        } else {
            if base.type_args.is_some() {
                return Err(format!(
                    "interface `{name}` supplies type arguments to non-generic base `{base_name}`"
                ));
            }
            resolve_interface(&base_name, raw, aliases, generic, resolved, in_progress)?
        };
        let HirType::Object(base_fields) = base_ty else {
            return Err(format!(
                "interface `{name}` can only extend object-shaped interface `{base_name}`"
            ));
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
        let field_ty = resolve_type_with_interfaces(
            &ann.type_ann,
            raw,
            aliases,
            generic,
            resolved,
            in_progress,
        )?;
        fields.push((field_name, field_ty));
    }

    in_progress.pop();

    let hir_ty = HirType::Object(fields);
    resolved.insert(name.to_string(), hir_ty.clone());
    Ok(hir_ty)
}

#[allow(clippy::too_many_arguments)]
fn resolve_generic_interface_dependencies(
    name: &str,
    decl: &TsInterfaceDecl,
    raw: &HashMap<Symbol, &TsInterfaceDecl>,
    aliases: &HashMap<Symbol, &swc_ecma_ast::TsTypeAliasDecl>,
    generic: &GenericInterfaces,
    resolved: &mut HashMap<Symbol, HirType>,
    in_progress: &mut Vec<Symbol>,
    dependency_progress: &mut Vec<Symbol>,
) -> Result<(), String> {
    if dependency_progress.iter().any(|active| active == name) {
        return Ok(());
    }
    dependency_progress.push(name.to_string());
    for base in &decl.extends {
        let Expr::Ident(base_ident) = base.expr.as_ref() else {
            return Err(format!(
                "generic interface `{name}` has an unsupported `extends` target"
            ));
        };
        let base_name = base_ident.sym.as_str();
        if raw.contains_key(base_name) || aliases.contains_key(base_name) {
            resolve_named_type(base_name, raw, aliases, generic, resolved, in_progress)?;
        } else if let Some(base_decl) = generic.interfaces.get(base_name) {
            resolve_generic_interface_dependencies(
                base_name,
                base_decl,
                raw,
                aliases,
                generic,
                resolved,
                in_progress,
                dependency_progress,
            )?;
        }
    }
    let inserted_progress = !in_progress.iter().any(|active| active == name);
    if inserted_progress {
        in_progress.push(name.to_string());
    }
    for member in &decl.body.body {
        if let TsTypeElement::TsPropertySignature(property) = member {
            if let Some(annotation) = &property.type_ann {
                resolve_type_dependencies(
                    &annotation.type_ann,
                    raw,
                    aliases,
                    generic,
                    resolved,
                    in_progress,
                )?;
            }
        }
    }
    if inserted_progress {
        in_progress.pop();
    }
    dependency_progress.pop();
    Ok(())
}

/// Like `lower_ts_type`, but additionally resolves a `TsTypeRef` naming a
/// not-yet-resolved interface, recursively. Used only while building the
/// interface table (`resolve_interfaces`); everywhere else, `lower_ts_type`
/// consults the finished, read-only table instead.
fn resolve_type_with_interfaces(
    ty: &TsType,
    raw: &HashMap<Symbol, &TsInterfaceDecl>,
    aliases: &HashMap<Symbol, &swc_ecma_ast::TsTypeAliasDecl>,
    generic: &GenericInterfaces,
    resolved: &mut HashMap<Symbol, HirType>,
    in_progress: &mut Vec<Symbol>,
) -> Result<HirType, String> {
    resolve_type_dependencies(ty, raw, aliases, generic, resolved, in_progress)?;
    lower_ts_type(ty, resolved, generic)
}

fn resolve_type_dependencies(
    ty: &TsType,
    interfaces: &HashMap<Symbol, &TsInterfaceDecl>,
    aliases: &HashMap<Symbol, &swc_ecma_ast::TsTypeAliasDecl>,
    generic: &GenericInterfaces,
    resolved: &mut HashMap<Symbol, HirType>,
    in_progress: &mut Vec<Symbol>,
) -> Result<(), String> {
    match ty {
        TsType::TsTypeRef(reference) => {
            if let swc_ecma_ast::TsEntityName::Ident(id) = &reference.type_name {
                let name = id.sym.as_str();
                if interfaces.contains_key(name) || aliases.contains_key(name) {
                    resolve_named_type(name, interfaces, aliases, generic, resolved, in_progress)?;
                } else if let Some(decl) = generic.interfaces.get(name) {
                    if !in_progress.iter().any(|active| active == name) {
                        resolve_generic_interface_dependencies(
                            name,
                            decl,
                            interfaces,
                            aliases,
                            generic,
                            resolved,
                            in_progress,
                            &mut Vec::new(),
                        )?;
                    }
                } else if let Some(decl) = generic.aliases.get(name) {
                    if !in_progress.iter().any(|active| active == name) {
                        in_progress.push(name.to_string());
                        resolve_type_dependencies(
                            &decl.type_ann,
                            interfaces,
                            aliases,
                            generic,
                            resolved,
                            in_progress,
                        )?;
                        in_progress.pop();
                    }
                }
            }
            if let Some(arguments) = &reference.type_params {
                for argument in &arguments.params {
                    resolve_type_dependencies(
                        argument,
                        interfaces,
                        aliases,
                        generic,
                        resolved,
                        in_progress,
                    )?;
                }
            }
        }
        TsType::TsParenthesizedType(parenthesized) => resolve_type_dependencies(
            &parenthesized.type_ann,
            interfaces,
            aliases,
            generic,
            resolved,
            in_progress,
        )?,
        TsType::TsArrayType(array) => resolve_type_dependencies(
            &array.elem_type,
            interfaces,
            aliases,
            generic,
            resolved,
            in_progress,
        )?,
        TsType::TsTupleType(tuple) => {
            for element in &tuple.elem_types {
                resolve_type_dependencies(
                    &element.ty,
                    interfaces,
                    aliases,
                    generic,
                    resolved,
                    in_progress,
                )?;
            }
        }
        TsType::TsUnionOrIntersectionType(value) => {
            let elements = match value {
                TsUnionOrIntersectionType::TsUnionType(union) => &union.types,
                TsUnionOrIntersectionType::TsIntersectionType(intersection) => &intersection.types,
            };
            for element in elements {
                resolve_type_dependencies(
                    element,
                    interfaces,
                    aliases,
                    generic,
                    resolved,
                    in_progress,
                )?;
            }
        }
        TsType::TsTypeLit(literal) => {
            for member in &literal.members {
                if let TsTypeElement::TsPropertySignature(property) = member {
                    if let Some(annotation) = &property.type_ann {
                        resolve_type_dependencies(
                            &annotation.type_ann,
                            interfaces,
                            aliases,
                            generic,
                            resolved,
                            in_progress,
                        )?;
                    }
                }
            }
        }
        TsType::TsFnOrConstructorType(TsFnOrConstructorType::TsFnType(function)) => {
            for parameter in &function.params {
                if let TsFnParam::Ident(parameter) = parameter {
                    if let Some(annotation) = &parameter.type_ann {
                        resolve_type_dependencies(
                            &annotation.type_ann,
                            interfaces,
                            aliases,
                            generic,
                            resolved,
                            in_progress,
                        )?;
                    }
                }
            }
            resolve_type_dependencies(
                &function.type_ann.type_ann,
                interfaces,
                aliases,
                generic,
                resolved,
                in_progress,
            )?;
        }
        _ => {}
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn lower_class_constructor(
    declaration: &ClassDecl,
    signatures: &HashMap<Symbol, FnSignature>,
    interfaces: &HashMap<Symbol, HirType>,
    generic_interfaces: &GenericInterfaces,
    enum_values: &EnumValues,
    enum_reverse_values: &EnumReverseValues,
    global_types: &HashMap<Symbol, HirType>,
    immutable_globals: &HashSet<Symbol>,
) -> Result<Vec<HirFunction>, String> {
    let class_name = declaration.ident.sym.to_string();
    let constructor_symbol = class_constructor_symbol(&class_name);
    let initializer_symbol = class_initializer_symbol(&class_name);
    let instance_type = interfaces[&class_name].clone();
    let constructor = declaration
        .class
        .body
        .iter()
        .find_map(|member| match member {
            ClassMember::Constructor(constructor) => Some(constructor),
            _ => None,
        });
    let source_params = constructor
        .map(|constructor| constructor.params.as_slice())
        .unwrap_or_default();
    let mut params = Vec::with_capacity(source_params.len());
    for (index, parameter) in source_params.iter().enumerate() {
        let pattern = class_constructor_param_pattern(parameter);
        let mut parameter = lower_param(
            &pattern,
            interfaces,
            generic_interfaces,
            false,
            &HashMap::new(),
        )?;
        parameter.ty = signatures[&constructor_symbol].params[index].clone();
        params.push(parameter);
    }
    if constructor.is_none() && declaration.class.super_class.is_some() {
        params = signatures[&constructor_symbol]
            .params
            .iter()
            .enumerate()
            .map(|(index, ty)| HirParam {
                name: format!("__thaw_implicit_super_arg_{index}"),
                ty: ty.clone(),
            })
            .collect();
    }

    let this_name = "__thaw_this".to_string();
    let mut initializer_params = vec![HirParam {
        name: this_name.clone(),
        ty: instance_type.clone(),
    }];
    initializer_params.extend(params.iter().cloned());

    let mut lowerer = FnLowerer::new(
        signatures,
        interfaces,
        generic_interfaces,
        enum_values,
        enum_reverse_values,
        instance_type.clone(),
        None,
    );
    seed_global_scope(&mut lowerer, global_types, immutable_globals);
    if let Some(base) = &declaration.class.super_class {
        let Expr::Ident(base) = base.as_ref() else {
            unreachable!("class layout validation accepts identifier bases only")
        };
        lowerer.super_initializer = Some((
            class_initializer_symbol(base.sym.as_ref()),
            interfaces[base.sym.as_ref()].clone(),
            base.sym.to_string(),
        ));
    }
    lowerer
        .bindings
        .entry("this".into())
        .or_default()
        .push(this_name.clone());
    for parameter in &initializer_params {
        lowerer
            .scope
            .insert(parameter.name.clone(), parameter.ty.clone());
        lowerer
            .bindings
            .entry(parameter.name.clone())
            .or_default()
            .push(parameter.name.clone());
    }

    let mut own_initializers = Vec::new();
    for member in &declaration.class.body {
        let ClassMember::ClassProp(property) = member else {
            continue;
        };
        if property.is_static {
            continue;
        }
        if let Some(initializer) = &property.value {
            let field = class_property_name(&property.key)?;
            let HirType::Object(fields) = &instance_type else {
                unreachable!("native class layouts are fixed objects")
            };
            let expected = fields
                .iter()
                .find_map(|(name, ty)| (name == &field).then(|| ty.clone()))
                .ok_or_else(|| format!("class `{class_name}` has no field `{field}`"))?;
            let value = lowerer.lower_expr(initializer)?;
            let value = lowerer.coerce_to_declared(&expected, value)?;
            own_initializers.push(HirStmt::Expr(HirExpr::PropAssign(
                Box::new(HirExpr::Var(this_name.clone())),
                instance_type.clone(),
                field,
                Box::new(value),
            )));
        }
    }
    for (source, parameter) in source_params.iter().zip(&params) {
        let ParamOrTsParamProp::TsParamProp(property) = source else {
            continue;
        };
        let field = parameter_property_binding(property)?.id.sym.to_string();
        own_initializers.push(HirStmt::Expr(HirExpr::PropAssign(
            Box::new(HirExpr::Var(this_name.clone())),
            instance_type.clone(),
            field,
            Box::new(HirExpr::Var(parameter.name.clone())),
        )));
    }
    let mut initializer_body = Vec::new();
    if declaration.class.super_class.is_some() {
        if let Some(constructor) = constructor {
            let block = constructor.body.as_ref().ok_or_else(|| {
                format!("class `{class_name}` constructor needs an implementation body")
            })?;
            let mut called_super = false;
            for statement in &block.stmts {
                let is_super = matches!(statement, Stmt::Expr(expression) if matches!(expression.expr.as_ref(), Expr::Call(call) if matches!(call.callee, Callee::Super(_))));
                initializer_body.extend(lowerer.lower_stmt_seq(statement)?);
                if is_super {
                    if called_super {
                        return Err(format!(
                            "derived class `{class_name}` constructor calls super more than once"
                        ));
                    }
                    called_super = true;
                    initializer_body.append(&mut own_initializers);
                }
            }
            if !called_super {
                return Err(format!(
                    "derived class `{class_name}` constructor must call super(...)"
                ));
            }
        } else {
            let (base_initializer, _, _) = lowerer
                .super_initializer
                .clone()
                .expect("derived class has a base initializer");
            let signature = &signatures[&base_initializer];
            if signature.params.len() != params.len() + 1 {
                return Err(format!(
                    "derived class `{class_name}` cannot forward its implicit constructor arguments to the base"
                ));
            }
            let mut args = vec![HirExpr::Var(this_name.clone())];
            args.extend(
                params
                    .iter()
                    .map(|parameter| HirExpr::Var(parameter.name.clone())),
            );
            initializer_body.push(HirStmt::Expr(HirExpr::Call(
                Box::new(HirExpr::Var(base_initializer)),
                args,
            )));
            initializer_body.append(&mut own_initializers);
        }
    } else {
        initializer_body.append(&mut own_initializers);
        if let Some(constructor) = constructor {
            let block = constructor.body.as_ref().ok_or_else(|| {
                format!("class `{class_name}` constructor needs an implementation body")
            })?;
            initializer_body.extend(lowerer.lower_stmts(&block.stmts)?);
        }
    }
    initializer_body.push(HirStmt::Return(Some(HirExpr::Var(this_name.clone()))));

    let mut initialize_args = vec![HirExpr::Var(this_name.clone())];
    initialize_args.extend(
        params
            .iter()
            .map(|parameter| HirExpr::Var(parameter.name.clone())),
    );
    let constructor = HirFunction {
        name: constructor_symbol,
        params: params.clone(),
        ret: instance_type.clone(),
        is_async: false,
        body: vec![
            HirStmt::Let(
                this_name,
                instance_type.clone(),
                HirExpr::ObjectAlloc(instance_type.clone()),
            ),
            HirStmt::Return(Some(HirExpr::Call(
                Box::new(HirExpr::Var(initializer_symbol.clone())),
                initialize_args,
            ))),
        ],
    };
    let initializer = HirFunction {
        name: initializer_symbol,
        params: initializer_params,
        ret: instance_type,
        is_async: false,
        body: initializer_body,
    };
    Ok(vec![constructor, initializer])
}

#[allow(clippy::too_many_arguments)]
fn lower_class_methods(
    declaration: &ClassDecl,
    signatures: &HashMap<Symbol, FnSignature>,
    interfaces: &HashMap<Symbol, HirType>,
    generic_interfaces: &GenericInterfaces,
    enum_values: &EnumValues,
    enum_reverse_values: &EnumReverseValues,
    global_types: &HashMap<Symbol, HirType>,
    immutable_globals: &HashSet<Symbol>,
) -> Result<Vec<HirFunction>, String> {
    let class_name = declaration.ident.sym.to_string();
    let instance_type = interfaces[&class_name].clone();
    let mut functions = Vec::new();
    for member in &declaration.class.body {
        let ClassMember::Method(method) = member else {
            continue;
        };
        if !matches!(
            method.kind,
            MethodKind::Method | MethodKind::Getter | MethodKind::Setter
        ) {
            continue;
        }
        let method_name = class_property_name(&method.key)?;
        let symbol = match method.kind {
            MethodKind::Getter => class_getter_symbol(&class_name, &method_name, method.is_static),
            MethodKind::Setter => class_setter_symbol(&class_name, &method_name, method.is_static),
            MethodKind::Method if method.is_static => {
                class_static_method_symbol(&class_name, &method_name)
            }
            MethodKind::Method => class_method_symbol(&class_name, &method_name),
        };
        let signature = &signatures[&symbol];
        let mut params = if method.is_static {
            Vec::new()
        } else {
            vec![HirParam {
                name: "__thaw_this".into(),
                ty: instance_type.clone(),
            }]
        };
        let receiver_offset = usize::from(!method.is_static);
        for (index, parameter) in method.function.params.iter().enumerate() {
            let mut parameter = lower_param(
                &parameter.pat,
                interfaces,
                generic_interfaces,
                false,
                &HashMap::new(),
            )?;
            parameter.ty = signature.params[index + receiver_offset].clone();
            params.push(parameter);
        }
        let body =
            method.function.body.as_ref().ok_or_else(|| {
                format!("class `{class_name}` method `{method_name}` needs a body")
            })?;
        let mut lowerer = FnLowerer::new(
            signatures,
            interfaces,
            generic_interfaces,
            enum_values,
            enum_reverse_values,
            signature.ret.clone(),
            None,
        );
        seed_global_scope(&mut lowerer, global_types, immutable_globals);
        lowerer.class_static_context = method.is_static;
        if let Some(base) = &declaration.class.super_class {
            let Expr::Ident(base) = base.as_ref() else {
                unreachable!("class layout validation accepts identifier bases only")
            };
            lowerer.super_initializer = Some((
                class_initializer_symbol(base.sym.as_ref()),
                interfaces[base.sym.as_ref()].clone(),
                base.sym.to_string(),
            ));
        }
        for parameter in &params {
            lowerer
                .scope
                .insert(parameter.name.clone(), parameter.ty.clone());
            lowerer
                .bindings
                .entry(parameter.name.clone())
                .or_default()
                .push(parameter.name.clone());
        }
        if !method.is_static {
            lowerer
                .bindings
                .entry("this".into())
                .or_default()
                .push("__thaw_this".into());
        }
        let mut lowered_body = lowerer.lower_stmts(&body.stmts)?;
        if method.kind == MethodKind::Setter {
            lowered_body.push(HirStmt::Return(Some(HirExpr::Var(
                params.last().expect("setter value parameter").name.clone(),
            ))));
        }
        functions.push(HirFunction {
            name: symbol,
            params,
            ret: signature.ret.clone(),
            is_async: signature.is_async,
            body: lowered_body,
        });
    }
    Ok(functions)
}

#[allow(clippy::too_many_arguments)]
fn lower_fn_decl(
    fn_decl: &FnDecl,
    signatures: &HashMap<Symbol, FnSignature>,
    interfaces: &HashMap<Symbol, HirType>,
    generic_interfaces: &GenericInterfaces,
    enum_values: &EnumValues,
    enum_reverse_values: &EnumReverseValues,
    global_types: &HashMap<Symbol, HirType>,
    immutable_globals: &HashSet<Symbol>,
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
        enum_values,
        enum_reverse_values,
        declared_ret.clone(),
        call_constraints,
    );
    seed_global_scope(&mut lowerer, global_types, immutable_globals);
    for param in &params {
        lowerer.immutable_bindings.remove(&param.name);
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

fn seed_global_scope(
    lowerer: &mut FnLowerer<'_>,
    global_types: &HashMap<Symbol, HirType>,
    immutable_globals: &HashSet<Symbol>,
) {
    for (name, ty) in global_types {
        lowerer.scope.insert(name.clone(), ty.clone());
        lowerer
            .bindings
            .entry(name.clone())
            .or_default()
            .push(name.clone());
    }
    lowerer
        .immutable_bindings
        .extend(immutable_globals.iter().cloned());
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
        TsType::TsParenthesizedType(parenthesized) => lower_ts_type(
            &parenthesized.type_ann,
            interfaces,
            generic_interfaces,
        ),
        TsType::TsKeywordType(kw) => match kw.kind {
            TsKeywordTypeKind::TsNumberKeyword => Ok(HirType::F64),
            TsKeywordTypeKind::TsStringKeyword => Ok(HirType::Str),
            TsKeywordTypeKind::TsBooleanKeyword => Ok(HirType::Bool),
            TsKeywordTypeKind::TsUndefinedKeyword => Ok(HirType::Undefined),
            TsKeywordTypeKind::TsNullKeyword => Ok(HirType::Null),
            TsKeywordTypeKind::TsVoidKeyword => Ok(HirType::Void),
            other => Err(format!(
                "unsupported type keyword {other:?} (supports number/string/boolean/void)"
            )),
        },
        TsType::TsLitType(literal) => match &literal.lit {
            swc_ecma_ast::TsLit::Number(_) => Ok(HirType::F64),
            swc_ecma_ast::TsLit::Str(_) => Ok(HirType::Str),
            swc_ecma_ast::TsLit::Bool(_) => Ok(HirType::Bool),
            other => Err(format!("unsupported literal type {other:?}")),
        },
        TsType::TsUnionOrIntersectionType(TsUnionOrIntersectionType::TsUnionType(union)) => {
            let mut elements = Vec::new();
            for element in &union.types {
                let element = lower_ts_type(element, interfaces, generic_interfaces)?;
                if !elements.contains(&element) {
                    elements.push(element);
                }
            }
            if let [element] = elements.as_slice() {
                return Ok(element.clone());
            }
            if elements.len() == 3
                && elements.contains(&HirType::Null)
                && elements.contains(&HirType::Undefined)
            {
                let payloads = elements
                    .iter()
                    .filter(|element| {
                        **element != HirType::Null && **element != HirType::Undefined
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                if let [payload] = payloads.as_slice() {
                    return Ok(HirType::Nullish(Box::new(payload.clone())));
                }
            }
            if elements.len() == 2 {
                if elements[0] == HirType::Undefined {
                    return Ok(HirType::Optional(Box::new(elements[1].clone())));
                }
                if elements[1] == HirType::Undefined {
                    return Ok(HirType::Optional(Box::new(elements[0].clone())));
                }
                if elements[0] == HirType::Null {
                    return Ok(HirType::Nullable(Box::new(elements[1].clone())));
                }
                if elements[1] == HirType::Null {
                    return Ok(HirType::Nullable(Box::new(elements[0].clone())));
                }
            }
            if elements.len() >= 2
                && elements.iter().all(|element| {
                    matches!(
                        element,
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
                    )
                })
            {
                Ok(HirType::Union(elements))
            } else {
                Err(format!("unsupported union type {elements:?}"))
            }
        }
        TsType::TsUnionOrIntersectionType(TsUnionOrIntersectionType::TsIntersectionType(
            intersection,
        )) => {
            let elements = intersection
                .types
                .iter()
                .map(|element| lower_ts_type(element, interfaces, generic_interfaces))
                .collect::<Result<Vec<_>, _>>()?;
            let Some(first) = elements.first() else {
                return Err("empty intersection type is not supported".into());
            };
            if elements.iter().all(|element| element == first) {
                Ok(first.clone())
            } else if elements.iter().all(|element| matches!(element, HirType::Object(_))) {
                let mut merged = Vec::<(Symbol, HirType)>::new();
                for element in elements {
                    let HirType::Object(fields) = element else { unreachable!() };
                    for (name, ty) in fields {
                        if let Some((_, existing)) =
                            merged.iter().find(|(existing, _)| existing == &name)
                        {
                            if existing != &ty {
                                return Err(format!(
                                    "intersection field `{name}` has conflicting types {existing:?} and {ty:?}"
                                ));
                            }
                        } else {
                            merged.push((name, ty));
                        }
                    }
                }
                Ok(HirType::Object(merged))
            } else {
                Err(format!("unsupported intersection type {elements:?}"))
            }
        }
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
                if let Some(decl) = generic_interfaces.interfaces.get(name) {
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
                if let Some(decl) = generic_interfaces.aliases.get(name) {
                    return resolve_generic_alias(
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
/// Base interfaces are expanded before the interface's own fields, using the
/// same substitution for generic base arguments. Type arguments are resolved
/// through an optional outer substitution, so function type variables
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
    let parameters = &decl
        .type_params
        .as_ref()
        .expect("caller only reaches here for a generic interface")
        .params;

    let type_args: &[Box<TsType>] = ty_ref
        .type_params
        .as_ref()
        .map(|params| params.params.as_slice())
        .unwrap_or(&[]);
    let required = parameters
        .iter()
        .take_while(|parameter| parameter.default.is_none())
        .count();
    if type_args.len() < required || type_args.len() > parameters.len() {
        let expected = if required == parameters.len() {
            required.to_string()
        } else {
            format!("{required}..={}", parameters.len())
        };
        return Err(format!(
            "interface `{name}` expects {expected} type argument(s), got {}",
            type_args.len()
        ));
    }
    let mut substitution = HashMap::new();
    for (index, parameter) in parameters.iter().enumerate() {
        let concrete = if let Some(argument) = type_args.get(index) {
            match outer_substitution {
                Some(outer) => resolve_ts_type_with_substitution(
                    argument,
                    outer,
                    interfaces,
                    generic_interfaces,
                    in_progress,
                )?,
                None => lower_ts_type(argument, interfaces, generic_interfaces)?,
            }
        } else {
            resolve_ts_type_with_substitution(
                parameter
                    .default
                    .as_ref()
                    .expect("arity validation requires a default"),
                &substitution,
                interfaces,
                generic_interfaces,
                in_progress,
            )?
        };
        if let Some(constraint) = &parameter.constraint {
            let constraint = resolve_ts_type_with_substitution(
                constraint,
                &substitution,
                interfaces,
                generic_interfaces,
                in_progress,
            )?;
            if !type_satisfies_constraint(&concrete, &constraint) {
                return Err(format!(
                    "type argument {concrete:?} does not satisfy constraint {constraint:?} for `{}` in interface `{name}`",
                    parameter.name.sym
                ));
            }
        }
        substitution.insert(parameter.name.sym.to_string(), concrete);
    }

    in_progress.push(name.to_string());

    let mut fields = Vec::with_capacity(decl.body.body.len());
    for base in &decl.extends {
        let Expr::Ident(base_ident) = base.expr.as_ref() else {
            return Err(format!(
                "generic interface `{name}` has an unsupported `extends` target (only a plain interface name is supported)"
            ));
        };
        let base_name = base_ident.sym.as_str();
        let base_ty = if let Some(base_decl) = generic_interfaces.interfaces.get(base_name) {
            let reference = swc_ecma_ast::TsTypeRef {
                span: base.span,
                type_name: swc_ecma_ast::TsEntityName::Ident(base_ident.clone()),
                type_params: base.type_args.clone(),
            };
            resolve_generic_interface(
                base_name,
                base_decl,
                &reference,
                interfaces,
                generic_interfaces,
                Some(&substitution),
                in_progress,
            )?
        } else {
            if base.type_args.is_some() {
                return Err(format!(
                    "interface `{name}` supplies type arguments to non-generic base `{base_name}`"
                ));
            }
            interfaces
                .get(base_name)
                .cloned()
                .ok_or_else(|| format!("unknown base interface `{base_name}` for `{name}`"))?
        };
        let HirType::Object(base_fields) = base_ty else {
            return Err(format!(
                "interface `{name}` can only extend object-shaped interface `{base_name}`"
            ));
        };
        for (field_name, field_ty) in base_fields {
            if fields.iter().any(|(existing, _)| existing == &field_name) {
                return Err(format!(
                    "interface `{name}` inherits field `{field_name}` from `{base_name}`, which collides with an earlier field of the same name"
                ));
            }
            fields.push((field_name, field_ty));
        }
    }
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
        if fields.iter().any(|(existing, _)| existing == &field_name) {
            return Err(format!(
                "interface `{name}` declares field `{field_name}`, which collides with an inherited field"
            ));
        }
        fields.push((field_name, field_ty));
    }

    in_progress.pop();

    Ok(HirType::Object(fields))
}

fn resolve_generic_alias(
    name: &str,
    decl: &swc_ecma_ast::TsTypeAliasDecl,
    ty_ref: &swc_ecma_ast::TsTypeRef,
    interfaces: &HashMap<Symbol, HirType>,
    generic_interfaces: &GenericInterfaces,
    outer_substitution: Option<&HashMap<Symbol, HirType>>,
    in_progress: &mut Vec<Symbol>,
) -> Result<HirType, String> {
    if in_progress.iter().any(|active| active == name) {
        return Err(format!(
            "generic type alias `{name}` is (indirectly) self-referential"
        ));
    }
    let parameters = &decl
        .type_params
        .as_ref()
        .expect("caller only reaches generic aliases")
        .params;
    let arguments = ty_ref
        .type_params
        .as_ref()
        .map(|parameters| parameters.params.as_slice())
        .unwrap_or_default();
    let required = parameters
        .iter()
        .take_while(|parameter| parameter.default.is_none())
        .count();
    if arguments.len() < required || arguments.len() > parameters.len() {
        let expected = if required == parameters.len() {
            required.to_string()
        } else {
            format!("{required}..={}", parameters.len())
        };
        return Err(format!(
            "type alias `{name}` expects {expected} type argument(s), got {}",
            arguments.len()
        ));
    }
    let mut substitution = HashMap::new();
    for (index, parameter) in parameters.iter().enumerate() {
        let concrete = if let Some(argument) = arguments.get(index) {
            match outer_substitution {
                Some(outer) => resolve_ts_type_with_substitution(
                    argument,
                    outer,
                    interfaces,
                    generic_interfaces,
                    in_progress,
                )?,
                None => lower_ts_type(argument, interfaces, generic_interfaces)?,
            }
        } else {
            resolve_ts_type_with_substitution(
                parameter
                    .default
                    .as_ref()
                    .expect("arity validation requires a default"),
                &substitution,
                interfaces,
                generic_interfaces,
                in_progress,
            )?
        };
        if let Some(constraint) = &parameter.constraint {
            let constraint = resolve_ts_type_with_substitution(
                constraint,
                &substitution,
                interfaces,
                generic_interfaces,
                in_progress,
            )?;
            if !type_satisfies_constraint(&concrete, &constraint) {
                return Err(format!(
                    "type argument {concrete:?} does not satisfy constraint {constraint:?} for `{}` in alias `{name}`",
                    parameter.name.sym
                ));
            }
        }
        substitution.insert(parameter.name.sym.to_string(), concrete);
    }
    in_progress.push(name.to_string());
    let result = resolve_ts_type_with_substitution(
        &decl.type_ann,
        &substitution,
        interfaces,
        generic_interfaces,
        in_progress,
    );
    in_progress.pop();
    result
}

fn type_satisfies_constraint(actual: &HirType, constraint: &HirType) -> bool {
    if actual == constraint || constraint == &HirType::Dynamic {
        return true;
    }
    match constraint {
        HirType::Union(elements) => elements
            .iter()
            .any(|element| type_satisfies_constraint(actual, element)),
        HirType::Object(required) => match actual {
            HirType::Object(fields) => required.iter().all(|(name, ty)| {
                fields
                    .iter()
                    .find(|(field, _)| field == name)
                    .is_some_and(|(_, actual)| type_satisfies_constraint(actual, ty))
            }),
            _ => false,
        },
        _ => false,
    }
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
            if let Some(decl) = generic_interfaces.interfaces.get(ref_name) {
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
            if let Some(decl) = generic_interfaces.aliases.get(ref_name) {
                return resolve_generic_alias(
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
        TsType::TsParenthesizedType(parenthesized) => resolve_ts_type_with_substitution(
            &parenthesized.type_ann,
            substitution,
            interfaces,
            generic_interfaces,
            in_progress,
        ),
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
        TsType::TsUnionOrIntersectionType(TsUnionOrIntersectionType::TsUnionType(union)) => {
            let mut elements = Vec::new();
            for element in &union.types {
                let element = resolve_ts_type_with_substitution(
                    element,
                    substitution,
                    interfaces,
                    generic_interfaces,
                    in_progress,
                )?;
                if !elements.contains(&element) {
                    elements.push(element);
                }
            }
            if let [element] = elements.as_slice() {
                return Ok(element.clone());
            }
            if elements.len() == 3
                && elements.contains(&HirType::Null)
                && elements.contains(&HirType::Undefined)
            {
                if let Some(payload) = elements
                    .iter()
                    .find(|element| !matches!(element, HirType::Null | HirType::Undefined))
                {
                    return Ok(HirType::Nullish(Box::new(payload.clone())));
                }
            }
            if elements.len() == 2 {
                if let Some(payload) = elements
                    .iter()
                    .find(|element| **element != HirType::Undefined)
                    .filter(|_| elements.contains(&HirType::Undefined))
                {
                    return Ok(HirType::Optional(Box::new(payload.clone())));
                }
                if let Some(payload) = elements
                    .iter()
                    .find(|element| **element != HirType::Null)
                    .filter(|_| elements.contains(&HirType::Null))
                {
                    return Ok(HirType::Nullable(Box::new(payload.clone())));
                }
            }
            Ok(HirType::Union(elements))
        }
        TsType::TsUnionOrIntersectionType(TsUnionOrIntersectionType::TsIntersectionType(
            intersection,
        )) => {
            let elements = intersection
                .types
                .iter()
                .map(|element| {
                    resolve_ts_type_with_substitution(
                        element,
                        substitution,
                        interfaces,
                        generic_interfaces,
                        in_progress,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            let Some(first) = elements.first() else {
                return Err("empty intersection type is not supported".into());
            };
            if elements.iter().all(|element| element == first) {
                return Ok(first.clone());
            }
            if elements
                .iter()
                .all(|element| matches!(element, HirType::Object(_)))
            {
                let mut fields = Vec::<(Symbol, HirType)>::new();
                for element in elements {
                    let HirType::Object(element_fields) = element else {
                        unreachable!()
                    };
                    for (name, ty) in element_fields {
                        if let Some((_, existing)) =
                            fields.iter().find(|(existing, _)| existing == &name)
                        {
                            if existing != &ty {
                                return Err(format!(
                                    "intersection field `{name}` has conflicting types"
                                ));
                            }
                        } else {
                            fields.push((name, ty));
                        }
                    }
                }
                return Ok(HirType::Object(fields));
            }
            Err(format!("unsupported intersection type {elements:?}"))
        }
        TsType::TsFnOrConstructorType(TsFnOrConstructorType::TsFnType(function)) => {
            if function.type_params.is_some() {
                return Err("generic function types are not supported yet".into());
            }
            let params = function
                .params
                .iter()
                .map(|parameter| {
                    let TsFnParam::Ident(parameter) = parameter else {
                        return Err("function types only support identifier parameters".into());
                    };
                    let annotation = parameter.type_ann.as_ref().ok_or_else(|| {
                        format!(
                            "function parameter `{}` needs a type annotation",
                            parameter.id.sym
                        )
                    })?;
                    resolve_ts_type_with_substitution(
                        &annotation.type_ann,
                        substitution,
                        interfaces,
                        generic_interfaces,
                        in_progress,
                    )
                })
                .collect::<Result<Vec<_>, String>>()?;
            let ret = resolve_ts_type_with_substitution(
                &function.type_ann.type_ann,
                substitution,
                interfaces,
                generic_interfaces,
                in_progress,
            )?;
            Ok(HirType::Function(params, Box::new(ret)))
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
#[derive(Clone)]
enum Target {
    Var(Symbol),
    Index(HirExpr, Box<HirExpr>),
    Prop(HirExpr, HirType, Symbol),
}

#[derive(Clone, Copy)]
enum ArrayPredicateMode {
    Some,
    Every,
    Find,
    FindIndex,
    FindLast,
    FindLastIndex,
}

fn target_to_read_expr(target: &Target) -> HirExpr {
    match target {
        Target::Var(name) => HirExpr::Var(name.clone()),
        Target::Index(arr, idx) => HirExpr::Index(Box::new(arr.clone()), idx.clone()),
        Target::Prop(obj, ty, field) => {
            HirExpr::PropAccess(Box::new(obj.clone()), ty.clone(), field.clone())
        }
    }
}

fn build_assign(target: Target, value: HirExpr) -> HirExpr {
    match target {
        Target::Var(name) => HirExpr::Assign(name, Box::new(value)),
        Target::Index(arr, idx) => HirExpr::IndexAssign(Box::new(arr), idx, Box::new(value)),
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
        | HirExpr::UnionMemberIsEqual(left, right, _, _)
        | HirExpr::UnionIsEqual(left, right, _)
        | HirExpr::Index(left, right)
        | HirExpr::TypedIndex(left, right, _)
        | HirExpr::ArraySetLen(left, right, _)
        | HirExpr::DynamicPropAccess(left, right, _, _) => {
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
        | HirExpr::ArrayAlloc(value, _)
        | HirExpr::ArrayLen(value)
        | HirExpr::EnumReverseLookup(value, _)
        | HirExpr::JsonAsNumber(value)
        | HirExpr::JsonAsString(value)
        | HirExpr::JsonAsBool(value)
        | HirExpr::UnionInject(value, _, _)
        | HirExpr::UnionTag(value, _)
        | HirExpr::UnionValue(value, _, _)
        | HirExpr::OptionalSome(value, _)
        | HirExpr::OptionalIsNone(value, _)
        | HirExpr::OptionalValue(value, _)
        | HirExpr::NullableSome(value, _)
        | HirExpr::NullableIsNone(value, _)
        | HirExpr::NullableValue(value, _)
        | HirExpr::NullishSome(value, _)
        | HirExpr::NullishIsNull(value, _)
        | HirExpr::NullishIsUndefined(value, _)
        | HirExpr::NullishIsNone(value, _)
        | HirExpr::NullishValue(value, _) => collect_referenced_bindings(value, names),
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
        HirExpr::Lit(_)
        | HirExpr::OptionalNone(_)
        | HirExpr::NullableNone(_)
        | HirExpr::NullishNull(_)
        | HirExpr::NullishUndefined(_)
        | HirExpr::EnvVar(_)
        | HirExpr::ObjectAlloc(_)
        | HirExpr::FunctionRef(..) => {}
    }
}

fn contains_await(expr: &HirExpr) -> bool {
    match expr {
        HirExpr::Await(_) | HirExpr::AwaitPromise(_, _) => true,
        HirExpr::BinOp(_, left, right)
        | HirExpr::UnionMemberIsEqual(left, right, _, _)
        | HirExpr::UnionIsEqual(left, right, _)
        | HirExpr::Index(left, right)
        | HirExpr::TypedIndex(left, right, _)
        | HirExpr::ArraySetLen(left, right, _)
        | HirExpr::DynamicPropAccess(left, right, _, _)
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
        | HirExpr::ArrayAlloc(value, _)
        | HirExpr::ArrayLen(value)
        | HirExpr::EnumReverseLookup(value, _)
        | HirExpr::PropAccess(value, _, _)
        | HirExpr::JsonGet(value, _)
        | HirExpr::JsonAsNumber(value)
        | HirExpr::JsonAsString(value)
        | HirExpr::JsonAsBool(value)
        | HirExpr::UnionInject(value, _, _)
        | HirExpr::UnionTag(value, _)
        | HirExpr::UnionValue(value, _, _)
        | HirExpr::OptionalSome(value, _)
        | HirExpr::OptionalIsNone(value, _)
        | HirExpr::OptionalValue(value, _)
        | HirExpr::NullableSome(value, _)
        | HirExpr::NullableIsNone(value, _)
        | HirExpr::NullableValue(value, _)
        | HirExpr::NullishSome(value, _)
        | HirExpr::NullishIsNull(value, _)
        | HirExpr::NullishIsUndefined(value, _)
        | HirExpr::NullishIsNone(value, _)
        | HirExpr::NullishValue(value, _) => contains_await(value),
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
        | HirExpr::OptionalNone(_)
        | HirExpr::NullableNone(_)
        | HirExpr::NullishNull(_)
        | HirExpr::NullishUndefined(_)
        | HirExpr::Var(_)
        | HirExpr::EnvVar(_)
        | HirExpr::ObjectAlloc(_) => false,
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
    collect_stmt_bindings_with_bound(stmts, names, &BTreeSet::new());
}

fn collect_expr_bindings_with_bound(
    expr: &HirExpr,
    names: &mut BTreeSet<Symbol>,
    bound: &BTreeSet<Symbol>,
) {
    let mut referenced = BTreeSet::new();
    collect_referenced_bindings(expr, &mut referenced);
    names.extend(referenced.into_iter().filter(|name| !bound.contains(name)));
}

fn collect_stmt_bindings_with_bound(
    stmts: &[HirStmt],
    names: &mut BTreeSet<Symbol>,
    initial_bound: &BTreeSet<Symbol>,
) {
    let mut bound = initial_bound.clone();
    for stmt in stmts {
        match stmt {
            HirStmt::Expr(expr) | HirStmt::Throw(expr) => {
                collect_expr_bindings_with_bound(expr, names, &bound)
            }
            HirStmt::Let(name, _, expr) => {
                collect_expr_bindings_with_bound(expr, names, &bound);
                bound.insert(name.clone());
            }
            HirStmt::Return(Some(expr)) => collect_expr_bindings_with_bound(expr, names, &bound),
            HirStmt::If(cond, then_body, else_body) => {
                collect_expr_bindings_with_bound(cond, names, &bound);
                collect_stmt_bindings_with_bound(then_body, names, &bound);
                collect_stmt_bindings_with_bound(else_body, names, &bound);
            }
            HirStmt::While(cond, body) => {
                collect_expr_bindings_with_bound(cond, names, &bound);
                collect_stmt_bindings_with_bound(body, names, &bound);
            }
            HirStmt::Try(body, catch_name, catch_body) => {
                collect_stmt_bindings_with_bound(body, names, &bound);
                let mut catch_bound = bound.clone();
                catch_bound.insert(catch_name.clone());
                collect_stmt_bindings_with_bound(catch_body, names, &catch_bound);
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
        HirType::Undefined => Some("undefined"),
        HirType::Null => Some("object"),
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
        HirType::Optional(_)
        | HirType::Nullable(_)
        | HirType::Nullish(_)
        | HirType::Void
        | HirType::Dynamic
        | HirType::JsValue => None,
    }
}

/// Lowers one function body. Holds the type scope (params + `let`s seen so
/// far) and the whole module's function signatures, needed to resolve
/// member access (`arr.length` vs `obj.field`) and to type-check/reorder
/// object literals against their declared shape.
struct FnLowerer<'a> {
    scope: HashMap<Symbol, HirType>,
    immutable_bindings: HashSet<Symbol>,
    narrowings: HashMap<Symbol, HirType>,
    nullable_narrowings: HashMap<Symbol, HirType>,
    nullish_narrowings: HashMap<Symbol, HirType>,
    union_narrowings: HashMap<Symbol, (Vec<usize>, Vec<HirType>)>,
    bindings: HashMap<Symbol, Vec<Symbol>>,
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
    generic_named_templates: HashMap<Symbol, Symbol>,
    loop_depth: usize,
    labels: Vec<(Symbol, usize, bool)>,
    super_initializer: Option<(Symbol, HirType, Symbol)>,
    class_static_context: bool,
}

type UnionTypeofNarrowing = (Symbol, Vec<usize>, Vec<usize>, Vec<HirType>, bool);

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
        enum_values: &'a EnumValues,
        enum_reverse_values: &'a EnumReverseValues,
        ret_type: HirType,
        call_constraints: Option<&'a RefCell<Vec<CallConstraint>>>,
    ) -> Self {
        Self {
            scope: HashMap::new(),
            immutable_bindings: HashSet::new(),
            narrowings: HashMap::new(),
            nullable_narrowings: HashMap::new(),
            nullish_narrowings: HashMap::new(),
            union_narrowings: HashMap::new(),
            bindings: HashMap::new(),
            next_binding: 0,
            signatures,
            interfaces,
            generic_interfaces,
            enum_values,
            enum_reverse_values,
            ret_type,
            call_constraints,
            generic_call_returns: HashMap::new(),
            generic_arrows: HashMap::new(),
            generic_named_templates: HashMap::new(),
            loop_depth: 0,
            labels: Vec::new(),
            super_initializer: None,
            class_static_context: false,
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
        let saved_narrowings = self.narrowings.clone();
        let saved_nullable_narrowings = self.nullable_narrowings.clone();
        let saved_nullish_narrowings = self.nullish_narrowings.clone();
        let saved_union_narrowings = self.union_narrowings.clone();
        let saved_generic_arrows = self.generic_arrows.clone();
        let saved_generic_named_templates = self.generic_named_templates.clone();
        let lowered = self.lower_stmts(stmts);
        self.bindings = saved;
        self.narrowings = saved_narrowings;
        self.nullable_narrowings = saved_nullable_narrowings;
        self.nullish_narrowings = saved_nullish_narrowings;
        self.union_narrowings = saved_union_narrowings;
        self.generic_arrows = saved_generic_arrows;
        self.generic_named_templates = saved_generic_named_templates;
        lowered
    }

    fn lower_stmts(&mut self, stmts: &[Stmt]) -> Result<Vec<HirStmt>, String> {
        let mut out = Vec::new();
        for stmt in stmts {
            out.extend(self.lower_stmt_seq(stmt)?);
            if let Stmt::If(if_stmt) = stmt {
                if if_stmt.alt.is_none() && Self::stmt_definitely_exits(&if_stmt.cons) {
                    if let Some((name, payload, present_when_true, absence_kind)) =
                        self.optional_undefined_narrowing(&if_stmt.test)
                    {
                        if !present_when_true {
                            match absence_kind {
                                0 => {
                                    self.narrowings.insert(name, payload);
                                }
                                1 => {
                                    self.nullable_narrowings.insert(name, payload);
                                }
                                2 => {
                                    self.nullish_narrowings.insert(name, payload);
                                }
                                _ => unreachable!(),
                            }
                        }
                    }
                    if let Some((name, matching, allowed, elements, equal_when_true)) =
                        self.union_typeof_narrowing(&if_stmt.test)
                    {
                        let continuing = if equal_when_true {
                            allowed
                                .into_iter()
                                .filter(|index| !matching.contains(index))
                                .collect::<Vec<_>>()
                        } else {
                            matching
                        };
                        if !continuing.is_empty() {
                            self.union_narrowings.insert(name, (continuing, elements));
                        }
                    }
                }
            }
        }
        Ok(out)
    }

    fn stmt_definitely_exits(stmt: &Stmt) -> bool {
        match stmt {
            Stmt::Return(_) | Stmt::Throw(_) => true,
            Stmt::Block(block) => block.stmts.last().is_some_and(Self::stmt_definitely_exits),
            Stmt::If(if_stmt) => if_stmt.alt.as_ref().is_some_and(|alternative| {
                Self::stmt_definitely_exits(&if_stmt.cons)
                    && Self::stmt_definitely_exits(alternative)
            }),
            _ => false,
        }
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

    fn lower_body_with_optional_narrowing(
        &mut self,
        stmt: &Stmt,
        narrowing: Option<&(Symbol, HirType, u8)>,
    ) -> Result<Vec<HirStmt>, String> {
        let saved = self.narrowings.clone();
        let saved_nullable = self.nullable_narrowings.clone();
        let saved_nullish = self.nullish_narrowings.clone();
        if let Some((name, payload, absence_kind)) = narrowing {
            match absence_kind {
                0 => {
                    self.narrowings.insert(name.clone(), payload.clone());
                }
                1 => {
                    self.nullable_narrowings
                        .insert(name.clone(), payload.clone());
                }
                2 => {
                    self.nullish_narrowings
                        .insert(name.clone(), payload.clone());
                }
                _ => unreachable!(),
            }
        }
        let lowered = self.lower_body(stmt);
        self.narrowings = saved;
        self.nullable_narrowings = saved_nullable;
        self.nullish_narrowings = saved_nullish;
        lowered
    }

    fn lower_expr_with_optional_narrowing(
        &mut self,
        expr: &Expr,
        narrowing: Option<&(Symbol, HirType, u8)>,
    ) -> Result<HirExpr, String> {
        let saved = self.narrowings.clone();
        let saved_nullable = self.nullable_narrowings.clone();
        let saved_nullish = self.nullish_narrowings.clone();
        if let Some((name, payload, absence_kind)) = narrowing {
            match absence_kind {
                0 => {
                    self.narrowings.insert(name.clone(), payload.clone());
                }
                1 => {
                    self.nullable_narrowings
                        .insert(name.clone(), payload.clone());
                }
                2 => {
                    self.nullish_narrowings
                        .insert(name.clone(), payload.clone());
                }
                _ => unreachable!(),
            }
        }
        let lowered = self.lower_expr(expr);
        self.narrowings = saved;
        self.nullable_narrowings = saved_nullable;
        self.nullish_narrowings = saved_nullish;
        lowered
    }

    /// Returns the optional binding tested by an undefined comparison and
    /// whether its payload is present in the true branch.
    fn optional_undefined_narrowing(&self, expr: &Expr) -> Option<(Symbol, HirType, bool, u8)> {
        if let Expr::Paren(paren) = expr {
            return self.optional_undefined_narrowing(&paren.expr);
        }
        if let Expr::Unary(unary) = expr {
            if unary.op == swc_ecma_ast::UnaryOp::Bang {
                return self
                    .optional_undefined_narrowing(&unary.arg)
                    .map(|(name, payload, present, nullable)| (name, payload, !present, nullable));
            }
            return None;
        }
        let Expr::Bin(binary) = expr else {
            return None;
        };
        if binary.op == BinaryOp::LogicalAnd {
            return self
                .optional_undefined_narrowing(&binary.left)
                .filter(|(_, _, present, _)| *present);
        }
        if binary.op == BinaryOp::LogicalOr {
            return self
                .optional_undefined_narrowing(&binary.left)
                .filter(|(_, _, present, _)| !*present);
        }
        let present_when_true = match binary.op {
            BinaryOp::NotEqEq | BinaryOp::NotEq => true,
            BinaryOp::EqEqEq | BinaryOp::EqEq => false,
            _ => return None,
        };
        let typeof_ident = match (binary.left.as_ref(), binary.right.as_ref()) {
            (Expr::Unary(unary), Expr::Lit(Lit::Str(value)))
                if unary.op == UnaryOp::TypeOf && value.value == *"undefined" =>
            {
                match unary.arg.as_ref() {
                    Expr::Ident(ident) => Some(ident),
                    _ => None,
                }
            }
            (Expr::Lit(Lit::Str(value)), Expr::Unary(unary))
                if unary.op == UnaryOp::TypeOf && value.value == *"undefined" =>
            {
                match unary.arg.as_ref() {
                    Expr::Ident(ident) => Some(ident),
                    _ => None,
                }
            }
            _ => None,
        };
        if let Some(ident) = typeof_ident {
            let name = self.resolve_binding(ident.sym.as_ref());
            let HirType::Optional(payload) = self.scope.get(&name)? else {
                return None;
            };
            return Some((name, payload.as_ref().clone(), present_when_true, 0));
        }
        let (ident, nullable) = match (binary.left.as_ref(), binary.right.as_ref()) {
            (Expr::Ident(value), Expr::Ident(undefined)) if undefined.sym == *"undefined" => {
                (value, false)
            }
            (Expr::Ident(undefined), Expr::Ident(value)) if undefined.sym == *"undefined" => {
                (value, false)
            }
            (Expr::Ident(value), Expr::Lit(Lit::Null(_))) => (value, true),
            (Expr::Lit(Lit::Null(_)), Expr::Ident(value)) => (value, true),
            _ => return None,
        };
        if !nullable && self.scope.contains_key("undefined") {
            return None;
        }
        let name = self.resolve_binding(ident.sym.as_ref());
        let (payload, absence_kind) = match (self.scope.get(&name)?, nullable) {
            (HirType::Optional(payload), false) => (payload, 0),
            (HirType::Nullable(payload), true) => (payload, 1),
            (HirType::Nullish(payload), _)
                if matches!(binary.op, BinaryOp::EqEq | BinaryOp::NotEq) =>
            {
                (payload, 2)
            }
            _ => return None,
        };
        Some((
            name,
            payload.as_ref().clone(),
            present_when_true,
            absence_kind,
        ))
    }

    fn union_typeof_narrowing(&self, expr: &Expr) -> Option<UnionTypeofNarrowing> {
        let Expr::Bin(binary) = expr else { return None };
        let equal_when_true = match binary.op {
            BinaryOp::EqEqEq | BinaryOp::EqEq => true,
            BinaryOp::NotEqEq | BinaryOp::NotEq => false,
            _ => return None,
        };
        let (ident, type_name) = match (binary.left.as_ref(), binary.right.as_ref()) {
            (Expr::Unary(unary), Expr::Lit(Lit::Str(name))) if unary.op == UnaryOp::TypeOf => {
                let Expr::Ident(ident) = unary.arg.as_ref() else {
                    return None;
                };
                (ident, name.value.to_string_lossy())
            }
            (Expr::Lit(Lit::Str(name)), Expr::Unary(unary)) if unary.op == UnaryOp::TypeOf => {
                let Expr::Ident(ident) = unary.arg.as_ref() else {
                    return None;
                };
                (ident, name.value.to_string_lossy())
            }
            _ => return None,
        };
        let name = self.resolve_binding(ident.sym.as_ref());
        let HirType::Union(elements) = self.scope.get(&name)? else {
            return None;
        };
        let allowed = self
            .union_narrowings
            .get(&name)
            .map(|(allowed, _)| allowed.clone())
            .unwrap_or_else(|| (0..elements.len()).collect());
        let matching = allowed
            .iter()
            .copied()
            .filter(|index| native_typeof_name(&elements[*index]) == Some(type_name.as_ref()))
            .collect::<Vec<_>>();
        (!matching.is_empty()).then(|| (name, matching, allowed, elements.clone(), equal_when_true))
    }

    fn lower_body_with_union_narrowing(
        &mut self,
        stmt: &Stmt,
        narrowing: Option<&(Symbol, Vec<usize>, Vec<HirType>)>,
        optional: Option<&(Symbol, HirType, u8)>,
    ) -> Result<Vec<HirStmt>, String> {
        let saved = self.union_narrowings.clone();
        if let Some((name, allowed, elements)) = narrowing {
            self.union_narrowings
                .insert(name.clone(), (allowed.clone(), elements.clone()));
        }
        let lowered = self.lower_body_with_optional_narrowing(stmt, optional);
        self.union_narrowings = saved;
        lowered
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
                let narrowing = self.optional_undefined_narrowing(&if_stmt.test);
                let union_narrowing = self.union_typeof_narrowing(&if_stmt.test);
                let cond = self.lower_expr(&if_stmt.test)?;
                self.expect_type(&HirType::Bool, &cond, "if condition")?;
                let then_narrowing = narrowing
                    .as_ref()
                    .filter(|(_, _, present, _)| *present)
                    .map(|(name, payload, _, nullable)| {
                        (name.clone(), payload.clone(), *nullable)
                    });
                let else_narrowing = narrowing
                    .as_ref()
                    .filter(|(_, _, present, _)| !*present)
                    .map(|(name, payload, _, nullable)| {
                        (name.clone(), payload.clone(), *nullable)
                    });
                let then_union = union_narrowing.as_ref().map(
                    |(name, matching, allowed, elements, equal)| {
                        let narrowed = if *equal {
                            matching.clone()
                        } else {
                            allowed
                                .iter()
                                .filter(|index| !matching.contains(index))
                                .copied()
                                .collect()
                        };
                        (name.clone(), narrowed, elements.clone())
                    },
                );
                let else_union = union_narrowing.as_ref().map(
                    |(name, matching, allowed, elements, equal)| {
                        let narrowed = if !*equal {
                            matching.clone()
                        } else {
                            allowed
                                .iter()
                                .filter(|index| !matching.contains(index))
                                .copied()
                                .collect()
                        };
                        (name.clone(), narrowed, elements.clone())
                    },
                );
                let then_branch = self
                    .lower_body_with_union_narrowing(
                        &if_stmt.cons,
                        then_union.as_ref(),
                        then_narrowing.as_ref(),
                    )?;
                let else_branch = match &if_stmt.alt {
                    Some(alt) => self.lower_body_with_union_narrowing(
                        alt,
                        else_union.as_ref(),
                        else_narrowing.as_ref(),
                    )?,
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
                if let (Expr::Arrow(arrow), Some(annotation)) = (init, binding.type_ann.as_ref()) {
                    if arrow.type_params.is_some() {
                        if let Some((expected, callable_name)) =
                            self.generic_callable_annotation_signature(&annotation.type_ann)?
                        {
                            let actual = self.generic_arrow_signature(arrow)?;
                            self.validate_generic_callable_shape(
                                &expected,
                                &actual,
                                &callable_name,
                            )?;
                            let hir_name = self.bind_local(&name, HirType::Dynamic);
                            self.generic_arrows.insert(hir_name, arrow.clone());
                            continue;
                        }
                    }
                }
                if let (Expr::Fn(function), Some(annotation)) = (init, binding.type_ann.as_ref()) {
                    if function.function.type_params.is_some() {
                        if let Some((expected, callable_name)) =
                            self.generic_callable_annotation_signature(&annotation.type_ann)?
                        {
                            let arrow = function_expression_as_arrow(function)?;
                            let actual = self.generic_arrow_signature(&arrow)?;
                            self.validate_generic_callable_shape(
                                &expected,
                                &actual,
                                &callable_name,
                            )?;
                            let hir_name = self.bind_local(&name, HirType::Dynamic);
                            self.generic_arrows.insert(hir_name, arrow);
                            continue;
                        }
                    }
                }
                if let (Expr::Ident(identifier), Some(annotation)) =
                    (init, binding.type_ann.as_ref())
                {
                    if let Some((expected, callable_name)) =
                        self.generic_callable_annotation_signature(&annotation.type_ann)?
                    {
                        if let Some(actual) = self.signatures.get(identifier.sym.as_ref()) {
                            if !actual.generic_type_params.is_empty() {
                                self.validate_generic_callable_shape(
                                    &expected,
                                    actual,
                                    &callable_name,
                                )?;
                                let hir_name = self.bind_local(&name, HirType::Dynamic);
                                self.generic_named_templates
                                    .insert(hir_name, identifier.sym.to_string());
                                continue;
                            }
                        }
                    }
                }
                if let (Expr::Ident(identifier), Some(annotation)) =
                    (init, binding.type_ann.as_ref())
                {
                    if let Some((expected, callable_name)) =
                        self.generic_callable_annotation_signature(&annotation.type_ann)?
                    {
                        let source_name = self.resolve_binding(identifier.sym.as_ref());
                        if let Some(arrow) = self.generic_arrows.get(&source_name).cloned() {
                            let actual = self.generic_arrow_signature(&arrow)?;
                            self.validate_generic_callable_shape(
                                &expected,
                                &actual,
                                &callable_name,
                            )?;
                            let hir_name = self.bind_local(&name, HirType::Dynamic);
                            self.generic_arrows.insert(hir_name, arrow);
                            continue;
                        }
                        if let Some(target) =
                            self.generic_named_templates.get(&source_name).cloned()
                        {
                            let actual = self
                                .signatures
                                .get(&target)
                                .expect("named generic template target");
                            self.validate_generic_callable_shape(
                                &expected,
                                actual,
                                &callable_name,
                            )?;
                            let hir_name = self.bind_local(&name, HirType::Dynamic);
                            self.generic_named_templates.insert(hir_name, target);
                            continue;
                        }
                    }
                }
                if let (Expr::Ident(identifier), None) = (init, binding.type_ann.as_ref()) {
                    let source_name = self.resolve_binding(identifier.sym.as_ref());
                    if let Some(arrow) = self.generic_arrows.get(&source_name).cloned() {
                        let hir_name = self.bind_local(&name, HirType::Dynamic);
                        self.generic_arrows.insert(hir_name, arrow);
                        continue;
                    }
                    if let Some(target) = self.generic_named_templates.get(&source_name).cloned() {
                        let hir_name = self.bind_local(&name, HirType::Dynamic);
                        self.generic_named_templates.insert(hir_name, target);
                        continue;
                    }
                }
                let annotated = binding
                    .type_ann
                    .as_ref()
                    .map(|annotation| {
                        lower_ts_type(
                            &annotation.type_ann,
                            self.interfaces,
                            self.generic_interfaces,
                        )
                    })
                    .transpose()?;
                if annotated.is_none()
                    && (matches!(init, Expr::Arrow(arrow) if arrow.type_params.is_some())
                        || matches!(init, Expr::Fn(function) if function.function.type_params.is_some()))
                {
                    let arrow = match init {
                        Expr::Arrow(arrow) => arrow.clone(),
                        Expr::Fn(function) => function_expression_as_arrow(function)?,
                        _ => unreachable!(),
                    };
                    if arrow.is_async || arrow.is_generator {
                        return Err(
                            "async and generator generic arrow variables are not supported".into(),
                        );
                    }
                    let hir_name = self.bind_local(&name, HirType::Dynamic);
                    self.generic_arrows.insert(hir_name, arrow);
                    continue;
                }
                let value = match (init, annotated.as_ref()) {
                    (Expr::Arrow(arrow), Some(HirType::Function(params, ret))) => {
                        self.lower_contextual_arrow(arrow, params, Some(ret))?
                    }
                    (Expr::Fn(function), Some(HirType::Function(params, ret))) => {
                        let arrow = function_expression_as_arrow(function)?;
                        self.lower_contextual_arrow(&arrow, params, Some(ret))?
                    }
                    _ => self.lower_expr(init)?,
                };

                let ty = match annotated {
                    Some(ty) => ty,
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
                                field_value =
                                    self.lower_nullish_coalescing(field_value, default)?;
                                binding_type = self.infer_expr_type(&field_value)?;
                            }
                            self.lower_binding_pattern(
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
            Pat::Assign(assign) => {
                let default = self.lower_expr(&assign.right)?;
                let value = self.lower_nullish_coalescing(value, default)?;
                let value_type = self.infer_expr_type(&value)?;
                self.lower_binding_pattern(&assign.left, value, &value_type, statements)
            }
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
        if let HirType::Union(elements) = declared {
            let actual = self.infer_expr_type(&value)?;
            if &actual == declared {
                return Ok(value);
            }
            if let Some(index) = elements.iter().position(|element| element == &actual) {
                return Ok(HirExpr::UnionInject(
                    Box::new(value),
                    index,
                    elements.clone(),
                ));
            }
            return Err(format!(
                "value has type {actual:?}, which is not a member of {declared:?}"
            ));
        }
        if let HirType::Optional(payload) = declared {
            return match self.infer_expr_type(&value)? {
                HirType::Undefined => Ok(HirExpr::OptionalNone(payload.as_ref().clone())),
                actual if actual == *declared => Ok(value),
                _ => {
                    let value = self.coerce_to_declared(payload.as_ref(), value)?;
                    Ok(HirExpr::OptionalSome(
                        Box::new(value),
                        payload.as_ref().clone(),
                    ))
                }
            };
        }
        if let HirType::Nullable(payload) = declared {
            return match self.infer_expr_type(&value)? {
                HirType::Null => Ok(HirExpr::NullableNone(payload.as_ref().clone())),
                actual if actual == *declared => Ok(value),
                _ => {
                    let value = self.coerce_to_declared(payload.as_ref(), value)?;
                    Ok(HirExpr::NullableSome(
                        Box::new(value),
                        payload.as_ref().clone(),
                    ))
                }
            };
        }
        if let HirType::Nullish(payload) = declared {
            return match self.infer_expr_type(&value)? {
                HirType::Null => Ok(HirExpr::NullishNull(payload.as_ref().clone())),
                HirType::Undefined => Ok(HirExpr::NullishUndefined(payload.as_ref().clone())),
                actual if actual == *declared => Ok(value),
                _ => {
                    let value = self.coerce_to_declared(payload.as_ref(), value)?;
                    Ok(HirExpr::NullishSome(
                        Box::new(value),
                        payload.as_ref().clone(),
                    ))
                }
            };
        }
        if let (HirType::Array(element), HirExpr::ArrayLit(values)) = (declared, &value) {
            let values = values
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    self.coerce_to_declared(element, value.clone())
                        .map_err(|error| format!("array element {index}: {error}"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(HirExpr::ArrayLit(values));
        }
        if let (HirType::Tuple(expected), HirExpr::ArrayLit(values)) = (declared, &value) {
            if expected.len() != values.len() {
                return Err(format!(
                    "tuple literal has {} element(s), expected {}",
                    values.len(),
                    expected.len()
                ));
            }
            let values = expected
                .iter()
                .zip(values)
                .enumerate()
                .map(|(index, (expected, value))| {
                    self.coerce_to_declared(expected, value.clone())
                        .map_err(|error| format!("tuple element {index}: {error}"))
                })
                .collect::<Result<Vec<_>, String>>()?;
            return Ok(HirExpr::ArrayLit(values));
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
                let field_value = self
                    .coerce_to_declared(expected_ty, field_value.clone())
                    .map_err(|error| format!("field `{name}`: {error}"))?;
                Ok((name.clone(), field_value))
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
            HirExpr::Lit(HirLit::Undefined) => Ok(HirType::Undefined),
            HirExpr::Lit(HirLit::Null) => Ok(HirType::Null),
            HirExpr::Var(name) => self
                .scope
                .get(name)
                .cloned()
                .ok_or_else(|| format!("unknown variable `{name}`")),
            HirExpr::FunctionRef(_, params, ret) => {
                Ok(HirType::Function(params.clone(), Box::new(ret.clone())))
            }
            HirExpr::OptionalSome(value, payload) => {
                self.expect_type(payload, value, "optional payload")?;
                Ok(HirType::Optional(Box::new(payload.clone())))
            }
            HirExpr::OptionalNone(payload) => Ok(HirType::Optional(Box::new(payload.clone()))),
            HirExpr::OptionalIsNone(value, payload) => {
                self.expect_type(
                    &HirType::Optional(Box::new(payload.clone())),
                    value,
                    "optional test",
                )?;
                Ok(HirType::Bool)
            }
            HirExpr::OptionalValue(value, payload) => {
                self.expect_type(
                    &HirType::Optional(Box::new(payload.clone())),
                    value,
                    "optional value extraction",
                )?;
                Ok(payload.clone())
            }
            HirExpr::NullableSome(value, payload) => {
                self.expect_type(payload, value, "nullable payload")?;
                Ok(HirType::Nullable(Box::new(payload.clone())))
            }
            HirExpr::NullableNone(payload) => Ok(HirType::Nullable(Box::new(payload.clone()))),
            HirExpr::NullableIsNone(value, payload) => {
                self.expect_type(
                    &HirType::Nullable(Box::new(payload.clone())),
                    value,
                    "nullable null check",
                )?;
                Ok(HirType::Bool)
            }
            HirExpr::NullableValue(value, payload) => {
                self.expect_type(
                    &HirType::Nullable(Box::new(payload.clone())),
                    value,
                    "nullable payload extraction",
                )?;
                Ok(payload.clone())
            }
            HirExpr::NullishSome(value, payload) => {
                self.expect_type(payload, value, "nullish payload")?;
                Ok(HirType::Nullish(Box::new(payload.clone())))
            }
            HirExpr::NullishNull(payload) | HirExpr::NullishUndefined(payload) => {
                Ok(HirType::Nullish(Box::new(payload.clone())))
            }
            HirExpr::NullishIsNull(value, payload)
            | HirExpr::NullishIsUndefined(value, payload)
            | HirExpr::NullishIsNone(value, payload) => {
                self.expect_type(
                    &HirType::Nullish(Box::new(payload.clone())),
                    value,
                    "nullish tag check",
                )?;
                Ok(HirType::Bool)
            }
            HirExpr::NullishValue(value, payload) => {
                self.expect_type(
                    &HirType::Nullish(Box::new(payload.clone())),
                    value,
                    "nullish payload extraction",
                )?;
                Ok(payload.clone())
            }
            HirExpr::UnionInject(value, index, elements) => {
                let member = elements
                    .get(*index)
                    .ok_or_else(|| format!("union member index {index} is out of bounds"))?;
                self.expect_type(member, value, "union payload")?;
                Ok(HirType::Union(elements.clone()))
            }
            HirExpr::UnionTag(value, elements) => {
                self.expect_type(&HirType::Union(elements.clone()), value, "union tag access")?;
                Ok(HirType::F64)
            }
            HirExpr::UnionValue(value, index, elements) => {
                self.expect_type(
                    &HirType::Union(elements.clone()),
                    value,
                    "union value extraction",
                )?;
                elements
                    .get(*index)
                    .cloned()
                    .ok_or_else(|| format!("union member index {index} is out of bounds"))
            }
            HirExpr::UnionMemberIsEqual(union, member, index, elements) => {
                self.expect_type(
                    &HirType::Union(elements.clone()),
                    union,
                    "union equality receiver",
                )?;
                let expected = elements
                    .get(*index)
                    .ok_or_else(|| format!("union member index {index} is out of bounds"))?;
                self.expect_type(expected, member, "union equality member")?;
                Ok(HirType::Bool)
            }
            HirExpr::UnionIsEqual(left, right, elements) => {
                let union = HirType::Union(elements.clone());
                self.expect_type(&union, left, "union equality left operand")?;
                self.expect_type(&union, right, "union equality right operand")?;
                Ok(HirType::Bool)
            }
            HirExpr::ArrayAlloc(length, element) => {
                self.expect_type(&HirType::F64, length, "array allocation length")?;
                Ok(HirType::Array(Box::new(element.clone())))
            }
            HirExpr::ArraySetLen(array, length, element) => {
                self.expect_type(
                    &HirType::Array(Box::new(element.clone())),
                    array,
                    "array length update receiver",
                )?;
                self.expect_type(&HirType::F64, length, "array length update")?;
                Ok(HirType::Array(Box::new(element.clone())))
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
                    "__thaw_number_array_sort"
                    | "__thaw_string_array_sort"
                    | "__thaw_bool_array_sort"
                    | "__thaw_object_array_sort"
                    | "__thaw_number_array_to_sorted"
                    | "__thaw_string_array_to_sorted"
                    | "__thaw_bool_array_to_sorted"
                    | "__thaw_object_array_to_sorted" => {
                        let [array] = args.as_slice() else {
                            return Err("array sort expects one operand".into());
                        };
                        let ty = self.infer_expr_type(array)?;
                        if !matches!(ty, HirType::Array(_)) {
                            return Err("array sort requires a homogeneous array".into());
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
                    | "__thaw_string_trim_end"
                    | "__thaw_string_to_lower_case"
                    | "__thaw_string_to_upper_case" => {
                        let [argument] = args.as_slice() else {
                            return Err("string trim expects one operand".into());
                        };
                        self.expect_type(&HirType::Str, argument, "string trim receiver")?;
                        return Ok(HirType::Str);
                    }
                    "__thaw_string_to_array" => {
                        let [value] = args.as_slice() else {
                            return Err("string iterator conversion expects one operand".into());
                        };
                        self.expect_type(&HirType::Str, value, "string iterator source")?;
                        return Ok(HirType::Array(Box::new(HirType::Str)));
                    }
                    "__thaw_string_repeat" => {
                        let [value, count] = args.as_slice() else {
                            return Err("string repeat expects two operands".into());
                        };
                        self.expect_type(&HirType::Str, value, "string repeat receiver")?;
                        self.expect_type(&HirType::F64, count, "string repeat count")?;
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
                    "__thaw_number_object_is" => {
                        let [left, right] = args.as_slice() else {
                            return Err("Object.is number comparison expects two operands".into());
                        };
                        self.expect_type(&HirType::F64, left, "Object.is left operand")?;
                        self.expect_type(&HirType::F64, right, "Object.is right operand")?;
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
                if let Some(return_type) = self.generic_call_returns.get(name) {
                    return Ok(return_type.clone());
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
                            let types = infer_generic_type_tuple(
                                sig,
                                &actual,
                                self.interfaces,
                                self.generic_interfaces,
                            )?;
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
            HirExpr::EnumReverseLookup(_, _) => Ok(HirType::Optional(Box::new(HirType::Str))),
            HirExpr::EnvVar(_) => Ok(HirType::Str),
            HirExpr::ObjectLit(fields) => {
                let fields = fields
                    .iter()
                    .map(|(name, value)| Ok((name.clone(), self.infer_expr_type(value)?)))
                    .collect::<Result<Vec<_>, String>>()?;
                Ok(HirType::Object(fields))
            }
            HirExpr::ObjectAlloc(ty @ HirType::Object(_)) => Ok(ty.clone()),
            HirExpr::ObjectAlloc(other) => Err(format!(
                "object allocation requires an object type, got {other:?}"
            )),
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
            HirExpr::DynamicPropAccess(_, _, _, result) => Ok(result.clone()),
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

    fn lower_nullish_coalescing(&mut self, lhs: HirExpr, rhs: HirExpr) -> Result<HirExpr, String> {
        let lhs_type = self.infer_expr_type(&lhs)?;
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
                (HirType::Null, HirType::Null) => Some(HirExpr::Lit(HirLit::Bool(true))),
                (HirType::Null, _) | (_, HirType::Null) => Some(HirExpr::Lit(HirLit::Bool(false))),
                (HirType::Undefined, HirType::Undefined) => Some(HirExpr::Lit(HirLit::Bool(true))),
                (HirType::Undefined, _) | (_, HirType::Undefined) => {
                    Some(HirExpr::Lit(HirLit::Bool(false)))
                }
                (HirType::Union(left), HirType::Union(right)) if left == right => Some(
                    HirExpr::UnionIsEqual(Box::new(lhs), Box::new(rhs), left.clone()),
                ),
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
                let name = self.resolve_binding("this");
                if self.scope.contains_key(&name) {
                    Ok(HirExpr::Var(name))
                } else {
                    Err("`this` is only available inside a native class constructor or method".into())
                }
            }
            Expr::Paren(paren) => self.lower_expr(&paren.expr),
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
                let mut lhs = self.lower_expr(&bin.left)?;
                let rhs_narrowing = self
                    .optional_undefined_narrowing(&bin.left)
                    .filter(|(_, _, present, _)| {
                        (bin.op == BinaryOp::LogicalAnd && *present)
                            || (bin.op == BinaryOp::LogicalOr && !*present)
                    })
                    .map(|(name, payload, _, nullable)| (name, payload, nullable));
                let mut rhs = self
                    .lower_expr_with_optional_narrowing(&bin.right, rhs_narrowing.as_ref())?;
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
                    if self.signatures.contains_key(&constructor) {
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

    fn lower_contextual_arrow(
        &mut self,
        arrow: &swc_ecma_ast::ArrowExpr,
        parameter_types: &[HirType],
        expected_return: Option<&HirType>,
    ) -> Result<HirExpr, String> {
        if arrow.is_async || arrow.is_generator {
            return Err("async and generator Promise callbacks are not supported".into());
        }
        if arrow.params.len() != parameter_types.len() {
            return Err(format!(
                "Promise callback expects {} parameter(s), got {}",
                parameter_types.len(),
                arrow.params.len()
            ));
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
                ret: HirType::Dynamic,
                is_async: false,
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
            Expr::Fn(function) => {
                let arrow = function_expression_as_arrow(function)?;
                return self.lower_contextual_arrow(&arrow, parameter_types, expected_return);
            }
            Expr::Ident(ident) => {
                let mut name = self.resolve_binding(ident.sym.as_ref());
                if let Some(arrow) = self.generic_arrows.get(&name).cloned() {
                    return self.lower_contextual_arrow(&arrow, parameter_types, expected_return);
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
                            return Err(
                                "dynamic object index requires every field to have the same type"
                                    .into(),
                            );
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
        if let AssignTarget::Simple(SimpleAssignTarget::Member(member)) = &assign.left {
            if let (Expr::Ident(receiver), Some(property)) =
                (member.obj.as_ref(), member_property_name(&member.prop))
            {
                let getter = class_getter_symbol(receiver.sym.as_ref(), &property, true);
                let setter = class_setter_symbol(receiver.sym.as_ref(), &property, true);
                let has_getter = self.signatures.contains_key(&getter);
                let has_setter = self.signatures.contains_key(&setter);
                if has_getter && !has_setter {
                    return Err(format!(
                        "cannot assign to readonly static member `{}.{}`",
                        receiver.sym, property
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
            };
        }

        if assign.op == AssignOp::NullishAssign {
            let current = target_to_read_expr(&target);
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
                                field_value =
                                    self.lower_nullish_coalescing(field_value, default)?;
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
                let value = self.lower_nullish_coalescing(value, default)?;
                let value_type = self.infer_expr_type(&value)?;
                self.lower_assignment_pattern(&assign.left, value, &value_type, statements)
            }
            Pat::Rest(_) => Err("rest patterns are only valid inside object/array patterns".into()),
            _ => Err("unsupported destructuring assignment target".into()),
        }
    }

    fn lower_update(&mut self, update: &swc_ecma_ast::UpdateExpr) -> Result<HirExpr, String> {
        if let Expr::Member(member) = update.arg.as_ref() {
            if let (Expr::Ident(receiver), Some(property)) =
                (member.obj.as_ref(), member_property_name(&member.prop))
            {
                let getter = class_getter_symbol(receiver.sym.as_ref(), &property, true);
                if let Some(getter_signature) = self.signatures.get(&getter).cloned() {
                    let setter = class_setter_symbol(receiver.sym.as_ref(), &property, true);
                    if !self.signatures.contains_key(&setter) {
                        return Err(format!(
                            "cannot update readonly static member `{}.{}`",
                            receiver.sym, property
                        ));
                    }
                    if getter_signature.ret != HirType::F64 {
                        return Err(format!(
                            "cannot apply ++/-- to non-number static member `{}.{}`",
                            receiver.sym, property
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
                if let (Expr::Ident(class), Some(property)) =
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

    fn lower_call(&mut self, call: &CallExpr) -> Result<HirExpr, String> {
        if matches!(call.callee, Callee::Super(_)) {
            let (symbol, _base_type, _base_name) = self
                .super_initializer
                .clone()
                .ok_or("`super(...)` is only valid in a derived class constructor")?;
            let signature = self
                .signatures
                .get(&symbol)
                .cloned()
                .ok_or_else(|| format!("missing base class initializer `{symbol}`"))?;
            if call.type_args.is_some()
                || call.args.iter().any(|argument| argument.spread.is_some())
            {
                return Err(
                    "native `super(...)` does not support type or spread arguments yet".into(),
                );
            }
            if call.args.len() + 1 != signature.params.len() {
                return Err(format!(
                    "base constructor expects {} argument(s), got {}",
                    signature.params.len() - 1,
                    call.args.len()
                ));
            }
            let this_name = self.resolve_binding("this");
            let mut args = vec![HirExpr::Var(this_name)];
            for (index, argument) in call.args.iter().enumerate() {
                let value = self.lower_expr(&argument.expr)?;
                args.push(self.coerce_to_declared(&signature.params[index + 1], value)?);
            }
            return Ok(HirExpr::Call(Box::new(HirExpr::Var(symbol)), args));
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
                let symbol = if self.class_static_context {
                    class_static_method_symbol(&base_name, &method_name)
                } else {
                    class_method_symbol(&base_name, &method_name)
                };
                let signature = self.signatures.get(&symbol).cloned().ok_or_else(|| {
                    format!("base class `{base_name}` has no method `{method_name}`")
                })?;
                if call.type_args.is_some()
                    || call.args.iter().any(|argument| argument.spread.is_some())
                {
                    return Err(
                        "native super methods do not support type or spread arguments yet".into(),
                    );
                }
                let receiver_count = usize::from(!self.class_static_context);
                if call.args.len() + receiver_count != signature.params.len() {
                    return Err(format!(
                        "super method `{base_name}.{method_name}` expects {} argument(s), got {}",
                        signature.params.len() - receiver_count,
                        call.args.len()
                    ));
                }
                let mut args = if self.class_static_context {
                    Vec::new()
                } else {
                    vec![HirExpr::Var(self.resolve_binding("this"))]
                };
                for (index, argument) in call.args.iter().enumerate() {
                    let value = self.lower_expr(&argument.expr)?;
                    args.push(
                        self.coerce_to_declared(&signature.params[index + receiver_count], value)?,
                    );
                }
                return Ok(HirExpr::Call(Box::new(HirExpr::Var(symbol)), args));
            }
        }
        let Callee::Expr(callee_expr) = &call.callee else {
            return Err("unsupported callee (super/import calls not supported)".into());
        };

        if let Expr::Member(member) = callee_expr.as_ref() {
            if let Some(property) = member_property_name(&member.prop) {
                if let Expr::Ident(class) = member.obj.as_ref() {
                    let class_name = class.sym.as_ref();
                    let symbol = class_static_method_symbol(class_name, &property);
                    if let Some(signature) = self.signatures.get(&symbol).cloned() {
                        if call.type_args.is_some() {
                            return Err(format!(
                                "native static method `{class_name}.{property}` is not generic"
                            ));
                        }
                        if call.args.iter().any(|argument| argument.spread.is_some()) {
                            return Err(
                                "native static method spread arguments are not supported yet"
                                    .into(),
                            );
                        }
                        if call.args.len() != signature.params.len() {
                            return Err(format!(
                                "static method `{class_name}.{property}` expects {} argument(s), got {}",
                                signature.params.len(),
                                call.args.len()
                            ));
                        }
                        let mut args = Vec::with_capacity(call.args.len());
                        for (index, argument) in call.args.iter().enumerate() {
                            let value = self.lower_expr(&argument.expr)?;
                            args.push(self.coerce_to_declared(&signature.params[index], value)?);
                        }
                        return Ok(HirExpr::Call(Box::new(HirExpr::Var(symbol)), args));
                    }
                }
                let known_class_receiver = match member.obj.as_ref() {
                    Expr::Ident(receiver) => {
                        let name = self.resolve_binding(receiver.sym.as_ref());
                        self.scope
                            .get(&name)
                            .and_then(class_name_from_type)
                            .is_some()
                    }
                    Expr::New(construction) => {
                        matches!(construction.callee.as_ref(), Expr::Ident(class) if self.signatures.contains_key(&class_constructor_symbol(class.sym.as_ref())))
                    }
                    _ => false,
                };
                if known_class_receiver {
                    let receiver = self.lower_expr(&member.obj)?;
                    let receiver_type = self.infer_expr_type(&receiver)?;
                    let class_name = class_name_from_type(&receiver_type)
                        .expect("the receiver was classified as a native class");
                    let symbol = class_method_symbol(class_name, &property);
                    if let Some(signature) = self.signatures.get(&symbol).cloned() {
                        if call.type_args.is_some() {
                            return Err(format!(
                                "native class method `{class_name}.{property}` is not generic"
                            ));
                        }
                        if call.args.iter().any(|argument| argument.spread.is_some()) {
                            return Err(
                                "native class method spread arguments are not supported yet".into(),
                            );
                        }
                        if call.args.len() + 1 != signature.params.len() {
                            return Err(format!(
                                "method `{class_name}.{property}` expects {} argument(s), got {}",
                                signature.params.len() - 1,
                                call.args.len()
                            ));
                        }
                        let mut args = vec![receiver];
                        for (index, argument) in call.args.iter().enumerate() {
                            let value = self.lower_expr(&argument.expr)?;
                            args.push(
                                self.coerce_to_declared(&signature.params[index + 1], value)?,
                            );
                        }
                        return Ok(HirExpr::Call(Box::new(HirExpr::Var(symbol)), args));
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
                    let prefix = match &element_type {
                        HirType::F64 => "number",
                        HirType::Str => "string",
                        HirType::Bool => "bool",
                        HirType::Object(_) => "object",
                        other => {
                            return Err(format!(
                                "array sort does not support element type {other:?}"
                            ))
                        }
                    };
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
            if let Expr::Ident(object) = member.obj.as_ref() {
                let property = match &member.prop {
                    MemberProp::Ident(property) => Some(property.sym.to_string()),
                    MemberProp::Computed(computed) => match computed.expr.as_ref() {
                        Expr::Lit(Lit::Str(property)) => {
                            Some(property.value.to_string_lossy().into_owned())
                        }
                        _ => None,
                    },
                    _ => None,
                };
                if let Some(property) = property {
                    let object_name = self.resolve_binding(object.sym.as_ref());
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
                    let object_ty = self
                        .narrowings
                        .get(&object_name)
                        .cloned()
                        .or_else(|| self.nullable_narrowings.get(&object_name).cloned())
                        .or_else(|| self.nullish_narrowings.get(&object_name).cloned())
                        .or_else(|| self.scope.get(&object_name).cloned());
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
                                property,
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
                                        property
                                    )
                                })
                            })
                            .collect::<Result<Vec<_>, _>>()?;
                        return Ok(HirExpr::Call(Box::new(callee), args));
                    }
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
        if let Some(type_args) = &call.type_args {
            let signature = self.generic_arrow_signature(arrow)?;
            resolve_explicit_generic_type_tuple(
                &signature,
                &type_args.params,
                &parameter_types,
                self.interfaces,
                self.generic_interfaces,
            )
            .map_err(|error| {
                format!("cannot explicitly specialize generic arrow `{name}`: {error}")
            })?;
        }
        let lambda = self
            .lower_contextual_arrow(arrow, &parameter_types, None)
            .map_err(|error| format!("cannot specialize generic arrow `{name}`: {error}"))?;
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
            ret: HirType::Dynamic,
            is_async: false,
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
            ret: HirType::Dynamic,
            is_async: false,
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
            ret: HirType::Dynamic,
            is_async: false,
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
        }

        let callee = self.lower_expr(&call.callee)?;
        let callee_type = self.infer_expr_type(&callee)?;
        let (payload, absence_kind) = match callee_type.clone() {
            HirType::Optional(payload) => (payload, 0),
            HirType::Nullable(payload) => (payload, 1),
            HirType::Nullish(payload) => (payload, 2),
            _ => return self.lower_call(&CallExpr::from(call.clone())),
        };
        let HirType::Function(params, return_type) = payload.as_ref() else {
            return Err(format!(
                "optional call requires a function payload, got {payload:?}"
            ));
        };
        if call.type_args.is_some() {
            return Err("optional native calls do not accept type arguments".into());
        }
        if call.args.iter().any(|argument| argument.spread.is_some()) {
            return Err("optional native call spread arguments are not supported".into());
        }
        if params.len() != call.args.len() {
            return Err(format!(
                "optional function expects {} argument(s), got {}",
                params.len(),
                call.args.len()
            ));
        }
        let arguments = call
            .args
            .iter()
            .zip(params)
            .map(|(argument, expected)| {
                let value = self.lower_expr(&argument.expr)?;
                self.coerce_to_declared(expected, value)
            })
            .collect::<Result<Vec<_>, String>>()?;

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
        let result = if return_type.as_ref() == &HirType::Void {
            HirExpr::Block(vec![HirStmt::If(
                is_none(bound),
                vec![HirStmt::Return(Some(HirExpr::Lit(HirLit::Undefined)))],
                vec![
                    HirStmt::Expr(invoked),
                    HirStmt::Return(Some(HirExpr::Lit(HirLit::Undefined))),
                ],
            )])
        } else {
            let (result_payload, present) = match return_type.as_ref() {
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
    fn lowers_top_level_bindings_and_exposes_them_to_functions() {
        let program = lower(
            r#"
                const base = 40;
                let answer: number = base + 2;
                function read(): number { return answer; }
                function main(): void { console.log(read()); }
            "#,
        );
        assert_eq!(program.globals.len(), 2);
        assert_eq!(program.globals[0].name, "base");
        assert_eq!(program.globals[0].ty, HirType::F64);
        assert!(!program.globals[0].mutable);
        assert_eq!(program.globals[1].name, "answer");
        assert_eq!(program.globals[1].ty, HirType::F64);
        assert!(program.globals[1].mutable);
        let read = program
            .functions
            .iter()
            .find(|function| function.name == "read")
            .unwrap();
        assert!(matches!(
            &read.body[0],
            HirStmt::Return(Some(HirExpr::Var(name))) if name == "answer"
        ));
    }

    #[test]
    fn infers_top_level_initializer_calls_to_forward_functions() {
        let program = lower(
            r#"
                const answer = makeAnswer();
                function makeAnswer(): number { return 42; }
                function main(): void { console.log(answer); }
            "#,
        );
        assert_eq!(program.globals[0].ty, HirType::F64);
        assert!(matches!(program.globals[0].init, HirExpr::Call(_, _)));
    }

    #[test]
    fn rejects_top_level_const_reassignment() {
        let module = thaw_parser::parse_typescript(
            "const answer = 42; function main(): void { answer = 43; }",
        )
        .unwrap();
        let error = lower_module(&module).unwrap_err();
        assert_eq!(error, "cannot assign to constant `answer`");
    }

    #[test]
    fn rejects_direct_forward_references_between_top_level_bindings() {
        let module = thaw_parser::parse_typescript(
            "const answer: number = base + 2; const base = 40; function main(): void {}",
        )
        .unwrap();
        let error = lower_module(&module).unwrap_err();
        assert!(error.contains("unknown variable `base`"), "{error}");
    }

    #[test]
    fn preserves_top_level_executable_statements_in_initializer_order() {
        let program = lower(
            r#"
                let answer = 40;
                answer = answer + 1;
                if (true) { answer++; }
                function main(): void { console.log(answer); }
            "#,
        );
        assert_eq!(program.initializers.len(), 3);
        assert!(matches!(
            program.initializers[0],
            HirInitStep::StoreGlobal(ref name, _) if name == "answer"
        ));
        assert!(matches!(
            program.initializers[1],
            HirInitStep::Statement(HirStmt::Expr(HirExpr::Assign(ref name, _)))
                if name == "answer"
        ));
        assert!(matches!(
            program.initializers[2],
            HirInitStep::Statement(HirStmt::If(_, _, _))
        ));
    }

    #[test]
    fn top_level_calls_constrain_unannotated_function_parameters() {
        let program = lower(
            r#"
                function configure(value): void { console.log(value); }
                configure(42);
                function main(): void {}
            "#,
        );
        let configure = program
            .functions
            .iter()
            .find(|function| function.name == "configure")
            .unwrap();
        assert_eq!(configure.params[0].ty, HirType::F64);
    }

    #[test]
    fn expands_nested_top_level_object_and_array_destructuring() {
        let program = lower(
            r#"
                const { point: { x, y }, values: [first, second] } = {
                    point: { x: 40, y: 2 },
                    values: [20, 22]
                };
                function main(): void {
                    console.log(x + y);
                    console.log(first + second);
                }
            "#,
        );
        for name in ["x", "y", "first", "second"] {
            let global = program
                .globals
                .iter()
                .find(|global| global.name == name)
                .unwrap_or_else(|| panic!("missing destructured global `{name}`"));
            assert_eq!(global.ty, HirType::F64);
            assert!(!global.mutable);
        }
    }

    #[test]
    fn expands_top_level_destructuring_defaults_and_array_rest() {
        let program = lower(
            r#"
                interface Config { fallback: number | undefined; }
                const { fallback = 42 }: Config = { fallback: undefined };
                const [head, ...tail] = [20, 10, 12];
                const { answer, ...metadata } = { answer: 42, label: "ready", code: 2 };
                function main(): void {
                    console.log(fallback);
                    console.log(head + tail[0] + tail[1]);
                    console.log(metadata.label);
                }
            "#,
        );
        assert_eq!(
            program
                .globals
                .iter()
                .find(|global| global.name == "fallback")
                .unwrap()
                .ty,
            HirType::F64
        );
        assert_eq!(
            program
                .globals
                .iter()
                .find(|global| global.name == "tail")
                .unwrap()
                .ty,
            HirType::Array(Box::new(HirType::F64))
        );
        assert_eq!(
            program
                .globals
                .iter()
                .find(|global| global.name == "metadata")
                .unwrap()
                .ty,
            HirType::Object(vec![
                ("label".into(), HirType::Str),
                ("code".into(), HirType::F64),
            ])
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
    fn enforces_declared_generic_function_constraints() {
        let primitive = thaw_parser::parse_typescript(
            r#"
            function numeric<T extends number>(value: T): T { return value; }
            function main(): void { numeric("wrong"); }
            "#,
        )
        .unwrap();
        let error = lower_module(&primitive).unwrap_err();
        assert!(error.contains("does not satisfy constraint F64"), "{error}");

        let dependent = thaw_parser::parse_typescript(
            r#"
            function choose<T, U extends T>(left: T, right: U): U { return right; }
            function main(): void { choose(1, "wrong"); }
            "#,
        )
        .unwrap();
        let error = lower_module(&dependent).unwrap_err();
        assert!(error.contains("does not satisfy constraint F64"), "{error}");

        let structural = thaw_parser::parse_typescript(
            r#"
            function named<T extends { name: string }>(value: T): T { return value; }
            function main(): void { named({ value: 1 }); }
            "#,
        )
        .unwrap();
        let error = lower_module(&structural).unwrap_err();
        assert!(
            error.contains("does not satisfy constraint Object"),
            "{error}"
        );
    }

    #[test]
    fn validates_generic_function_defaults_against_constraints() {
        let module = thaw_parser::parse_typescript(
            r#"
            function invalid<T extends number = string>(): T { return "wrong"; }
            function main(): void { invalid(); }
            "#,
        )
        .unwrap();
        let error = lower_module(&module).unwrap_err();
        assert!(error.contains("does not satisfy constraint F64"), "{error}");
    }

    #[test]
    fn rejects_required_type_parameters_after_defaults() {
        for (source, kind) in [
            (
                "interface Invalid<T = string, U> { first: T; second: U } function main(): void {}",
                "generic interface",
            ),
            (
                "type Invalid<T = string, U> = { first: T; second: U }; function main(): void {}",
                "generic type alias",
            ),
            (
                "function invalid<T = string, U>(value: U): U { return value; } function main(): void { invalid(1); }",
                "generic function",
            ),
        ] {
            let module = thaw_parser::parse_typescript(source).unwrap();
            let error = lower_module(&module).unwrap_err();
            assert!(error.contains(kind), "{error}");
            assert!(error.contains("required type parameter `U`"), "{error}");
        }
    }

    #[test]
    fn validates_explicit_generic_function_type_arguments() {
        for (source, expected) in [
            (
                "function id<T>(value: T): T { return value; } function main(): void { id<string>(1); }",
                "explicit type is Str",
            ),
            (
                "function pair<T, U>(left: T, right: U): T { return left; } function main(): void { pair<number>(1, 2); }",
                "expects 2 explicit type argument",
            ),
            (
                "function numeric<T extends number>(value: T): T { return value; } function main(): void { numeric<string>(\"x\"); }",
                "does not satisfy constraint F64",
            ),
            (
                "function plain(value: number): number { return value; } function main(): void { plain<number>(1); }",
                "non-generic function `plain`",
            ),
        ] {
            let module = thaw_parser::parse_typescript(source).unwrap();
            let error = lower_module(&module).unwrap_err();
            assert!(error.contains(expected), "{error}");
        }
    }

    #[test]
    fn validates_contextually_specialized_generic_callbacks() {
        let module = thaw_parser::parse_typescript(
            r#"
            function numeric<T extends number>(value: T): T { return value; }
            function main(): void { ["wrong"].map(numeric); }
            "#,
        )
        .unwrap();
        let error = lower_module(&module).unwrap_err();
        assert!(
            error.contains("cannot specialize generic callback `numeric`"),
            "{error}"
        );
        assert!(error.contains("does not satisfy constraint F64"), "{error}");
    }

    #[test]
    fn validates_contextual_generic_arrow_constraints() {
        let module = thaw_parser::parse_typescript(
            r#"
            function main(): void {
                ["wrong"].map(<T extends number>(value: T): T => value);
            }
            "#,
        )
        .unwrap();
        let error = lower_module(&module).unwrap_err();
        assert!(error.contains("does not satisfy constraint F64"), "{error}");
    }

    #[test]
    fn validates_generic_instantiation_expressions() {
        for (source, expected) in [
            (
                "function numeric<T extends number>(value: T): T { return value; } function main(): void { const bad = numeric<string>; }",
                "does not satisfy constraint F64",
            ),
            (
                "function plain(value: number): number { return value; } function main(): void { const bad = plain<number>; }",
                "non-generic function `plain`",
            ),
        ] {
            let module = thaw_parser::parse_typescript(source).unwrap();
            let error = lower_module(&module).unwrap_err();
            assert!(error.contains(expected), "{error}");
        }
    }

    #[test]
    fn validates_annotated_generic_arrow_constraints() {
        let module = thaw_parser::parse_typescript(
            r#"
            function main(): void {
                const invalid: (value: string) => string =
                    <T extends number>(value: T): T => value;
            }
            "#,
        )
        .unwrap();
        let error = lower_module(&module).unwrap_err();
        assert!(error.contains("does not satisfy constraint F64"), "{error}");
    }

    #[test]
    fn validates_local_generic_arrow_calls() {
        for (source, expected) in [
            (
                "function main(): void { const numeric = <T extends number>(value: T): T => value; numeric(\"wrong\"); }",
                "does not satisfy constraint F64",
            ),
            (
                "function main(): void { const pair = <T, U>(left: T, right: U): T => left; pair(1); }",
                "expects 2 argument(s), got 1",
            ),
            (
                "function main(): void { const identity = <T>(value: T): T => value; identity<string>(1); }",
                "explicit type is Str",
            ),
        ] {
            let module = thaw_parser::parse_typescript(source).unwrap();
            let error = lower_module(&module).unwrap_err();
            assert!(error.contains(expected), "{error}");
        }
    }

    #[test]
    fn validates_user_function_generic_callback_constraints() {
        let module = thaw_parser::parse_typescript(
            r#"
            function apply(callback: (value: string) => string, value: string): string {
                return callback(value);
            }
            function numeric<T extends number>(value: T): T { return value; }
            function main(): void { apply(numeric, "wrong"); }
            "#,
        )
        .unwrap();
        let error = lower_module(&module).unwrap_err();
        assert!(
            error.contains("cannot specialize generic callback `numeric`"),
            "{error}"
        );
        assert!(error.contains("does not satisfy constraint F64"), "{error}");
    }

    #[test]
    fn validates_generic_function_type_alias_assignments() {
        for (source, expected) in [
            (
                "type Identity = <T>(value: T) => T; function main(): void { const bad: Identity = <U>(value: U): string => String(value); }",
                "incompatible with function type alias `Identity`",
            ),
            (
                "type Numeric = <T extends number>(value: T) => T; function main(): void { const bad: Numeric = <U extends string>(value: U): U => value; }",
                "constraints do not match function type alias `Numeric`",
            ),
            (
                "type Identity = <T>(value: T) => T; function bad<T>(value: T): string { return \"wrong\"; } function main(): void { const invalid: Identity = bad; }",
                "incompatible with function type alias `Identity`",
            ),
            (
                "type Identity = <T>(value: T) => T; type Stringify = <T>(value: T) => string; function main(): void { const identity: Identity = <T>(value: T): T => value; const invalid: Stringify = identity; }",
                "incompatible with function type alias `Stringify`",
            ),
            (
                "type Forward = Stringify; type Stringify = <T>(value: T) => string; function main(): void { const invalid: Forward = <T>(value: T): T => value; }",
                "incompatible with function type alias `Forward`",
            ),
            (
                "type Stringify = { <T>(value: T): string }; function main(): void { const invalid: Stringify = <T>(value: T): T => value; }",
                "incompatible with function type alias `Stringify`",
            ),
            (
                "type Identity = <T>(value: T) => T; function main(): void { const invalid: Identity = function<T>(value: T): string { return String(value); }; }",
                "incompatible with function type alias `Identity`",
            ),
            (
                "type Factory = <T = string>() => T; function main(): void { const invalid: Factory = <T = number>(): T => 1; }",
                "defaults do not match function type alias `Factory`",
            ),
            (
                "type Identity = <T>(value: T) => T; function main(): void { const invalid: Identity = <T>(value: T) => String(value); }",
                "incompatible with function type alias `Identity`",
            ),
            (
                "type Stringify = <T>(value: T) => string; function main(): void { const invalid: Stringify = <T>(value: T) => 1; }",
                "incompatible with function type alias `Stringify`",
            ),
            (
                "type Nullify = <T>(value: T) => null; function main(): void { const invalid: Nullify = <T>(value: T) => undefined; }",
                "incompatible with function type alias `Nullify`",
            ),
            (
                "type Identity = <T>(value: T) => T; function main(): void { const invalid: Identity = <T>(value: T) => \"value=\" + String(value); }",
                "incompatible with function type alias `Identity`",
            ),
            (
                "type Predicate = <T>(value: T, flag: boolean) => boolean; function main(): void { const invalid: Predicate = <T>(value: T, flag: boolean) => flag && \"wrong\"; }",
                "needs an explicit return type",
            ),
            (
                "type OptionalIdentity = <T>(value?: T) => T; function main(): void { const invalid: OptionalIdentity = <T>(value: T): T => value; }",
                "optional parameters do not match function type alias `OptionalIdentity`",
            ),
            (
                "type Choose = <T, U>(left: T, right: U) => T; function main(): void { const invalid: Choose = function<T, U>(left: T, right: U) { if (true) return left; return right; }; }",
                "needs an explicit return type",
            ),
            (
                "type Identity = <T>(value: T) => T; async function asynchronous<T>(value: T): Promise<T> { return value; } function main(): void { const invalid: Identity = asynchronous; }",
                "incompatible with function type alias `Identity`",
            ),
        ] {
            let module = thaw_parser::parse_typescript(source).unwrap();
            let error = lower_module(&module).unwrap_err();
            assert!(error.contains(expected), "{error}");
        }
    }

    #[test]
    fn validates_generic_callable_interface_assignments() {
        for (source, name) in [
            (
                "interface Identity { <T>(value: T): T; } function main(): void { const invalid: Identity = <U>(value: U): string => \"wrong\"; }",
                "Identity",
            ),
            (
                "interface Derived extends Middle {} interface Middle extends Identity {} interface Identity { <T>(value: T): T; } function main(): void { const invalid: Derived = <U>(value: U): string => \"wrong\"; }",
                "Derived",
            ),
        ] {
            let module = thaw_parser::parse_typescript(source).unwrap();
            let error = lower_module(&module).unwrap_err();
            assert!(
                error.contains(&format!(
                    "incompatible with function type alias `{name}`"
                )),
                "{error}"
            );
        }
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
    fn dynamic_computed_object_reads_require_uniform_fields() {
        let module = thaw_parser::parse_typescript(
            r#"function read(key: string): number | undefined {
                const mixed = { value: 1, label: "one" };
                return mixed[key];
            }"#,
        )
        .unwrap();
        assert!(lower_module(&module)
            .unwrap_err()
            .contains("requires every field to have the same type"));
    }

    #[test]
    fn dynamic_computed_object_reads_flatten_uniform_tagged_fields() {
        let program = lower(
            r#"function read(
                optional: { a: number | undefined; b: number | undefined },
                nullable: { a: number | null; b: number | null },
                nullish: { a: number | null | undefined; b: number | null | undefined },
                key: string
            ): void {
                console.log(optional[key]);
                console.log(nullable[key]);
                console.log(nullish[key]);
            }"#,
        );
        let body = &program.functions[0].body;
        assert!(matches!(
            &body[0],
            HirStmt::Expr(HirExpr::Call(_, args))
                if matches!(&args[0], HirExpr::DynamicPropAccess(_, _, _, HirType::Optional(inner)) if inner.as_ref() == &HirType::F64)
        ));
        assert!(matches!(
            &body[1],
            HirStmt::Expr(HirExpr::Call(_, args))
                if matches!(&args[0], HirExpr::DynamicPropAccess(_, _, _, HirType::Nullish(inner)) if inner.as_ref() == &HirType::F64)
        ));
        assert!(matches!(
            &body[2],
            HirStmt::Expr(HirExpr::Call(_, args))
                if matches!(&args[0], HirExpr::DynamicPropAccess(_, _, _, HirType::Nullish(inner)) if inner.as_ref() == &HirType::F64)
        ));
    }

    #[test]
    fn lowers_numeric_and_string_enum_members_declared_after_functions() {
        let program = lower(
            r#"function value(direction: Direction): number {
                return direction + Direction.Next;
            }
            function main(): void {
                console.log(value(Direction.None));
                console.log(Direction["Mask"]);
                console.log(Label.Alias);
            }
            enum Direction { None, Up = 4, Next = Up + 2, Mask = 1 << 3 }
            enum Label { Ready = "ready", Alias = Ready }"#,
        );
        let value = program
            .functions
            .iter()
            .find(|function| function.name == "value")
            .unwrap();
        assert_eq!(value.params[0].ty, HirType::F64);
        assert!(matches!(
            &value.body[0],
            HirStmt::Return(Some(HirExpr::BinOp(BinOp::Add, _, right)))
                if matches!(right.as_ref(), HirExpr::Lit(HirLit::F64(6.0)))
        ));
    }

    #[test]
    fn rejects_invalid_enum_native_layouts_and_members() {
        for (source, expected) in [
            (
                r#"enum Mixed { Number = 1, Text = "text" }
                   function main(): void {}"#,
                "mixes numeric and string members",
            ),
            (
                r#"enum Text { First = "first", Second }
                   function main(): void {}"#,
                "needs an initializer after a string member",
            ),
            (
                r#"enum Value { Present = 1 }
                   function main(): void { console.log(Value.Missing); }"#,
                "has no member `Missing`",
            ),
            (
                r#"enum Text { Present = "present" }
                   function main(): void { console.log(Text[0]); }"#,
                "does not support numeric reverse lookup",
            ),
        ] {
            let module = thaw_parser::parse_typescript(source).unwrap();
            let error = lower_module(&module).unwrap_err();
            assert!(error.contains(expected), "{error}");
        }
    }

    #[test]
    fn lowers_runtime_numeric_enum_reverse_lookup_as_optional_string() {
        let program = lower(
            r#"enum Status { Idle, Ready = 4, Alias = 4 }
               function read(index: number): string | undefined {
                   return Status[index];
               }"#,
        );
        assert!(matches!(
            &program.functions[0].body[0],
            HirStmt::Return(Some(HirExpr::EnumReverseLookup(index, entries)))
                if matches!(index.as_ref(), HirExpr::Var(name) if name == "index")
                    && entries == &vec![(0.0, "Idle".into()), (4.0, "Alias".into())]
        ));
    }

    #[test]
    fn merges_compatible_enum_declarations_in_source_order() {
        let program = lower(
            r#"enum Status {}
               enum Status { First }
               enum Status { Second }
               enum Status { Third = 2 }
               function read(index: number): string | undefined {
                   return Status[index];
               }"#,
        );
        assert!(matches!(
            &program.functions[0].body[0],
            HirStmt::Return(Some(HirExpr::EnumReverseLookup(_, entries)))
                if entries == &vec![(0.0, "Second".into()), (2.0, "Third".into())]
        ));

        for (source, expected) in [
            (
                r#"enum Value { First = 1 }
                   enum Value { First = 2 }
                   function main(): void {}"#,
                "duplicate member `First`",
            ),
            (
                r#"enum Value { First = 1 }
                   enum Value { Text = "text" }
                   function main(): void {}"#,
                "mixes numeric and string members",
            ),
        ] {
            let module = thaw_parser::parse_typescript(source).unwrap();
            let error = lower_module(&module).unwrap_err();
            assert!(error.contains(expected), "{error}");
        }
    }

    #[test]
    fn normalizes_same_layout_literal_unions_and_intersections() {
        let program = lower(
            r#"function text(value: "start" | "stop"): string { return value; }
               function numberValue(value: 1 | 2 | number): number { return value; }
               function flag(value: true | false): boolean { return value; }
               function intersection(value: string & "fixed"): string { return value; }
               function objectValue(value: { kind: "ready" | "waiting" }): string {
                   return value.kind;
               }
               function values(input: ("a" | "b")[]): string[] { return input; }"#,
        );
        assert_eq!(program.functions[0].params[0].ty, HirType::Str);
        assert_eq!(program.functions[1].params[0].ty, HirType::F64);
        assert_eq!(program.functions[2].params[0].ty, HirType::Bool);
        assert_eq!(program.functions[3].params[0].ty, HirType::Str);
        assert!(matches!(
            &program.functions[4].params[0].ty,
            HirType::Object(fields) if fields == &vec![("kind".into(), HirType::Str)]
        ));
        assert_eq!(
            program.functions[5].params[0].ty,
            HirType::Array(Box::new(HirType::Str))
        );

        let module =
            thaw_parser::parse_typescript(r#"function mixed(value: string | number): void {}"#)
                .unwrap();
        let mixed = lower_module(&module).unwrap();
        assert_eq!(
            mixed.functions[0].params[0].ty,
            HirType::Union(vec![HirType::Str, HirType::F64])
        );
    }

    #[test]
    fn lowers_heterogeneous_unions_to_tagged_injections() {
        let program = lower(
            r#"function identity(value: string | number): string | number { return value; }
               function kind(value: string | number): string { return typeof value; }
               function main(): void {
                   const first: string | number = "text";
                   const second: string | number = 2;
                   kind(identity(first));
                   kind(identity(second));
               }"#,
        );
        let union = HirType::Union(vec![HirType::Str, HirType::F64]);
        assert_eq!(program.functions[0].params[0].ty, union);
        assert_eq!(program.functions[0].ret, union);
        assert!(matches!(
            &program.functions[2].body[0],
            HirStmt::Let(_, ty, HirExpr::UnionInject(value, 0, members))
                if ty == &union
                    && members == &vec![HirType::Str, HirType::F64]
                    && matches!(value.as_ref(), HirExpr::Lit(HirLit::Str(_)))
        ));
        assert!(matches!(
            &program.functions[2].body[1],
            HirStmt::Let(_, ty, HirExpr::UnionInject(value, 1, members))
                if ty == &union
                    && members == &vec![HirType::Str, HirType::F64]
                    && matches!(value.as_ref(), HirExpr::Lit(HirLit::F64(2.0)))
        ));
    }

    #[test]
    fn resolves_forward_type_aliases_and_rejects_cycles() {
        let program = lower(
            r#"type Later = Base & { count: number };
               type Base = { name: string };
               type Choice = string | number;
               function choose(value: Choice): Later {
                   return { count: 2, name: "alias" };
               }"#,
        );
        assert_eq!(
            program.functions[0].params[0].ty,
            HirType::Union(vec![HirType::Str, HirType::F64])
        );
        assert_eq!(
            program.functions[0].ret,
            HirType::Object(vec![
                ("name".into(), HirType::Str),
                ("count".into(), HirType::F64),
            ])
        );

        let module = thaw_parser::parse_typescript("type A = B; type B = A;").unwrap();
        assert!(lower_module(&module)
            .unwrap_err()
            .contains("type declaration cycle"));

        let mixed = lower(
            r#"interface Item { label: Label; count: Count }
               type Count = number;
               type Label = string;
               function item(value: Item): string { return value.label; }"#,
        );
        assert_eq!(
            mixed.functions[0].params[0].ty,
            HirType::Object(vec![
                ("label".into(), HirType::Str),
                ("count".into(), HirType::F64),
            ])
        );

        let module =
            thaw_parser::parse_typescript("type Link = Node; interface Node { next: Link; }")
                .unwrap();
        assert!(lower_module(&module)
            .unwrap_err()
            .contains("self-referential"));
    }

    #[test]
    fn validates_generic_type_alias_instantiations() {
        let module = thaw_parser::parse_typescript(
            "type Boxed<T> = { value: T }; function bad(value: Boxed<number, string>): void {}",
        )
        .unwrap();
        assert!(lower_module(&module)
            .unwrap_err()
            .contains("expects 1 type argument(s), got 2"));

        let module = thaw_parser::parse_typescript(
            "type Loop<T> = Loop<T>; function bad(value: Loop<number>): void {}",
        )
        .unwrap();
        assert!(lower_module(&module)
            .unwrap_err()
            .contains("generic type alias `Loop` is (indirectly) self-referential"));

        let module = thaw_parser::parse_typescript(
            "type Numeric<T extends number> = { value: T }; function bad(value: Numeric<string>): void {}",
        )
        .unwrap();
        assert!(lower_module(&module)
            .unwrap_err()
            .contains("does not satisfy constraint F64"));
    }

    #[test]
    fn validates_generic_interface_defaults_and_constraints() {
        let module = thaw_parser::parse_typescript(
            "interface Numeric<T extends number> { value: T } function bad(value: Numeric<string>): void {}",
        )
        .unwrap();
        assert!(lower_module(&module)
            .unwrap_err()
            .contains("does not satisfy constraint F64"));
    }

    #[test]
    fn validates_generic_interface_inherited_field_collisions() {
        let module = thaw_parser::parse_typescript(
            "interface Base<T> { value: T } interface Child<T> extends Base<T> { value: T } function bad(value: Child<number>): void {}",
        )
        .unwrap();
        assert!(lower_module(&module)
            .unwrap_err()
            .contains("collides with an inherited field"));
    }

    #[test]
    fn validates_concrete_generic_base_constraints() {
        let module = thaw_parser::parse_typescript(
            "interface Bad extends Numeric<string> {} interface Numeric<T extends number> { value: T } function bad(value: Bad): void {}",
        )
        .unwrap();
        assert!(lower_module(&module)
            .unwrap_err()
            .contains("does not satisfy constraint F64"));
    }

    #[test]
    fn rejects_non_object_generic_alias_base() {
        let module = thaw_parser::parse_typescript(
            "type Value<T> = T; interface Bad extends Value<number> {} function bad(value: Bad): void {}",
        )
        .unwrap();
        assert!(lower_module(&module)
            .unwrap_err()
            .contains("can only extend object-shaped"));
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
                &EnumValues::new(),
                &EnumReverseValues::new(),
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
                variadic: None,
                variadic_abi: crate::FfiVariadicAbi::Native,
                ret: HirType::F64,
                error_abi: crate::FfiErrorAbi::Direct,
                return_ownership: crate::FfiOwnership::Borrowed,
                error_ownership: crate::FfiOwnership::Borrowed,
                param_string_abis: vec![crate::FfiStringAbi::NullTerminated; 2],
                return_string_abi: crate::FfiStringAbi::NullTerminated,
                calling_convention: crate::FfiCallingConvention::C,
                aggregate_return_abi: crate::FfiAggregateAbi::Internal,
                aggregate_return_layout: None,
            }]
        );

        let main = &program.functions[0];
        assert_eq!(
            main.body[0],
            HirStmt::Expr(HirExpr::Call(
                Box::new(HirExpr::Var("console.log".into())),
                vec![HirExpr::FfiCall(
                    Box::new(crate::FfiSignature {
                        symbol: "native_add".into(),
                        params: vec![HirType::F64, HirType::F64],
                        variadic: None,
                        variadic_abi: crate::FfiVariadicAbi::Native,
                        ret: HirType::F64,
                        error_abi: crate::FfiErrorAbi::Direct,
                        return_ownership: crate::FfiOwnership::Borrowed,
                        error_ownership: crate::FfiOwnership::Borrowed,
                        param_string_abis: vec![crate::FfiStringAbi::NullTerminated; 2],
                        return_string_abi: crate::FfiStringAbi::NullTerminated,
                        calling_convention: crate::FfiCallingConvention::C,
                        aggregate_return_abi: crate::FfiAggregateAbi::Internal,
                        aggregate_return_layout: None,
                    }),
                    vec![
                        HirExpr::Lit(HirLit::F64(2.0)),
                        HirExpr::Lit(HirLit::F64(3.0)),
                    ],
                )],
            ))
        );
    }

    #[test]
    fn rejects_unsupported_ambient_variadic_element_types() {
        let module = thaw_parser::parse_typescript(
            r#"declare function native_merge(...values: (boolean | undefined)[][]): number;
               function main(): void { console.log(native_merge([true])); }"#,
        )
        .unwrap();
        let error = lower_module(&module).unwrap_err();
        assert!(error.contains("unsupported rest element layout"), "{error}");
    }

    #[test]
    fn variadic_ambient_calls_still_require_every_fixed_argument() {
        let module = thaw_parser::parse_typescript(
            r#"declare function native_sum(count: number, ...values: number[]): number;
               function main(): void { console.log(native_sum()); }"#,
        )
        .unwrap();
        let error = lower_module(&module).unwrap_err();
        assert!(
            error.contains("expects at least 1 argument(s), got 0"),
            "{error}"
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

    #[test]
    fn lowers_native_class_construction_and_this_field_initialization() {
        let program = lower(
            r#"class Counter {
                value: number = 1;
                label: string;
                constructor(value: number, label: string) {
                    this.value = value;
                    this.label = label;
                }
            }
            function main(): number {
                const counter = new Counter(42, "ready");
                return counter.value;
            }"#,
        );
        let constructor = program
            .functions
            .iter()
            .find(|function| function.name == "__thaw_class_Counter_constructor")
            .expect("native class constructor");
        assert_eq!(constructor.params.len(), 2);
        assert!(matches!(
            &constructor.body[0],
            HirStmt::Let(name, HirType::Object(fields), HirExpr::ObjectAlloc(_))
                if name == "__thaw_this"
                    && fields.iter().any(|(name, ty)| name == "value" && ty == &HirType::F64)
                    && fields.iter().any(|(name, ty)| name == "label" && ty == &HirType::Str)
        ));
        let initializer = program
            .functions
            .iter()
            .find(|function| function.name == "__thaw_class_Counter_initialize")
            .expect("native class initializer");
        assert_eq!(initializer.params[0].name, "__thaw_this");
        assert!(matches!(
            constructor.body.last(),
            Some(HirStmt::Return(Some(HirExpr::Call(callee, args))))
                if matches!(callee.as_ref(), HirExpr::Var(name) if name == "__thaw_class_Counter_initialize")
                    && matches!(args.first(), Some(HirExpr::Var(name)) if name == "__thaw_this")
        ));
        assert_eq!(
            initializer
                .body
                .iter()
                .filter(|statement| matches!(statement, HirStmt::Expr(HirExpr::PropAssign(..))))
                .count(),
            3
        );
        let main = program
            .functions
            .iter()
            .find(|function| function.name == "main")
            .unwrap();
        assert!(matches!(
            &main.body[0],
            HirStmt::Let(_, HirType::Object(_), HirExpr::Call(callee, _))
                if matches!(callee.as_ref(), HirExpr::Var(name) if name == "__thaw_class_Counter_constructor")
        ));
    }

    #[test]
    fn lowers_native_class_instance_methods_with_explicit_receiver() {
        let program = lower(
            r#"class Counter {
                value: number;
                constructor(value: number) { this.value = value; }
                add(delta: number): number {
                    this.value += delta;
                    return this.value;
                }
            }
            function main(): number {
                const counter = new Counter(40);
                return counter.add(2);
            }"#,
        );
        let method = program
            .functions
            .iter()
            .find(|function| function.name == "__thaw_class_Counter_method_add")
            .expect("native class method");
        assert_eq!(method.params[0].name, "__thaw_this");
        assert!(matches!(method.params[0].ty, HirType::Object(_)));
        let main = program
            .functions
            .iter()
            .find(|function| function.name == "main")
            .unwrap();
        assert!(format!("{:?}", main.body).contains(
            "Call(Var(\"__thaw_class_Counter_method_add\"), [Var(\"counter\"), Lit(F64(2.0))])"
        ));
    }

    #[test]
    fn lowers_native_class_static_methods_without_a_receiver() {
        let program = lower(
            r#"class MathBox {
                static add(left: number, right: number): number { return left + right; }
            }
            function main(): number { return MathBox.add(40, 2); }"#,
        );
        let method = program
            .functions
            .iter()
            .find(|function| function.name == "__thaw_class_MathBox_static_add")
            .expect("native static method");
        assert_eq!(method.params.len(), 2);
        let main = program
            .functions
            .iter()
            .find(|function| function.name == "main")
            .unwrap();
        assert!(format!("{:?}", main.body).contains(
            "Call(Var(\"__thaw_class_MathBox_static_add\"), [Lit(F64(40.0)), Lit(F64(2.0))])"
        ));
    }

    #[test]
    fn lowers_native_class_instance_and_static_getters() {
        let program = lower(
            r#"class Box {
                value: number;
                constructor(value: number) { this.value = value; }
                get doubled(): number { return this.value * 2; }
                static get version(): string { return "v1"; }
            }
            function main(): number {
                const box = new Box(21);
                console.log(Box.version);
                return box.doubled;
            }"#,
        );
        assert!(program
            .functions
            .iter()
            .any(|function| function.name == "__thaw_class_Box_instance_getter_doubled"));
        assert!(program
            .functions
            .iter()
            .any(|function| function.name == "__thaw_class_Box_static_getter_version"));
        let main = program
            .functions
            .iter()
            .find(|function| function.name == "main")
            .unwrap();
        let body = format!("{:?}", main.body);
        assert!(body.contains("__thaw_class_Box_static_getter_version"));
        assert!(body.contains("__thaw_class_Box_instance_getter_doubled"));
    }

    #[test]
    fn lowers_native_class_setters_and_preserves_assignment_values() {
        let program = lower(
            r#"let version: number = 0;
            class Box {
                stored: number;
                constructor(value: number) { this.stored = value; }
                set value(next: number) { this.stored = next; }
                static set current(next: number) { version = next; }
            }
            function main(): number {
                const box = new Box(1);
                const assigned = (box.value = 40);
                const selected = (Box.current = 2);
                return assigned + selected + version;
            }"#,
        );
        assert!(program
            .functions
            .iter()
            .any(|function| function.name == "__thaw_class_Box_instance_setter_value"));
        assert!(program
            .functions
            .iter()
            .any(|function| function.name == "__thaw_class_Box_static_setter_current"));
        let main = program
            .functions
            .iter()
            .find(|function| function.name == "main")
            .unwrap();
        let body = format!("{:?}", main.body);
        assert!(body.contains("__thaw_class_Box_instance_setter_value"));
        assert!(body.contains("__thaw_class_Box_static_setter_current"));
        let setter = program
            .functions
            .iter()
            .find(|function| function.name == "__thaw_class_Box_instance_setter_value")
            .unwrap();
        assert_eq!(setter.ret, HirType::F64);
        assert!(matches!(
            setter.body.last(),
            Some(HirStmt::Return(Some(HirExpr::Var(name)))) if name == "next"
        ));
    }

    #[test]
    fn lowers_constructor_parameter_properties_as_instance_fields() {
        let program = lower(
            r#"class Point {
                constructor(public x: number, readonly label: string) {}
                sum(y: number): number { return this.x + y; }
            }
            function main(): number {
                const point = new Point(40, "ready");
                console.log(point.label);
                return point.sum(2);
            }"#,
        );
        let constructor = program
            .functions
            .iter()
            .find(|function| function.name == "__thaw_class_Point_constructor")
            .unwrap();
        let HirType::Object(fields) = &constructor.ret else {
            panic!("class layout")
        };
        assert!(fields
            .iter()
            .any(|(name, ty)| name == "x" && ty == &HirType::F64));
        assert!(fields
            .iter()
            .any(|(name, ty)| name == "label" && ty == &HirType::Str));
        let initializer = program
            .functions
            .iter()
            .find(|function| function.name == "__thaw_class_Point_initialize")
            .unwrap();
        assert_eq!(
            initializer
                .body
                .iter()
                .filter(|statement| matches!(statement, HirStmt::Expr(HirExpr::PropAssign(..))))
                .count(),
            2
        );
    }

    #[test]
    fn builds_forward_class_inheritance_layouts_in_base_to_derived_order() {
        let program = lower(
            r#"class Derived extends Base {
                label: string;
                read(): number { return this.value; }
            }
            class Base { value: number; }
            function main(): number {
                const value = new Derived();
                return value.read();
            }"#,
        );
        let constructor = program
            .functions
            .iter()
            .find(|function| function.name == "__thaw_class_Derived_constructor")
            .unwrap();
        let HirType::Object(fields) = &constructor.ret else {
            panic!("derived layout")
        };
        assert_eq!(
            fields,
            &vec![
                ("__thaw_class_identity_Derived".into(), HirType::Bool),
                ("value".into(), HirType::F64),
                ("label".into(), HirType::Str),
            ]
        );
    }

    #[test]
    fn rejects_class_inheritance_cycles_and_field_collisions() {
        let cycle = thaw_parser::parse_typescript(
            "class First extends Second {} class Second extends First {}",
        )
        .unwrap();
        assert!(lower_module(&cycle)
            .unwrap_err()
            .contains("class inheritance cycle"));

        let collision = thaw_parser::parse_typescript(
            "class Base { value: number; } class Derived extends Base { value: number; }",
        )
        .unwrap();
        assert!(lower_module(&collision)
            .unwrap_err()
            .contains("collides with an inherited or local field"));
    }

    #[test]
    fn lowers_super_to_the_base_initializer_on_the_same_instance() {
        let program = lower(
            r#"class Derived extends Base {
                label: string = "ready";
                constructor(value: number) { super(value); }
                answer(): number { return this.value; }
            }
            class Base { constructor(public value: number) {} }
            function main(): number { return new Derived(42).answer(); }"#,
        );
        let initializer = program
            .functions
            .iter()
            .find(|function| function.name == "__thaw_class_Derived_initialize")
            .unwrap();
        assert!(matches!(
            &initializer.body[0],
            HirStmt::Expr(HirExpr::Call(callee, args))
                if matches!(callee.as_ref(), HirExpr::Var(name) if name == "__thaw_class_Base_initialize")
                    && matches!(args.first(), Some(HirExpr::Var(name)) if name == "__thaw_this")
        ));
        assert!(matches!(
            &initializer.body[1],
            HirStmt::Expr(HirExpr::PropAssign(_, _, field, _)) if field == "label"
        ));
    }

    #[test]
    fn generates_typed_inherited_member_wrappers_and_prefers_overrides() {
        let program = lower(
            r#"class Base {
                constructor(public value: number) {}
                answer(): number { return this.value; }
                get doubled(): number { return this.value * 2; }
                set current(next: number) { this.value = next; }
            }
            class Derived extends Base {
                constructor(value: number) { super(value); }
                answer(): number { return this.value + 1; }
            }
            function main(): number {
                const value = new Derived(20);
                value.current = 21;
                console.log(value.doubled);
                return value.answer();
            }"#,
        );
        for symbol in [
            "__thaw_class_Derived_instance_getter_doubled",
            "__thaw_class_Derived_instance_setter_current",
        ] {
            assert!(program
                .functions
                .iter()
                .any(|function| function.name == symbol));
        }
        let answer = program
            .functions
            .iter()
            .filter(|function| function.name == "__thaw_class_Derived_method_answer")
            .collect::<Vec<_>>();
        assert_eq!(answer.len(), 1, "override must suppress inherited wrapper");
    }

    #[test]
    fn lowers_super_method_calls_to_the_direct_base_implementation() {
        let program = lower(
            r#"class Base {
                constructor(public value: number) {}
                answer(delta: number): number { return this.value + delta; }
            }
            class Derived extends Base {
                constructor(value: number) { super(value); }
                answer(delta: number): number { return super.answer(delta) + 1; }
            }
            function main(): number { return new Derived(40).answer(1); }"#,
        );
        let method = program
            .functions
            .iter()
            .find(|function| function.name == "__thaw_class_Derived_method_answer")
            .unwrap();
        assert!(format!("{:?}", method.body).contains(
            "Call(Var(\"__thaw_class_Base_method_answer\"), [Var(\"__thaw_this\"), Var(\"delta\")])"
        ));
    }

    #[test]
    fn lowers_super_getter_and_setter_access_to_base_accessors() {
        let program = lower(
            r#"class Base {
                stored: number;
                constructor(value: number) { this.stored = value; }
                get value(): number { return this.stored; }
                set value(next: number) { this.stored = next; }
            }
            class Derived extends Base {
                constructor(value: number) { super(value); }
                get value(): number { return super.value + 1; }
                set value(next: number) { super.value = next + 1; }
            }
            function main(): number {
                const value = new Derived(1);
                value.value = 20;
                return value.value;
            }"#,
        );
        let getter = program
            .functions
            .iter()
            .find(|function| function.name == "__thaw_class_Derived_instance_getter_value")
            .unwrap();
        assert!(format!("{:?}", getter.body).contains("__thaw_class_Base_instance_getter_value"));
        let setter = program
            .functions
            .iter()
            .find(|function| function.name == "__thaw_class_Derived_instance_setter_value")
            .unwrap();
        assert!(format!("{:?}", setter.body).contains("__thaw_class_Base_instance_setter_value"));
    }

    #[test]
    fn inherits_static_members_and_prefers_static_overrides() {
        let program = lower(
            r#"let stored: number = 0;
            class Base {
                static add(left: number, right: number): number { return left + right; }
                static get current(): number { return stored; }
                static set current(next: number) { stored = next; }
            }
            class Derived extends Base {
                static add(left: number, right: number): number { return left + right + 1; }
            }
            function main(): number {
                Derived.current = 40;
                return Derived.current + Derived.add(1, 1);
            }"#,
        );
        for symbol in [
            "__thaw_class_Derived_static_getter_current",
            "__thaw_class_Derived_static_setter_current",
        ] {
            assert!(program
                .functions
                .iter()
                .any(|function| function.name == symbol));
        }
        assert_eq!(
            program
                .functions
                .iter()
                .filter(|function| function.name == "__thaw_class_Derived_static_add")
                .count(),
            1
        );
    }

    #[test]
    fn lowers_static_super_methods_getters_and_setters() {
        let program = lower(
            r#"let stored: number = 0;
            class Base {
                static add(value: number): number { return value + 1; }
                static get current(): number { return stored; }
                static set current(next: number) { stored = next; }
            }
            class Derived extends Base {
                static add(value: number): number { return super.add(value) + 1; }
                static get current(): number { return super.current + 1; }
                static set current(next: number) { super.current = next + 1; }
            }
            function main(): number {
                Derived.current = 40;
                return Derived.current + Derived.add(0);
            }"#,
        );
        for symbol in [
            "__thaw_class_Derived_static_add",
            "__thaw_class_Derived_static_getter_current",
            "__thaw_class_Derived_static_setter_current",
        ] {
            let function = program
                .functions
                .iter()
                .find(|function| function.name == symbol)
                .unwrap();
            assert!(format!("{:?}", function.body).contains("__thaw_class_Base_static"));
        }
    }

    #[test]
    fn forwards_implicit_derived_constructor_arguments_through_multiple_levels() {
        let program = lower(
            r#"class Leaf extends Middle {}
            class Middle extends Base {}
            class Base {
                constructor(public value: number, public label: string) {}
                answer(): number { return this.value; }
            }
            function main(): number {
                const value = new Leaf(42, "ready");
                console.log(value.label);
                return value.answer();
            }"#,
        );
        for class_name in ["Middle", "Leaf"] {
            let constructor = program
                .functions
                .iter()
                .find(|function| function.name == format!("__thaw_class_{class_name}_constructor"))
                .unwrap();
            assert_eq!(constructor.params.len(), 2);
        }
        let leaf_initializer = program
            .functions
            .iter()
            .find(|function| function.name == "__thaw_class_Leaf_initialize")
            .unwrap();
        assert!(format!("{:?}", leaf_initializer.body).contains("__thaw_class_Middle_initialize"));
    }

    #[test]
    fn validates_native_class_implements_against_inherited_layout() {
        let program = lower(
            r#"interface NamedValue<N, V> { name: N; value: V; }
            class Named {
                constructor(public name: string) {}
            }
            class Value extends Named implements NamedValue<string, number> {
                constructor(name: string, public value: number) { super(name); }
            }
            function main(): number { return new Value("answer", 42).value; }"#,
        );
        assert!(program
            .functions
            .iter()
            .any(|function| function.name == "__thaw_class_Value_constructor"));
    }

    #[test]
    fn rejects_native_class_missing_an_implemented_field() {
        let module = thaw_parser::parse_typescript(
            r#"interface Required { value: number; label: string; }
            class Incomplete implements Required {
                constructor(public value: number) {}
            }"#,
        )
        .unwrap();
        let error = lower_module(&module).unwrap_err();
        assert!(
            error.contains("missing field `label` required by `Required`"),
            "{error}"
        );
    }

    #[test]
    fn rejects_native_class_implemented_field_type_mismatch() {
        let module = thaw_parser::parse_typescript(
            r#"type Required = { value: number };
            class Mismatch implements Required {
                constructor(public value: string) {}
            }"#,
        )
        .unwrap();
        let error = lower_module(&module).unwrap_err();
        assert!(
            error.contains("field `value` has type Str, but `Required` requires F64"),
            "{error}"
        );
    }

    #[test]
    fn lowers_initialized_native_static_fields_as_globals() {
        let program = lower(
            r#"class Counter {
                static base: number = 40;
                static value: number = Counter.base + 2;
                static readonly label: string = "ready";
                static next(): number { Counter.value += 1; return Counter.value; }
            }
            function main(): number { console.log(Counter.label); return Counter.next(); }"#,
        );
        for field in ["base", "value", "label"] {
            assert!(program
                .globals
                .iter()
                .any(|global| { global.name == class_static_field_symbol("Counter", field) }));
        }
        assert!(program.initializers.iter().any(|step| matches!(
            step,
            HirInitStep::StoreGlobal(name, _)
                if name == &class_static_field_symbol("Counter", "value")
        )));
    }

    #[test]
    fn rejects_assignment_to_readonly_native_static_field() {
        let module = thaw_parser::parse_typescript(
            r#"class Constants { static readonly answer: number = 42; }
            function main(): void { Constants.answer = 0; }"#,
        )
        .unwrap();
        let error = lower_module(&module).unwrap_err();
        assert!(error.contains("cannot assign to constant"), "{error}");
    }

    #[test]
    fn inherits_native_static_fields_without_copying_storage() {
        let program = lower(
            r#"class Base {
                static value: number = 40;
                static readonly label: string = "shared";
            }
            class Middle extends Base {}
            class Leaf extends Middle {}
            function main(): number { Leaf.value = 42; return Base.value; }"#,
        );
        assert!(program
            .functions
            .iter()
            .any(|function| function.name == class_getter_symbol("Leaf", "value", true)));
        assert!(program
            .functions
            .iter()
            .any(|function| function.name == class_setter_symbol("Leaf", "value", true)));
        assert!(!program
            .globals
            .iter()
            .any(|global| { global.name == class_static_field_symbol("Leaf", "value") }));
        assert!(!program
            .functions
            .iter()
            .any(|function| function.name == class_setter_symbol("Leaf", "label", true)));
    }

    #[test]
    fn rejects_assignment_to_inherited_readonly_native_static_field() {
        let module = thaw_parser::parse_typescript(
            r#"class Base { static readonly label: string = "fixed"; }
            class Derived extends Base {}
            function main(): void { Derived.label = "changed"; }"#,
        )
        .unwrap();
        let error = lower_module(&module).unwrap_err();
        assert!(
            error.contains("cannot assign to readonly static member `Derived.label`"),
            "{error}"
        );
    }

    #[test]
    fn lowers_native_static_blocks_in_class_body_order() {
        let program = lower(
            r#"let trace: string = "";
            class Counter {
                static value: number = 1;
                static { Counter.value += 40; trace += "A"; }
                static result: number = Counter.value + 1;
                static { trace += "B"; }
            }
            function main(): number { console.log(trace); return Counter.result; }"#,
        );
        let value = class_static_field_symbol("Counter", "value");
        let result = class_static_field_symbol("Counter", "result");
        let value_index = program
            .initializers
            .iter()
            .position(|step| matches!(step, HirInitStep::StoreGlobal(name, _) if name == &value))
            .unwrap();
        let result_index = program
            .initializers
            .iter()
            .position(|step| matches!(step, HirInitStep::StoreGlobal(name, _) if name == &result))
            .unwrap();
        assert!(result_index > value_index + 1);
    }

    #[test]
    fn lowers_string_literal_computed_native_class_members() {
        let program = lower(
            r#"class Box {
                ["value"]: number;
                static ["count"]: number = 40;
                constructor(value: number) { this["value"] = value; }
                ["add"](delta: number): number { return this["value"] + delta; }
                get ["current"](): number { return this["value"]; }
                set ["current"](value: number) { this["value"] = value; }
                static ["next"](): number { return ++Box["count"]; }
            }
            function main(): number {
                const value = new Box(40);
                value["current"] = value["add"](2);
                return value["current"] + Box["next"]();
            }"#,
        );
        for symbol in [
            class_method_symbol("Box", "add"),
            class_getter_symbol("Box", "current", false),
            class_setter_symbol("Box", "current", false),
            class_static_method_symbol("Box", "next"),
        ] {
            assert!(program
                .functions
                .iter()
                .any(|function| function.name == symbol));
        }
    }

    #[test]
    fn rejects_dynamically_computed_native_class_members() {
        let module = thaw_parser::parse_typescript(
            r#"const key: string = "value";
            class Box { [key]: number = 42; }
            function main(): void {}"#,
        )
        .unwrap();
        let error = lower_module(&module).unwrap_err();
        assert!(
            error.contains("computed members require a string-literal name"),
            "{error}"
        );
    }
}
