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
    let hex = match hex.split_once("__arity_") {
        Some((hex, arity)) if arity.parse::<usize>().is_ok() => hex,
        Some(_) => return None,
        None => hex,
    };
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

include!("lower/module/helpers.rs");
include!("lower/module/classes.rs");
include!("lower/module/pipeline.rs");
include!("lower/module/globals.rs");

include!("lower/types.rs");

include!("lower/metadata.rs");

include!("lower/interfaces.rs");

include!("lower/declarations.rs");

include!("lower/type_resolution.rs");

include!("lower/control_flow.rs");

include!("lower/context.rs");
include!("lower/calls.rs");

impl<'a> FnLowerer<'a> {}

include!("lower/statements/context.rs");
include!("lower/statements/narrowing.rs");
include!("lower/statements/lowering.rs");
include!("lower/statements/declarations.rs");

impl<'a> FnLowerer<'a> {}

include!("lower/destructuring.rs");

impl<'a> FnLowerer<'a> {}

include!("lower/inference/coercions.rs");
include!("lower/inference/properties.rs");
include!("lower/inference/callables.rs");
include!("lower/inference/types.rs");

impl<'a> FnLowerer<'a> {}

include!("lower/expressions/coercions.rs");
include!("lower/expressions/lowering.rs");
include!("lower/expressions/functions.rs");

impl<'a> FnLowerer<'a> {}

include!("lower/promises.rs");

impl<'a> FnLowerer<'a> {}

include!("lower/objects.rs");

impl<'a> FnLowerer<'a> {}

include!("lower/assignments/targets.rs");
include!("lower/assignments/lowering.rs");
include!("lower/assignments/updates.rs");

impl<'a> FnLowerer<'a> {}

include!("lower/arrays/support.rs");
include!("lower/arrays/transformations.rs");
include!("lower/arrays/operations.rs");

impl<'a> FnLowerer<'a> {}

include!("lower/invocations/static_builtins.rs");
include!("lower/invocations/instance_builtins.rs");
include!("lower/invocations/promises.rs");

include!("lower/invocations.rs");

impl<'a> FnLowerer<'a> {}

include!("lower/generic_calls.rs");

#[cfg(test)]
mod tests;
