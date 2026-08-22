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
    AssignOp, AssignTarget, BinaryOp, CallExpr, Callee, Decl, Expr, FnDecl, KeyValueProp, Lit,
    MemberExpr, MemberProp, Module, ModuleItem, ObjectLit as SwcObjectLit, Pat, Prop, PropName,
    PropOrSpread, SimpleAssignTarget, Stmt, TsKeywordTypeKind, TsType, TsTypeElement, UpdateOp,
    VarDecl, VarDeclOrExpr,
};

use crate::{BinOp, HirExpr, HirFunction, HirLit, HirParam, HirProgram, HirStmt, HirType, Symbol};

/// Signature info needed to type calls to other top-level functions during
/// lowering, collected in a pre-pass over the whole module before any
/// function body is lowered (so forward references and mutual calls work).
struct FnSignature {
    params: Vec<HirType>,
    ret: HirType,
}

pub fn lower_module(module: &Module) -> Result<HirProgram, String> {
    let mut signatures: HashMap<Symbol, FnSignature> = HashMap::new();
    let mut fn_decls = Vec::new();

    for item in &module.body {
        match item {
            ModuleItem::Stmt(Stmt::Decl(Decl::Fn(fn_decl))) => {
                let name = fn_decl.ident.sym.to_string();
                let func = &fn_decl.function;
                let params = func
                    .params
                    .iter()
                    .map(|p| lower_param(&p.pat).map(|p| p.ty))
                    .collect::<Result<Vec<_>, _>>()?;
                let ret = lower_fn_return_type(func.is_async, &func.return_type, &name)?;
                signatures.insert(name, FnSignature { params, ret });
                fn_decls.push(fn_decl);
            }
            ModuleItem::Stmt(_) => {
                return Err(
                    "Phase 0/1/2 only support top-level function declarations; wrap other code in a function"
                        .into(),
                )
            }
            ModuleItem::ModuleDecl(_) => {
                return Err("import/export declarations are not supported yet".into())
            }
        }
    }

    let functions = fn_decls
        .into_iter()
        .map(|fn_decl| lower_fn_decl(fn_decl, &signatures))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(HirProgram { functions })
}

fn lower_fn_decl(
    fn_decl: &FnDecl,
    signatures: &HashMap<Symbol, FnSignature>,
) -> Result<HirFunction, String> {
    let name = fn_decl.ident.sym.to_string();
    let func = &fn_decl.function;

    let params = func
        .params
        .iter()
        .map(|param| lower_param(&param.pat))
        .collect::<Result<Vec<_>, _>>()?;

    let ret = lower_fn_return_type(func.is_async, &func.return_type, &name)?;

    let body_block = func
        .body
        .as_ref()
        .ok_or_else(|| format!("function `{name}` has no body (ambient/overload decl?)"))?;

    let mut lowerer = FnLowerer::new(signatures, ret.clone());
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
) -> Result<HirType, String> {
    let declared = match return_type {
        Some(ann) => lower_ts_type(&ann.type_ann)?,
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

fn lower_param(pat: &Pat) -> Result<HirParam, String> {
    let Pat::Ident(binding) = pat else {
        return Err("only simple identifier parameters are supported".into());
    };
    let name = binding.id.sym.to_string();
    let ty = match &binding.type_ann {
        Some(ann) => lower_ts_type(&ann.type_ann)?,
        None => {
            return Err(format!(
                "parameter `{name}` needs an explicit type annotation (no type inference for params)"
            ))
        }
    };
    Ok(HirParam { name, ty })
}

fn lower_ts_type(ty: &TsType) -> Result<HirType, String> {
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
        TsType::TsArrayType(arr) => Ok(HirType::Array(Box::new(lower_ts_type(&arr.elem_type)?))),
        TsType::TsTypeRef(ty_ref) => {
            // Accept `Array<T>` / `Promise<T>` as the two built-in generic
            // spellings we recognize. This is not general generics support
            // (still deferred per the roadmap) -- just these two names.
            let ref_name = match &ty_ref.type_name {
                swc_ecma_ast::TsEntityName::Ident(id) => Some(id.sym.as_str()),
                swc_ecma_ast::TsEntityName::TsQualifiedName(_) => None,
            };
            let single_type_param = ty_ref
                .type_params
                .as_ref()
                .and_then(|params| match params.params.as_slice() {
                    [elem] => Some(elem.as_ref()),
                    _ => None,
                });

            match (ref_name, single_type_param) {
                (Some("Array"), Some(elem)) => Ok(HirType::Array(Box::new(lower_ts_type(elem)?))),
                (Some("Promise"), Some(inner)) => {
                    Ok(HirType::Promise(Box::new(lower_ts_type(inner)?)))
                }
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
                    Ok((name, lower_ts_type(&ann.type_ann)?))
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok(HirType::Object(fields))
        }
        other => Err(format!(
            "unsupported type annotation {other:?} (supports primitive keywords, T[]/Array<T>, and object type literals)"
        )),
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
    ret_type: HirType,
}

impl<'a> FnLowerer<'a> {
    fn new(signatures: &'a HashMap<Symbol, FnSignature>, ret_type: HirType) -> Self {
        Self {
            scope: HashMap::new(),
            signatures,
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
                    Some(ann) => lower_ts_type(&ann.type_ann)?,
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
                if name == "console.log" {
                    return Ok(HirType::F64);
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
                let index = self.lower_expr(&computed.expr)?;
                Ok(HirExpr::Index(Box::new(obj), Box::new(index)))
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
                    other => Err(format!(
                        "unsupported property access `.{}` on a value of type {other:?}",
                        prop.sym
                    )),
                }
            }
            _ => Err("unsupported property access".into()),
        }
    }

    fn lower_assign_target(&mut self, target: &AssignTarget) -> Result<Target, String> {
        let AssignTarget::Simple(simple) = target else {
            return Err("destructuring assignment targets are not supported".into());
        };
        match simple {
            SimpleAssignTarget::Ident(binding) => Ok(Target::Var(binding.id.sym.to_string())),
            SimpleAssignTarget::Member(member) => match &member.prop {
                MemberProp::Computed(computed) => Ok(Target::Index(
                    self.lower_expr(&member.obj)?,
                    self.lower_expr(&computed.expr)?,
                )),
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

        let value = if let Target::Var(name) = &target {
            match self.scope.get(name).cloned() {
                Some(ty) => self.coerce_to_declared(&ty, value)?,
                None => value,
            }
        } else {
            value
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
                MemberProp::Computed(computed) => Target::Index(
                    self.lower_expr(&member.obj)?,
                    self.lower_expr(&computed.expr)?,
                ),
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

        let param_types = self.signatures.get(&callee_name).map(|sig| sig.params.clone());

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
}
