use super::*;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct HirProgram {
    pub globals: Vec<HirGlobal>,
    pub initializers: Vec<HirInitStep>,
    pub functions: Vec<HirFunction>,
    /// Ambient function declarations (`declare function foo(...): T;`, no
    /// body) -- see docs/design/bridge.md section 6. Calls to one of these
    /// names lower to `HirExpr::FfiCall` instead of `HirExpr::Call`;
    /// codegen declares each as an `extern "C"` symbol that the final link
    /// step must resolve from elsewhere (a real native library today; a
    /// thaw-registry-fetched one eventually).
    pub extern_functions: Vec<FfiSignature>,
}

/// Applies an explicitly configured error ABI to an ambient symbol and every
/// call site that carries its copied [`FfiSignature`]. Direct ABI remains the
/// default so existing native libraries are unaffected.
pub fn set_ffi_error_abi(
    program: &mut HirProgram,
    symbol: &str,
    error_abi: FfiErrorAbi,
) -> Result<(), String> {
    let mut found = false;
    for signature in &mut program.extern_functions {
        if signature.symbol == symbol {
            signature.error_abi = error_abi.clone();
            found = true;
        }
    }

    fn visit_expr(expr: &mut HirExpr, symbol: &str, abi: &FfiErrorAbi, found: &mut bool) {
        match expr {
            HirExpr::FfiCall(signature, args) => {
                if signature.symbol == symbol {
                    signature.error_abi = abi.clone();
                    *found = true;
                }
                for arg in args {
                    visit_expr(arg, symbol, abi, found);
                }
            }
            HirExpr::BinOp(_, left, right)
            | HirExpr::UnionMemberIsEqual(left, right, _, _)
            | HirExpr::UnionIsEqual(left, right, _)
            | HirExpr::Index(left, right)
            | HirExpr::TypedIndex(left, right, _)
            | HirExpr::ArraySetLen(left, right, _)
            | HirExpr::DynamicPropAccess(left, right, _, _)
            | HirExpr::JsonKey(left, right)
            | HirExpr::JsonDelete(left, right) => {
                visit_expr(left, symbol, abi, found);
                visit_expr(right, symbol, abi, found);
            }
            HirExpr::JsonSet(object, key, value, _) | HirExpr::JsonIndexSet(object, key, value) => {
                visit_expr(object, symbol, abi, found);
                visit_expr(key, symbol, abi, found);
                visit_expr(value, symbol, abi, found);
            }
            HirExpr::Call(callee, args) => {
                visit_expr(callee, symbol, abi, found);
                for arg in args {
                    visit_expr(arg, symbol, abi, found);
                }
            }
            HirExpr::FunctionCallWithThis(callee, this_arg, args, _, _) => {
                visit_expr(callee, symbol, abi, found);
                visit_expr(this_arg, symbol, abi, found);
                for arg in args {
                    visit_expr(arg, symbol, abi, found);
                }
            }
            HirExpr::FunctionBindThis(callee, this_arg, args, _, _) => {
                visit_expr(callee, symbol, abi, found);
                visit_expr(this_arg, symbol, abi, found);
                for arg in args {
                    visit_expr(arg, symbol, abi, found);
                }
            }
            HirExpr::DynamicCall(_, args) => {
                for arg in args {
                    visit_expr(arg, symbol, abi, found);
                }
            }
            HirExpr::Await(inner)
            | HirExpr::AwaitPromise(inner, _)
            | HirExpr::PromiseAllArray(inner, _)
            | HirExpr::PromiseRaceArray(inner, _)
            | HirExpr::PromiseAnyArray(inner, _)
            | HirExpr::PromiseAllSettledArray(inner, _)
            | HirExpr::Assign(_, inner)
            | HirExpr::ArrayAlloc(inner, _)
            | HirExpr::ArrayLen(inner)
            | HirExpr::EnumReverseLookup(inner, _)
            | HirExpr::JsonAsNumber(inner)
            | HirExpr::JsonAsString(inner)
            | HirExpr::JsonAsBool(inner)
            | HirExpr::JsonAsNative(inner, _)
            | HirExpr::OptionalSome(inner, _)
            | HirExpr::OptionalIsNone(inner, _)
            | HirExpr::OptionalValue(inner, _)
            | HirExpr::NullableSome(inner, _)
            | HirExpr::NullableIsNone(inner, _)
            | HirExpr::NullableValue(inner, _)
            | HirExpr::NullishSome(inner, _)
            | HirExpr::NullishIsNull(inner, _)
            | HirExpr::NullishIsUndefined(inner, _)
            | HirExpr::NullishIsNone(inner, _)
            | HirExpr::NullishValue(inner, _)
            | HirExpr::UnionInject(inner, _, _)
            | HirExpr::UnionTag(inner, _)
            | HirExpr::UnionValue(inner, _, _) => visit_expr(inner, symbol, abi, found),
            HirExpr::RecursiveClosure(_, _, closure) => visit_expr(closure, symbol, abi, found),
            HirExpr::TypedClosure(_, closure) => visit_expr(closure, symbol, abi, found),
            HirExpr::Lambda(_, _, _, body) => visit_expr(body, symbol, abi, found),
            HirExpr::PromiseNew(executor, _, _) => visit_expr(executor, symbol, abi, found),
            HirExpr::PromiseThen(source, callback, _, _, _, _) => {
                visit_expr(source, symbol, abi, found);
                visit_expr(callback, symbol, abi, found);
            }
            HirExpr::PromiseFinally(source, callback, _, _) => {
                visit_expr(source, symbol, abi, found);
                visit_expr(callback, symbol, abi, found);
            }
            HirExpr::ThrowValue(error, fallback) => {
                visit_expr(error, symbol, abi, found);
                visit_expr(fallback, symbol, abi, found);
            }
            HirExpr::Block(stmts) => visit_stmts(stmts, symbol, abi, found),
            HirExpr::ArrayLit(values)
            | HirExpr::ArrayConcat(values, _)
            | HirExpr::PromiseAll(values, _)
            | HirExpr::PromiseAllTuple(values, _)
            | HirExpr::PromiseRace(values, _)
            | HirExpr::PromiseAny(values, _)
            | HirExpr::PromiseAllSettled(values, _) => {
                for value in values {
                    visit_expr(value, symbol, abi, found);
                }
            }
            HirExpr::IndexAssign(array, index, value) => {
                visit_expr(array, symbol, abi, found);
                visit_expr(index, symbol, abi, found);
                visit_expr(value, symbol, abi, found);
            }
            HirExpr::ObjectLit(fields) => {
                for (_, value) in fields {
                    visit_expr(value, symbol, abi, found);
                }
            }
            HirExpr::JsonObjectLit(fields, _) => {
                for (_, value) in fields {
                    visit_expr(value, symbol, abi, found);
                }
            }
            HirExpr::PropAccess(object, _, _) | HirExpr::JsonGet(object, _) => {
                visit_expr(object, symbol, abi, found);
            }
            HirExpr::PropAssign(object, _, _, value) | HirExpr::JsonIndex(object, value) => {
                visit_expr(object, symbol, abi, found);
                visit_expr(value, symbol, abi, found);
            }
            HirExpr::Lit(_)
            | HirExpr::OptionalNone(_)
            | HirExpr::NullableNone(_)
            | HirExpr::NullishNull(_)
            | HirExpr::NullishUndefined(_)
            | HirExpr::Var(_)
            | HirExpr::EnvVar(_)
            | HirExpr::ObjectAlloc(_)
            | HirExpr::FunctionRef(..)
            | HirExpr::MethodRef(..) => {}
        }
    }

    fn visit_stmts(stmts: &mut [HirStmt], symbol: &str, abi: &FfiErrorAbi, found: &mut bool) {
        for stmt in stmts {
            match stmt {
                HirStmt::Expr(expr) | HirStmt::Throw(expr) | HirStmt::Let(_, _, expr) => {
                    visit_expr(expr, symbol, abi, found)
                }
                HirStmt::Return(Some(expr)) => visit_expr(expr, symbol, abi, found),
                HirStmt::If(condition, then_body, else_body) => {
                    visit_expr(condition, symbol, abi, found);
                    visit_stmts(then_body, symbol, abi, found);
                    visit_stmts(else_body, symbol, abi, found);
                }
                HirStmt::While(condition, body) => {
                    visit_expr(condition, symbol, abi, found);
                    visit_stmts(body, symbol, abi, found);
                }
                HirStmt::Try(body, _, catch_body) => {
                    visit_stmts(body, symbol, abi, found);
                    visit_stmts(catch_body, symbol, abi, found);
                }
                HirStmt::Return(None)
                | HirStmt::Break
                | HirStmt::Continue
                | HirStmt::BreakDepth(_)
                | HirStmt::ContinueDepth(_) => {}
            }
        }
    }

    for function in &mut program.functions {
        visit_stmts(&mut function.body, symbol, &error_abi, &mut found);
    }
    if found {
        Ok(())
    } else {
        Err(format!(
            "FFI metadata references unknown ambient function `{symbol}`"
        ))
    }
}

pub fn set_ffi_ownership(
    program: &mut HirProgram,
    symbol: &str,
    return_ownership: FfiOwnership,
    error_ownership: FfiOwnership,
) -> Result<(), String> {
    fn update_expr(
        expr: &mut HirExpr,
        symbol: &str,
        returns: &FfiOwnership,
        errors: &FfiOwnership,
        found: &mut bool,
    ) {
        if let HirExpr::FfiCall(signature, _) = expr {
            if signature.symbol == symbol {
                signature.return_ownership = returns.clone();
                signature.error_ownership = errors.clone();
                *found = true;
            }
        }
        match expr {
            HirExpr::FfiCall(_, args)
            | HirExpr::ArrayLit(args)
            | HirExpr::ArrayConcat(args, _)
            | HirExpr::PromiseAll(args, _)
            | HirExpr::PromiseAllTuple(args, _)
            | HirExpr::PromiseRace(args, _)
            | HirExpr::PromiseAny(args, _)
            | HirExpr::PromiseAllSettled(args, _) => {
                for arg in args {
                    update_expr(arg, symbol, returns, errors, found);
                }
            }
            HirExpr::Call(callee, args) => {
                update_expr(callee, symbol, returns, errors, found);
                for arg in args {
                    update_expr(arg, symbol, returns, errors, found);
                }
            }
            HirExpr::FunctionCallWithThis(callee, this_arg, args, _, _) => {
                update_expr(callee, symbol, returns, errors, found);
                update_expr(this_arg, symbol, returns, errors, found);
                for arg in args {
                    update_expr(arg, symbol, returns, errors, found);
                }
            }
            HirExpr::FunctionBindThis(callee, this_arg, args, _, _) => {
                update_expr(callee, symbol, returns, errors, found);
                update_expr(this_arg, symbol, returns, errors, found);
                for arg in args {
                    update_expr(arg, symbol, returns, errors, found);
                }
            }
            HirExpr::DynamicCall(_, args) => {
                for arg in args {
                    update_expr(arg, symbol, returns, errors, found);
                }
            }
            HirExpr::BinOp(_, left, right)
            | HirExpr::UnionMemberIsEqual(left, right, _, _)
            | HirExpr::UnionIsEqual(left, right, _)
            | HirExpr::Index(left, right)
            | HirExpr::TypedIndex(left, right, _)
            | HirExpr::ArraySetLen(left, right, _)
            | HirExpr::DynamicPropAccess(left, right, _, _)
            | HirExpr::JsonKey(left, right)
            | HirExpr::JsonDelete(left, right) => {
                update_expr(left, symbol, returns, errors, found);
                update_expr(right, symbol, returns, errors, found);
            }
            HirExpr::JsonSet(object, key, value, _) | HirExpr::JsonIndexSet(object, key, value) => {
                update_expr(object, symbol, returns, errors, found);
                update_expr(key, symbol, returns, errors, found);
                update_expr(value, symbol, returns, errors, found);
            }
            HirExpr::Await(inner)
            | HirExpr::AwaitPromise(inner, _)
            | HirExpr::PromiseAllArray(inner, _)
            | HirExpr::PromiseRaceArray(inner, _)
            | HirExpr::PromiseAnyArray(inner, _)
            | HirExpr::PromiseAllSettledArray(inner, _)
            | HirExpr::Assign(_, inner)
            | HirExpr::ArrayAlloc(inner, _)
            | HirExpr::ArrayLen(inner)
            | HirExpr::EnumReverseLookup(inner, _)
            | HirExpr::JsonAsNumber(inner)
            | HirExpr::JsonAsString(inner)
            | HirExpr::JsonAsBool(inner)
            | HirExpr::JsonAsNative(inner, _)
            | HirExpr::OptionalSome(inner, _)
            | HirExpr::OptionalIsNone(inner, _)
            | HirExpr::OptionalValue(inner, _)
            | HirExpr::NullableSome(inner, _)
            | HirExpr::NullableIsNone(inner, _)
            | HirExpr::NullableValue(inner, _)
            | HirExpr::NullishSome(inner, _)
            | HirExpr::NullishIsNull(inner, _)
            | HirExpr::NullishIsUndefined(inner, _)
            | HirExpr::NullishIsNone(inner, _)
            | HirExpr::NullishValue(inner, _)
            | HirExpr::UnionInject(inner, _, _)
            | HirExpr::UnionTag(inner, _)
            | HirExpr::UnionValue(inner, _, _) => {
                update_expr(inner, symbol, returns, errors, found)
            }
            HirExpr::RecursiveClosure(_, _, closure) => {
                update_expr(closure, symbol, returns, errors, found)
            }
            HirExpr::TypedClosure(_, closure) => {
                update_expr(closure, symbol, returns, errors, found)
            }
            HirExpr::Lambda(_, _, _, body) => update_expr(body, symbol, returns, errors, found),
            HirExpr::PromiseNew(executor, _, _) => {
                update_expr(executor, symbol, returns, errors, found)
            }
            HirExpr::PromiseThen(source, callback, _, _, _, _) => {
                update_expr(source, symbol, returns, errors, found);
                update_expr(callback, symbol, returns, errors, found);
            }
            HirExpr::PromiseFinally(source, callback, _, _) => {
                update_expr(source, symbol, returns, errors, found);
                update_expr(callback, symbol, returns, errors, found);
            }
            HirExpr::ThrowValue(error, fallback) => {
                update_expr(error, symbol, returns, errors, found);
                update_expr(fallback, symbol, returns, errors, found);
            }
            HirExpr::Block(stmts) => update_stmts(stmts, symbol, returns, errors, found),
            HirExpr::IndexAssign(a, b, c) => {
                update_expr(a, symbol, returns, errors, found);
                update_expr(b, symbol, returns, errors, found);
                update_expr(c, symbol, returns, errors, found);
            }
            HirExpr::ObjectLit(fields) => {
                for (_, value) in fields {
                    update_expr(value, symbol, returns, errors, found);
                }
            }
            HirExpr::JsonObjectLit(fields, _) => {
                for (_, value) in fields {
                    update_expr(value, symbol, returns, errors, found);
                }
            }
            HirExpr::PropAccess(object, _, _) | HirExpr::JsonGet(object, _) => {
                update_expr(object, symbol, returns, errors, found)
            }
            HirExpr::PropAssign(object, _, _, value) | HirExpr::JsonIndex(object, value) => {
                update_expr(object, symbol, returns, errors, found);
                update_expr(value, symbol, returns, errors, found);
            }
            HirExpr::Lit(_)
            | HirExpr::OptionalNone(_)
            | HirExpr::NullableNone(_)
            | HirExpr::NullishNull(_)
            | HirExpr::NullishUndefined(_)
            | HirExpr::Var(_)
            | HirExpr::EnvVar(_)
            | HirExpr::ObjectAlloc(_)
            | HirExpr::FunctionRef(..)
            | HirExpr::MethodRef(..) => {}
        }
    }
    fn update_stmts(
        stmts: &mut [HirStmt],
        symbol: &str,
        returns: &FfiOwnership,
        errors: &FfiOwnership,
        found: &mut bool,
    ) {
        for stmt in stmts {
            match stmt {
                HirStmt::Expr(expr) | HirStmt::Throw(expr) | HirStmt::Let(_, _, expr) => {
                    update_expr(expr, symbol, returns, errors, found)
                }
                HirStmt::Return(Some(expr)) => update_expr(expr, symbol, returns, errors, found),
                HirStmt::If(condition, then_body, else_body) => {
                    update_expr(condition, symbol, returns, errors, found);
                    update_stmts(then_body, symbol, returns, errors, found);
                    update_stmts(else_body, symbol, returns, errors, found);
                }
                HirStmt::While(condition, body) => {
                    update_expr(condition, symbol, returns, errors, found);
                    update_stmts(body, symbol, returns, errors, found);
                }
                HirStmt::Try(body, _, catch_body) => {
                    update_stmts(body, symbol, returns, errors, found);
                    update_stmts(catch_body, symbol, returns, errors, found);
                }
                HirStmt::Return(None)
                | HirStmt::Break
                | HirStmt::Continue
                | HirStmt::BreakDepth(_)
                | HirStmt::ContinueDepth(_) => {}
            }
        }
    }
    let mut found = false;
    for signature in &mut program.extern_functions {
        if signature.symbol == symbol {
            signature.return_ownership = return_ownership.clone();
            signature.error_ownership = error_ownership.clone();
            found = true;
        }
    }
    for function in &mut program.functions {
        update_stmts(
            &mut function.body,
            symbol,
            &return_ownership,
            &error_ownership,
            &mut found,
        );
    }
    if found {
        Ok(())
    } else {
        Err(format!(
            "FFI ownership metadata references unknown ambient function `{symbol}`"
        ))
    }
}

pub fn set_ffi_string_abi(
    program: &mut HirProgram,
    symbol: &str,
    param_abis: Vec<FfiStringAbi>,
    return_abi: FfiStringAbi,
    calling_convention: FfiCallingConvention,
    aggregate_return_abi: FfiAggregateAbi,
) -> Result<(), String> {
    fn update_expr(
        expr: &mut HirExpr,
        symbol: &str,
        params: &[FfiStringAbi],
        returns: FfiStringAbi,
        calling_convention: FfiCallingConvention,
        aggregate_return_abi: FfiAggregateAbi,
        found: &mut bool,
    ) {
        if let HirExpr::FfiCall(signature, _) = expr {
            if signature.symbol == symbol {
                signature.param_string_abis = params.to_vec();
                signature.return_string_abi = returns;
                signature.calling_convention = calling_convention;
                signature.aggregate_return_abi = aggregate_return_abi;
                *found = true;
            }
        }
        match expr {
            HirExpr::FfiCall(_, values)
            | HirExpr::ArrayLit(values)
            | HirExpr::ArrayConcat(values, _)
            | HirExpr::PromiseAll(values, _)
            | HirExpr::PromiseAllTuple(values, _)
            | HirExpr::PromiseRace(values, _)
            | HirExpr::PromiseAny(values, _)
            | HirExpr::PromiseAllSettled(values, _) => {
                for value in values {
                    update_expr(
                        value,
                        symbol,
                        params,
                        returns,
                        calling_convention,
                        aggregate_return_abi,
                        found,
                    );
                }
            }
            HirExpr::Call(callee, values) => {
                update_expr(
                    callee,
                    symbol,
                    params,
                    returns,
                    calling_convention,
                    aggregate_return_abi,
                    found,
                );
                for value in values {
                    update_expr(
                        value,
                        symbol,
                        params,
                        returns,
                        calling_convention,
                        aggregate_return_abi,
                        found,
                    );
                }
            }
            HirExpr::FunctionCallWithThis(callee, this_arg, values, _, _) => {
                update_expr(
                    callee,
                    symbol,
                    params,
                    returns,
                    calling_convention,
                    aggregate_return_abi,
                    found,
                );
                update_expr(
                    this_arg,
                    symbol,
                    params,
                    returns,
                    calling_convention,
                    aggregate_return_abi,
                    found,
                );
                for value in values {
                    update_expr(
                        value,
                        symbol,
                        params,
                        returns,
                        calling_convention,
                        aggregate_return_abi,
                        found,
                    );
                }
            }
            HirExpr::FunctionBindThis(callee, this_arg, values, _, _) => {
                update_expr(
                    callee,
                    symbol,
                    params,
                    returns,
                    calling_convention,
                    aggregate_return_abi,
                    found,
                );
                update_expr(
                    this_arg,
                    symbol,
                    params,
                    returns,
                    calling_convention,
                    aggregate_return_abi,
                    found,
                );
                for value in values {
                    update_expr(
                        value,
                        symbol,
                        params,
                        returns,
                        calling_convention,
                        aggregate_return_abi,
                        found,
                    );
                }
            }
            HirExpr::DynamicCall(_, values) => {
                for value in values {
                    update_expr(
                        value,
                        symbol,
                        params,
                        returns,
                        calling_convention,
                        aggregate_return_abi,
                        found,
                    );
                }
            }
            HirExpr::BinOp(_, left, right)
            | HirExpr::UnionMemberIsEqual(left, right, _, _)
            | HirExpr::UnionIsEqual(left, right, _)
            | HirExpr::Index(left, right)
            | HirExpr::TypedIndex(left, right, _)
            | HirExpr::ArraySetLen(left, right, _)
            | HirExpr::DynamicPropAccess(left, right, _, _)
            | HirExpr::JsonKey(left, right)
            | HirExpr::JsonDelete(left, right) => {
                update_expr(
                    left,
                    symbol,
                    params,
                    returns,
                    calling_convention,
                    aggregate_return_abi,
                    found,
                );
                update_expr(
                    right,
                    symbol,
                    params,
                    returns,
                    calling_convention,
                    aggregate_return_abi,
                    found,
                );
            }
            HirExpr::JsonSet(object, key, value, _) | HirExpr::JsonIndexSet(object, key, value) => {
                update_expr(
                    object,
                    symbol,
                    params,
                    returns,
                    calling_convention,
                    aggregate_return_abi,
                    found,
                );
                update_expr(
                    key,
                    symbol,
                    params,
                    returns,
                    calling_convention,
                    aggregate_return_abi,
                    found,
                );
                update_expr(
                    value,
                    symbol,
                    params,
                    returns,
                    calling_convention,
                    aggregate_return_abi,
                    found,
                );
            }
            HirExpr::Await(inner)
            | HirExpr::AwaitPromise(inner, _)
            | HirExpr::PromiseAllArray(inner, _)
            | HirExpr::PromiseRaceArray(inner, _)
            | HirExpr::PromiseAnyArray(inner, _)
            | HirExpr::PromiseAllSettledArray(inner, _)
            | HirExpr::Assign(_, inner)
            | HirExpr::ArrayAlloc(inner, _)
            | HirExpr::ArrayLen(inner)
            | HirExpr::EnumReverseLookup(inner, _)
            | HirExpr::JsonAsNumber(inner)
            | HirExpr::JsonAsString(inner)
            | HirExpr::JsonAsBool(inner)
            | HirExpr::JsonAsNative(inner, _)
            | HirExpr::OptionalSome(inner, _)
            | HirExpr::OptionalIsNone(inner, _)
            | HirExpr::OptionalValue(inner, _)
            | HirExpr::NullableSome(inner, _)
            | HirExpr::NullableIsNone(inner, _)
            | HirExpr::NullableValue(inner, _)
            | HirExpr::NullishSome(inner, _)
            | HirExpr::NullishIsNull(inner, _)
            | HirExpr::NullishIsUndefined(inner, _)
            | HirExpr::NullishIsNone(inner, _)
            | HirExpr::NullishValue(inner, _)
            | HirExpr::UnionInject(inner, _, _)
            | HirExpr::UnionTag(inner, _)
            | HirExpr::UnionValue(inner, _, _) => update_expr(
                inner,
                symbol,
                params,
                returns,
                calling_convention,
                aggregate_return_abi,
                found,
            ),
            HirExpr::RecursiveClosure(_, _, closure) => update_expr(
                closure,
                symbol,
                params,
                returns,
                calling_convention,
                aggregate_return_abi,
                found,
            ),
            HirExpr::TypedClosure(_, closure) => update_expr(
                closure,
                symbol,
                params,
                returns,
                calling_convention,
                aggregate_return_abi,
                found,
            ),
            HirExpr::Lambda(_, _, _, body) => update_expr(
                body,
                symbol,
                params,
                returns,
                calling_convention,
                aggregate_return_abi,
                found,
            ),
            HirExpr::PromiseNew(executor, _, _) => update_expr(
                executor,
                symbol,
                params,
                returns,
                calling_convention,
                aggregate_return_abi,
                found,
            ),
            HirExpr::PromiseThen(source, callback, _, _, _, _) => {
                update_expr(
                    source,
                    symbol,
                    params,
                    returns,
                    calling_convention,
                    aggregate_return_abi,
                    found,
                );
                update_expr(
                    callback,
                    symbol,
                    params,
                    returns,
                    calling_convention,
                    aggregate_return_abi,
                    found,
                );
            }
            HirExpr::PromiseFinally(source, callback, _, _) => {
                update_expr(
                    source,
                    symbol,
                    params,
                    returns,
                    calling_convention,
                    aggregate_return_abi,
                    found,
                );
                update_expr(
                    callback,
                    symbol,
                    params,
                    returns,
                    calling_convention,
                    aggregate_return_abi,
                    found,
                );
            }
            HirExpr::ThrowValue(error, fallback) => {
                update_expr(
                    error,
                    symbol,
                    params,
                    returns,
                    calling_convention,
                    aggregate_return_abi,
                    found,
                );
                update_expr(
                    fallback,
                    symbol,
                    params,
                    returns,
                    calling_convention,
                    aggregate_return_abi,
                    found,
                );
            }
            HirExpr::Block(stmts) => update_stmts(
                stmts,
                symbol,
                params,
                returns,
                calling_convention,
                aggregate_return_abi,
                found,
            ),
            HirExpr::IndexAssign(a, b, c) => {
                update_expr(
                    a,
                    symbol,
                    params,
                    returns,
                    calling_convention,
                    aggregate_return_abi,
                    found,
                );
                update_expr(
                    b,
                    symbol,
                    params,
                    returns,
                    calling_convention,
                    aggregate_return_abi,
                    found,
                );
                update_expr(
                    c,
                    symbol,
                    params,
                    returns,
                    calling_convention,
                    aggregate_return_abi,
                    found,
                );
            }
            HirExpr::ObjectLit(fields) => {
                for (_, value) in fields {
                    update_expr(
                        value,
                        symbol,
                        params,
                        returns,
                        calling_convention,
                        aggregate_return_abi,
                        found,
                    );
                }
            }
            HirExpr::JsonObjectLit(fields, _) => {
                for (_, value) in fields {
                    update_expr(
                        value,
                        symbol,
                        params,
                        returns,
                        calling_convention,
                        aggregate_return_abi,
                        found,
                    );
                }
            }
            HirExpr::PropAccess(object, _, _) | HirExpr::JsonGet(object, _) => update_expr(
                object,
                symbol,
                params,
                returns,
                calling_convention,
                aggregate_return_abi,
                found,
            ),
            HirExpr::PropAssign(object, _, _, value) | HirExpr::JsonIndex(object, value) => {
                update_expr(
                    object,
                    symbol,
                    params,
                    returns,
                    calling_convention,
                    aggregate_return_abi,
                    found,
                );
                update_expr(
                    value,
                    symbol,
                    params,
                    returns,
                    calling_convention,
                    aggregate_return_abi,
                    found,
                );
            }
            HirExpr::Lit(_)
            | HirExpr::OptionalNone(_)
            | HirExpr::NullableNone(_)
            | HirExpr::NullishNull(_)
            | HirExpr::NullishUndefined(_)
            | HirExpr::Var(_)
            | HirExpr::EnvVar(_)
            | HirExpr::ObjectAlloc(_)
            | HirExpr::FunctionRef(..)
            | HirExpr::MethodRef(..) => {}
        }
    }
    fn update_stmts(
        stmts: &mut [HirStmt],
        symbol: &str,
        params: &[FfiStringAbi],
        returns: FfiStringAbi,
        calling_convention: FfiCallingConvention,
        aggregate_return_abi: FfiAggregateAbi,
        found: &mut bool,
    ) {
        for stmt in stmts {
            match stmt {
                HirStmt::Expr(expr) | HirStmt::Throw(expr) | HirStmt::Let(_, _, expr) => {
                    update_expr(
                        expr,
                        symbol,
                        params,
                        returns,
                        calling_convention,
                        aggregate_return_abi,
                        found,
                    )
                }
                HirStmt::Return(Some(expr)) => update_expr(
                    expr,
                    symbol,
                    params,
                    returns,
                    calling_convention,
                    aggregate_return_abi,
                    found,
                ),
                HirStmt::If(condition, then_body, else_body) => {
                    update_expr(
                        condition,
                        symbol,
                        params,
                        returns,
                        calling_convention,
                        aggregate_return_abi,
                        found,
                    );
                    update_stmts(
                        then_body,
                        symbol,
                        params,
                        returns,
                        calling_convention,
                        aggregate_return_abi,
                        found,
                    );
                    update_stmts(
                        else_body,
                        symbol,
                        params,
                        returns,
                        calling_convention,
                        aggregate_return_abi,
                        found,
                    );
                }
                HirStmt::While(condition, body) => {
                    update_expr(
                        condition,
                        symbol,
                        params,
                        returns,
                        calling_convention,
                        aggregate_return_abi,
                        found,
                    );
                    update_stmts(
                        body,
                        symbol,
                        params,
                        returns,
                        calling_convention,
                        aggregate_return_abi,
                        found,
                    );
                }
                HirStmt::Try(body, _, catch_body) => {
                    update_stmts(
                        body,
                        symbol,
                        params,
                        returns,
                        calling_convention,
                        aggregate_return_abi,
                        found,
                    );
                    update_stmts(
                        catch_body,
                        symbol,
                        params,
                        returns,
                        calling_convention,
                        aggregate_return_abi,
                        found,
                    );
                }
                HirStmt::Return(None)
                | HirStmt::Break
                | HirStmt::Continue
                | HirStmt::BreakDepth(_)
                | HirStmt::ContinueDepth(_) => {}
            }
        }
    }

    let mut found = false;
    for signature in &mut program.extern_functions {
        if signature.symbol == symbol {
            if param_abis.len() != signature.params.len() {
                return Err(format!(
                    "FFI string ABI metadata for `{symbol}` has {} parameter layouts, expected {}",
                    param_abis.len(),
                    signature.params.len()
                ));
            }
            for (index, (ty, abi)) in signature.params.iter().zip(&param_abis).enumerate() {
                if *abi == FfiStringAbi::PointerLength && *ty != HirType::Str {
                    return Err(format!(
                        "FFI parameter {index} of `{symbol}` uses string pointer-length ABI but is not a string"
                    ));
                }
            }
            if return_abi == FfiStringAbi::PointerLength && signature.ret != HirType::Str {
                return Err(format!(
                    "FFI return of `{symbol}` uses string pointer-length ABI but is not a string"
                ));
            }
            signature.param_string_abis = param_abis.clone();
            signature.return_string_abi = return_abi;
            signature.calling_convention = calling_convention;
            signature.aggregate_return_abi = aggregate_return_abi;
            found = true;
        }
    }
    for function in &mut program.functions {
        update_stmts(
            &mut function.body,
            symbol,
            &param_abis,
            return_abi,
            calling_convention,
            aggregate_return_abi,
            &mut found,
        );
    }
    if found {
        Ok(())
    } else {
        Err(format!(
            "FFI string ABI metadata references unknown ambient function `{symbol}`"
        ))
    }
}

pub fn set_ffi_aggregate_layout(
    program: &mut HirProgram,
    symbol: &str,
    layout: FfiAggregateLayout,
) -> Result<(), String> {
    let Some(signature) = program
        .extern_functions
        .iter_mut()
        .find(|signature| signature.symbol == symbol)
    else {
        return Err(format!(
            "FFI aggregate layout metadata references unknown ambient function `{symbol}`"
        ));
    };
    let HirType::Object(fields) = &signature.ret else {
        return Err(format!(
            "FFI aggregate layout for `{symbol}` requires a fixed object return"
        ));
    };
    if signature.aggregate_return_abi == FfiAggregateAbi::Internal {
        return Err(format!(
            "FFI aggregate layout for `{symbol}` requires a portable or packed aggregate return ABI"
        ));
    }
    fn validate_layout(
        symbol: &str,
        path: &str,
        fields: &[(Symbol, HirType)],
        layout: &FfiAggregateLayout,
        root: bool,
    ) -> Result<(), String> {
        if layout.field_offsets.len() != fields.len()
            || layout.field_layouts.len() != fields.len()
            || layout.field_bitfields.len() != fields.len()
        {
            return Err(format!(
                "FFI aggregate layout for `{symbol}` at `{path}` has {} field offsets, {} field layouts, and {} bitfield entries, expected {} of each",
                layout.field_offsets.len(),
                layout.field_layouts.len(),
                layout.field_bitfields.len(),
                fields.len()
            ));
        }
        if layout.alignment == 0 || !layout.alignment.is_power_of_two() {
            return Err(format!(
                "FFI aggregate layout for `{symbol}` at `{path}` needs a non-zero power-of-two alignment"
            ));
        }
        if layout.size == 0 || !layout.size.is_multiple_of(u64::from(layout.alignment)) {
            return Err(format!(
                "FFI aggregate layout for `{symbol}` at `{path}` needs a non-zero size divisible by its alignment"
            ));
        }
        if !root && layout.indirect {
            return Err(format!(
                "FFI aggregate layout for `{symbol}` at `{path}` requires `indirect: false` for nested objects"
            ));
        }
        if !root && !layout.register_classes.is_empty() {
            return Err(format!(
                "FFI aggregate layout for `{symbol}` at `{path}` cannot declare nested register classes"
            ));
        }
        if root && layout.indirect && !layout.register_classes.is_empty() {
            return Err(format!(
                "FFI aggregate layout for `{symbol}` at `{path}` cannot combine indirect return storage with register classes"
            ));
        }
        if root && !layout.indirect {
            let expected = layout.size.div_ceil(8) as usize;
            if layout.size > 16 || layout.register_classes.len() != expected {
                return Err(format!(
                    "FFI direct aggregate layout for `{symbol}` requires one register class per 8-byte unit and supports at most 16 bytes"
                ));
            }
        }
        let mut previous_end = 0;
        let mut shared_bit_storage = None;
        for (index, ((name, ty), offset)) in fields.iter().zip(&layout.field_offsets).enumerate() {
            let child = layout.field_layouts[index].as_deref();
            let bitfield = layout.field_bitfields[index].as_ref();
            let field_size = match (ty, child, bitfield) {
                (HirType::Bool | HirType::F64, None, Some(bitfield)) => {
                    let storage_bits = u16::from(bitfield.storage_bytes) * 8;
                    if !matches!(bitfield.storage_bytes, 1 | 2 | 4 | 8)
                        || bitfield.bit_width == 0
                        || u16::from(bitfield.bit_offset) + u16::from(bitfield.bit_width)
                            > storage_bits
                    {
                        return Err(format!(
                            "FFI bitfield `{path}.{name}` of `{symbol}` has an invalid bit offset or storage size"
                        ));
                    }
                    if *ty == HirType::Bool && (bitfield.bit_width != 1 || bitfield.signed) {
                        return Err(format!(
                            "FFI boolean bitfield `{path}.{name}` of `{symbol}` requires width 1 and `signed: false`"
                        ));
                    }
                    u64::from(bitfield.storage_bytes)
                }
                (_, _, Some(_)) => {
                    return Err(format!(
                        "FFI bitfield `{path}.{name}` of `{symbol}` requires a boolean field without a nested layout"
                    ))
                }
                (HirType::Bool, None, None) => 1,
                (HirType::F64 | HirType::I64 | HirType::Str, None, None) => 8,
                (HirType::Object(child_fields), Some(child_layout), None) => {
                    validate_layout(
                        symbol,
                        &format!("{path}.{name}"),
                        child_fields,
                        child_layout,
                        false,
                    )?;
                    child_layout.size
                }
                (HirType::Object(_), None, None) => {
                    return Err(format!(
                        "FFI aggregate layout field `{path}.{name}` of `{symbol}` needs a nested field layout"
                    ))
                }
                (_, Some(_), None) => {
                    return Err(format!(
                        "FFI aggregate layout field `{path}.{name}` of `{symbol}` has a nested layout for a non-object type"
                    ))
                }
                (other, None, None) => {
                    return Err(format!(
                        "FFI aggregate layout field `{path}.{name}` of `{symbol}` has unsupported explicit-layout type {other:?}"
                    ))
                }
            };
            let end = offset.saturating_add(field_size);
            let shares_storage = bitfield.is_some() && shared_bit_storage == Some((*offset, end));
            if (*offset < previous_end && !shares_storage) || end > layout.size {
                return Err(format!(
                    "FFI aggregate layout field `{path}.{name}` of `{symbol}` is outside or overlaps the declared size"
                ));
            }
            previous_end = previous_end.max(end);
            shared_bit_storage = bitfield.map(|_| (*offset, end));
        }
        Ok(())
    }

    if layout.field_offsets.len() != fields.len() {
        return Err(format!(
            "FFI aggregate layout for `{symbol}` has {} field offsets, expected {}",
            layout.field_offsets.len(),
            fields.len()
        ));
    }
    validate_layout(symbol, "return", fields, &layout, true)?;
    signature.aggregate_return_layout = Some(layout);
    Ok(())
}

pub fn set_ffi_variadic_abi(
    program: &mut HirProgram,
    symbol: &str,
    abi: FfiVariadicAbi,
) -> Result<(), String> {
    let Some(signature) = program
        .extern_functions
        .iter_mut()
        .find(|signature| signature.symbol == symbol)
    else {
        return Err(format!(
            "FFI variadic ABI metadata references unknown ambient function `{symbol}`"
        ));
    };
    if abi != FfiVariadicAbi::Native && signature.variadic != Some(HirType::F64) {
        return Err(format!(
            "FFI integer variadic ABI for `{symbol}` requires a number[] rest parameter"
        ));
    }
    signature.variadic_abi = abi;
    Ok(())
}
