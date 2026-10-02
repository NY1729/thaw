use super::*;
use std::collections::BTreeSet;

/// The set of pre-existing variable names a `while` loop's `cond`
/// expression or `body` statements capture into a nested closure (an
/// `HirExpr::Lambda`'s own `captures` list) -- not every bare variable
/// reference, and not recursing into a closure's own body, matching
/// `HirExpr::Lambda`'s own invariant: everything the closure body needs
/// from its enclosing scope is already flattened into this list by
/// lowering (see `crates/thaw-hir/src/lower/expressions/functions.rs`).
///
/// `compile_while` (thaw-llvm) needs this to eagerly promote every such
/// variable to a shared arena cell *before* compiling either side --
/// otherwise, whichever side's own closure happens to compile first
/// reactively promotes the variable on first capture, leaving the
/// *other* side (compiled from a copy taken before that promotion)
/// reading a stale, un-promoted cell. This is round24's `.test()`/
/// `.lastIndex` staleness bug: a naive fix that simply reordered
/// `compile_while` to compile `cond` before `body` was tried and
/// reverted (round38) after it broke `for...of` over a sparse array,
/// whose own index-normalization closure triggers the identical
/// promotion from the opposite side -- the real fix needs both sides'
/// captures known up front, regardless of compile order.
pub fn closure_captured_names_in_while(cond: &HirExpr, body: &[HirStmt]) -> BTreeSet<Symbol> {
    let mut names = BTreeSet::new();
    collect_closure_captures_expr(cond, &mut names);
    for stmt in body {
        collect_closure_captures_stmt(stmt, &mut names);
    }
    names
}

fn collect_closure_captures_expr(expr: &HirExpr, names: &mut BTreeSet<Symbol>) {
    match expr {
        HirExpr::Lambda(captures, _, _, _) => {
            names.extend(captures.iter().map(|capture| capture.name.clone()));
        }
        HirExpr::Var(_) | HirExpr::PostfixUpdate(_, _) => {}
        HirExpr::Assign(_, value) => collect_closure_captures_expr(value, names),
        HirExpr::BinOp(_, left, right)
        | HirExpr::EvalThen(left, right)
        | HirExpr::UnionMemberIsEqual(left, right, _, _)
        | HirExpr::UnionIsEqual(left, right, _)
        | HirExpr::Index(left, right)
        | HirExpr::TypedIndex(left, right, _)
        | HirExpr::ArraySetLen(left, right, _)
        | HirExpr::DynamicPropAccess(left, right, _, _)
        | HirExpr::JsonKey(left, right)
        | HirExpr::JsonDelete(left, right) => {
            collect_closure_captures_expr(left, names);
            collect_closure_captures_expr(right, names);
        }
        HirExpr::Conditional(test, consequent, alternate, _) => {
            collect_closure_captures_expr(test, names);
            collect_closure_captures_expr(consequent, names);
            collect_closure_captures_expr(alternate, names);
        }
        HirExpr::JsonSet(object, key, value, _, _) | HirExpr::JsonIndexSet(object, key, value) => {
            collect_closure_captures_expr(object, names);
            collect_closure_captures_expr(key, names);
            collect_closure_captures_expr(value, names);
        }
        HirExpr::Call(callee, args) => {
            collect_closure_captures_expr(callee, names);
            for arg in args {
                collect_closure_captures_expr(arg, names);
            }
        }
        HirExpr::FunctionCallWithThis(callee, this_arg, args, _, _)
        | HirExpr::FunctionBindThis(callee, this_arg, args, _, _) => {
            collect_closure_captures_expr(callee, names);
            collect_closure_captures_expr(this_arg, names);
            for arg in args {
                collect_closure_captures_expr(arg, names);
            }
        }
        HirExpr::Await(value)
        | HirExpr::AwaitPromise(value, _)
        | HirExpr::PromiseNew(value, _, _, _)
        | HirExpr::PromiseNewMixed(value, _, _)
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
        | HirExpr::JsonAsNative(value, _)
        | HirExpr::JsValueAsJson(value)
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
        | HirExpr::NullishValue(value, _) => collect_closure_captures_expr(value, names),
        HirExpr::RecursiveClosure(_, _, closure) => collect_closure_captures_expr(closure, names),
        HirExpr::TypedClosure(_, closure) | HirExpr::NonArrowFunction(closure) => collect_closure_captures_expr(closure, names),
        HirExpr::PromiseThen(source, callback, _, _, _, _) => {
            collect_closure_captures_expr(source, names);
            collect_closure_captures_expr(callback, names);
        }
        HirExpr::PromiseFinally(source, callback, _, _) => {
            collect_closure_captures_expr(source, names);
            collect_closure_captures_expr(callback, names);
        }
        HirExpr::ThrowValue(error, fallback) => {
            collect_closure_captures_expr(error, names);
            collect_closure_captures_expr(fallback, names);
        }
        HirExpr::Block(stmts) => {
            for stmt in stmts {
                collect_closure_captures_stmt(stmt, names);
            }
        }
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
                collect_closure_captures_expr(arg, names);
            }
        }
        HirExpr::IndexAssign(array, index, value) => {
            collect_closure_captures_expr(array, names);
            collect_closure_captures_expr(index, names);
            collect_closure_captures_expr(value, names);
        }
        HirExpr::ObjectLit(fields) | HirExpr::JsonObjectLit(fields, _) => {
            for (_, value) in fields {
                collect_closure_captures_expr(value, names);
            }
        }
        HirExpr::PropAccess(object, _, _) | HirExpr::JsonGet(object, _) => {
            collect_closure_captures_expr(object, names);
        }
        HirExpr::PropAssign(object, _, _, value) | HirExpr::JsonIndex(object, value) => {
            collect_closure_captures_expr(object, names);
            collect_closure_captures_expr(value, names);
        }
        HirExpr::Lit(_)
        | HirExpr::OptionalNone(_)
        | HirExpr::NullableNone(_)
        | HirExpr::NullishNull(_)
        | HirExpr::NullishUndefined(_)
        | HirExpr::EnvVar(_)
        | HirExpr::ObjectAlloc(_)
        | HirExpr::ClassAlloc(_)
        | HirExpr::FunctionRef(..)
        | HirExpr::MethodRef(..) => {}
    }
}

fn collect_closure_captures_stmt(stmt: &HirStmt, names: &mut BTreeSet<Symbol>) {
    match stmt {
        HirStmt::Expr(expr) | HirStmt::Throw(expr) => collect_closure_captures_expr(expr, names),
        HirStmt::Let(_, _, expr) => collect_closure_captures_expr(expr, names),
        HirStmt::Return(Some(expr)) => collect_closure_captures_expr(expr, names),
        HirStmt::If(cond, then_body, else_body) => {
            collect_closure_captures_expr(cond, names);
            for stmt in then_body {
                collect_closure_captures_stmt(stmt, names);
            }
            for stmt in else_body {
                collect_closure_captures_stmt(stmt, names);
            }
        }
        HirStmt::While(cond, body) => {
            collect_closure_captures_expr(cond, names);
            for stmt in body {
                collect_closure_captures_stmt(stmt, names);
            }
        }
        HirStmt::Try(body, _, catch_body, _) => {
            for stmt in body {
                collect_closure_captures_stmt(stmt, names);
            }
            for stmt in catch_body {
                collect_closure_captures_stmt(stmt, names);
            }
        }
        HirStmt::Return(None)
        | HirStmt::Break
        | HirStmt::Continue
        | HirStmt::BreakDepth(_)
        | HirStmt::ContinueDepth(_) => {}
    }
}
