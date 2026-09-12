// Erases the `HirType::Bytes` marker.
// 
// `Bytes` is a lowering-time-only distinction: it type-checks and
// dispatches (`buf.toString("utf8")` vs an array's `.toString()`)
// differently from `Array(F64)`, but is *physically* an `Array(F64)`.
// Codegen and every downstream consumer only knows `Array`, so this
// pass rewrites every `Bytes` reachable from a lowered `HirProgram` to
// `Array(F64)` before it leaves `thaw_hir::lower`.
// 
// It walks the whole IR. A missed child-recursion arm would leave a
// nested `Bytes` behind.
// 
// `include!`d into `lower.rs`, so it shares that module's imports.


fn erase_bytes(program: &mut HirProgram) {
    for global in &mut program.globals {
        erase_global(global);
    }
    for step in &mut program.initializers {
        match step {
            HirInitStep::StoreGlobal(_, value) => erase_expr(value),
            HirInitStep::Statement(stmt) => erase_stmt(stmt),
        }
    }
    for function in &mut program.functions {
        erase_function(function);
    }
    for extern_fn in &mut program.extern_functions {
        erase_ffi(extern_fn);
    }
}

fn erase_global(global: &mut crate::HirGlobal) {
    erase_ty(&mut global.ty);
    erase_expr(&mut global.init);
}

fn erase_function(function: &mut HirFunction) {
    for param in &mut function.params {
        erase_param(param);
    }
    erase_ty(&mut function.ret);
    erase_stmts(&mut function.body);
}

fn erase_param(param: &mut HirParam) {
    erase_ty(&mut param.ty);
}

fn erase_ffi(sig: &mut FfiSignature) {
    for ty in &mut sig.params {
        erase_ty(ty);
    }
    if let Some(ty) = &mut sig.variadic {
        erase_ty(ty);
    }
    erase_ty(&mut sig.ret);
}

fn erase_dyn(sig: &mut DynamicSignature) {
    for ty in &mut sig.params {
        erase_ty(ty);
    }
    erase_ty(&mut sig.ret);
}

fn erase_ty(ty: &mut HirType) {
    match ty {
        HirType::Bytes => *ty = HirType::Array(Box::new(HirType::F64)),
        HirType::Array(inner)
        | HirType::Promise(inner)
        | HirType::Dictionary(inner)
        | HirType::Set(inner)
        | HirType::Optional(inner)
        | HirType::Nullable(inner)
        | HirType::Nullish(inner) => erase_ty(inner),
        HirType::Map(key, value) => {
            erase_ty(key);
            erase_ty(value);
        }
        HirType::Tuple(elements) | HirType::Union(elements) => {
            for element in elements {
                erase_ty(element);
            }
        }
        HirType::Object(fields) => {
            for (_, field) in fields {
                erase_ty(field);
            }
        }
        HirType::Function(params, ret) => {
            for param in params {
                erase_ty(param);
            }
            erase_ty(ret);
        }
        HirType::CallableFunction(params, _, rest, ret) => {
            for param in params {
                erase_ty(param);
            }
            if let Some(rest) = rest {
                erase_ty(rest);
            }
            erase_ty(ret);
        }
        HirType::F64
        | HirType::I64
        | HirType::Bool
        | HirType::Undefined
        | HirType::Null
        | HirType::Void
        | HirType::Str
        | HirType::Symbol
        | HirType::StrLiteral(_)
        | HirType::Json
        | HirType::JsValue
        | HirType::Dynamic => {}
    }
}

fn erase_tys(types: &mut [HirType]) {
    for ty in types {
        erase_ty(ty);
    }
}

fn erase_stmts(stmts: &mut [HirStmt]) {
    for stmt in stmts {
        erase_stmt(stmt);
    }
}

fn erase_stmt(stmt: &mut HirStmt) {
    match stmt {
        HirStmt::Expr(expr) | HirStmt::Throw(expr) => erase_expr(expr),
        HirStmt::Return(value) => {
            if let Some(value) = value {
                erase_expr(value);
            }
        }
        HirStmt::Let(_, ty, value) => {
            erase_ty(ty);
            erase_expr(value);
        }
        HirStmt::If(cond, then_body, else_body) => {
            erase_expr(cond);
            erase_stmts(then_body);
            erase_stmts(else_body);
        }
        HirStmt::While(cond, body) => {
            erase_expr(cond);
            erase_stmts(body);
        }
        HirStmt::Try(body, _, handler) => {
            erase_stmts(body);
            erase_stmts(handler);
        }
        HirStmt::Break
        | HirStmt::Continue
        | HirStmt::BreakDepth(_)
        | HirStmt::ContinueDepth(_) => {}
    }
}

fn erase_exprs(exprs: &mut [HirExpr]) {
    for expr in exprs {
        erase_expr(expr);
    }
}

fn erase_expr(expr: &mut HirExpr) {
    match expr {
        HirExpr::Lit(_)
        | HirExpr::Var(_)
        | HirExpr::EnvVar(_)
        | HirExpr::PostfixUpdate(_, _) => {}

        HirExpr::BinOp(_, left, right)
        | HirExpr::Index(left, right)
        | HirExpr::JsonKey(left, right)
        | HirExpr::JsonIndex(left, right)
        | HirExpr::JsonDelete(left, right) => {
            erase_expr(left);
            erase_expr(right);
        }
        HirExpr::ThrowValue(a, b) => {
            erase_expr(a);
            erase_expr(b);
        }

        HirExpr::Conditional(a, b, c, ty) => {
            erase_expr(a);
            erase_expr(b);
            erase_expr(c);
            erase_ty(ty);
        }

        HirExpr::OptionalSome(value, ty)
        | HirExpr::OptionalIsNone(value, ty)
        | HirExpr::OptionalValue(value, ty)
        | HirExpr::NullableSome(value, ty)
        | HirExpr::NullableIsNone(value, ty)
        | HirExpr::NullableValue(value, ty)
        | HirExpr::NullishSome(value, ty)
        | HirExpr::NullishIsNull(value, ty)
        | HirExpr::NullishIsUndefined(value, ty)
        | HirExpr::NullishIsNone(value, ty)
        | HirExpr::NullishValue(value, ty)
        | HirExpr::AwaitPromise(value, ty)
        | HirExpr::JsonAsNative(value, ty) => {
            erase_expr(value);
            erase_ty(ty);
        }

        HirExpr::OptionalNone(ty)
        | HirExpr::NullableNone(ty)
        | HirExpr::NullishNull(ty)
        | HirExpr::NullishUndefined(ty)
        | HirExpr::ObjectAlloc(ty) => erase_ty(ty),

        HirExpr::UnionInject(value, _, types) | HirExpr::UnionValue(value, _, types) => {
            erase_expr(value);
            erase_tys(types);
        }
        HirExpr::UnionTag(value, types) => {
            erase_expr(value);
            erase_tys(types);
        }
        HirExpr::UnionMemberIsEqual(left, right, _, types)
        | HirExpr::UnionIsEqual(left, right, types) => {
            erase_expr(left);
            erase_expr(right);
            erase_tys(types);
        }

        HirExpr::Call(callee, args) => {
            erase_expr(callee);
            erase_exprs(args);
        }
        HirExpr::FunctionCallWithThis(callee, this_arg, args, types, ret)
        | HirExpr::FunctionBindThis(callee, this_arg, args, types, ret) => {
            erase_expr(callee);
            erase_expr(this_arg);
            erase_exprs(args);
            erase_tys(types);
            erase_ty(ret);
        }

        HirExpr::PromiseAll(args, ty)
        | HirExpr::PromiseRace(args, ty)
        | HirExpr::PromiseAny(args, ty)
        | HirExpr::PromiseAllSettled(args, ty)
        | HirExpr::ArrayConcat(args, ty) => {
            erase_exprs(args);
            erase_ty(ty);
        }
        HirExpr::PromiseAllTuple(args, types) => {
            erase_exprs(args);
            erase_tys(types);
        }
        HirExpr::PromiseAllArray(value, ty)
        | HirExpr::PromiseRaceArray(value, ty)
        | HirExpr::PromiseAnyArray(value, ty)
        | HirExpr::PromiseAllSettledArray(value, ty)
        | HirExpr::ArrayAlloc(value, ty) => {
            erase_expr(value);
            erase_ty(ty);
        }
        HirExpr::ArraySetLen(value, len, ty) => {
            erase_expr(value);
            erase_expr(len);
            erase_ty(ty);
        }
        HirExpr::PromiseNew(value, ty, _) => {
            erase_expr(value);
            erase_ty(ty);
        }
        HirExpr::PromiseThen(source, callback, a, b, _, _) => {
            erase_expr(source);
            erase_expr(callback);
            erase_ty(a);
            erase_ty(b);
        }
        HirExpr::PromiseFinally(source, callback, a, b) => {
            erase_expr(source);
            erase_expr(callback);
            erase_ty(a);
            erase_ty(b);
        }
        HirExpr::Await(value) | HirExpr::ArrayLen(value) => erase_expr(value),

        HirExpr::Lambda(captures, params, ret, body) => {
            for param in captures.iter_mut().chain(params.iter_mut()) {
                erase_param(param);
            }
            erase_ty(ret);
            erase_expr(body);
        }
        HirExpr::RecursiveClosure(_, ty, closure) | HirExpr::TypedClosure(ty, closure) => {
            erase_ty(ty);
            erase_expr(closure);
        }
        HirExpr::FunctionRef(_, types, ret) | HirExpr::MethodRef(_, _, types, ret, _) => {
            erase_tys(types);
            erase_ty(ret);
        }
        HirExpr::Block(stmts) => erase_stmts(stmts),

        HirExpr::FfiCall(sig, args) => {
            erase_ffi(sig);
            erase_exprs(args);
        }
        HirExpr::DynamicCall(sig, args) => {
            erase_dyn(sig);
            erase_exprs(args);
        }

        HirExpr::Assign(_, value) => erase_expr(value),

        HirExpr::ArrayLit(items) => erase_exprs(items),

        HirExpr::TypedIndex(object, index, ty) => {
            erase_expr(object);
            erase_expr(index);
            erase_ty(ty);
        }
        HirExpr::IndexAssign(object, index, value) => {
            erase_expr(object);
            erase_expr(index);
            erase_expr(value);
        }

        HirExpr::ObjectLit(fields) => {
            for (_, value) in fields {
                erase_expr(value);
            }
        }
        HirExpr::JsonObjectLit(fields, ty) => {
            for (_, value) in fields {
                erase_expr(value);
            }
            erase_ty(ty);
        }

        HirExpr::PropAccess(object, ty, _) => {
            erase_expr(object);
            erase_ty(ty);
        }
        HirExpr::DynamicPropAccess(object, key, fields, ty) => {
            erase_expr(object);
            erase_expr(key);
            for (_, field_ty) in fields {
                erase_ty(field_ty);
            }
            erase_ty(ty);
        }
        HirExpr::EnumReverseLookup(value, _) => erase_expr(value),
        HirExpr::PropAssign(object, ty, _, value) => {
            erase_expr(object);
            erase_ty(ty);
            erase_expr(value);
        }
        HirExpr::JsonGet(object, _) => erase_expr(object),
        HirExpr::JsonSet(object, key, value, ty, _) => {
            erase_expr(object);
            erase_expr(key);
            erase_expr(value);
            erase_ty(ty);
        }
        HirExpr::JsonIndexSet(object, key, value) => {
            erase_expr(object);
            erase_expr(key);
            erase_expr(value);
        }
        HirExpr::JsonAsNumber(value)
        | HirExpr::JsonAsString(value)
        | HirExpr::JsonAsBool(value)
        | HirExpr::JsValueAsJson(value) => erase_expr(value),
    }
}
