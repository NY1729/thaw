/// An assignment target used by assignment and update lowering.
#[derive(Clone)]
enum Target {
    Var(Symbol),
    Index(HirExpr, Box<HirExpr>),
    Prop(HirExpr, HirType, Symbol),
    Dictionary(HirExpr, Box<HirExpr>, HirType),
    JsonIndex(HirExpr, Box<HirExpr>),
    DynamicProperty(HirExpr, Box<HirExpr>),
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

fn target_to_read_expr(target: &Target) -> Result<HirExpr, String> {
    Ok(match target {
        Target::Var(name) => HirExpr::Var(name.clone()),
        Target::Index(arr, idx) => HirExpr::Index(Box::new(arr.clone()), idx.clone()),
        Target::Prop(obj, ty, field) => {
            HirExpr::PropAccess(Box::new(obj.clone()), ty.clone(), field.clone())
        }
        Target::Dictionary(object, key, element) => match element {
            HirType::F64 => HirExpr::JsonAsNumber(Box::new(HirExpr::JsonKey(
                Box::new(object.clone()),
                key.clone(),
            ))),
            HirType::Str => HirExpr::JsonAsString(Box::new(HirExpr::JsonKey(
                Box::new(object.clone()),
                key.clone(),
            ))),
            HirType::Bool => HirExpr::JsonAsBool(Box::new(HirExpr::JsonKey(
                Box::new(object.clone()),
                key.clone(),
            ))),
            HirType::Json => HirExpr::JsonKey(Box::new(object.clone()), key.clone()),
            other => return Err(format!("unsupported dictionary value type {other:?}")),
        },
        Target::JsonIndex(object, index) => {
            HirExpr::JsonIndex(Box::new(object.clone()), index.clone())
        }
        Target::DynamicProperty(object, key) => HirExpr::Call(
            Box::new(HirExpr::Var("getDynamicProperty".into())),
            vec![object.clone(), key.as_ref().clone()],
        ),
    })
}

fn build_assign(target: Target, value: HirExpr) -> HirExpr {
    match target {
        Target::Var(name) => HirExpr::Assign(name, Box::new(value)),
        Target::Index(arr, idx) => HirExpr::IndexAssign(Box::new(arr), idx, Box::new(value)),
        Target::Prop(obj, ty, field) => {
            HirExpr::PropAssign(Box::new(obj), ty, field, Box::new(value))
        }
        Target::Dictionary(object, key, element) => {
            HirExpr::JsonSet(Box::new(object), key, Box::new(value), element, false)
        }
        Target::JsonIndex(object, index) => {
            HirExpr::JsonIndexSet(Box::new(object), index, Box::new(value))
        }
        Target::DynamicProperty(object, key) => HirExpr::Call(
            Box::new(HirExpr::Var("setDynamicProperty".into())),
            vec![object, *key, value],
        ),
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
        HirExpr::PostfixUpdate(name, _) => {
            names.insert(name.clone());
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
            collect_referenced_bindings(left, names);
            collect_referenced_bindings(right, names);
        }
        HirExpr::Conditional(test, consequent, alternate, _) => {
            collect_referenced_bindings(test, names);
            collect_referenced_bindings(consequent, names);
            collect_referenced_bindings(alternate, names);
        }
        HirExpr::JsonSet(object, key, value, _, _)
        | HirExpr::JsonIndexSet(object, key, value) => {
            collect_referenced_bindings(object, names);
            collect_referenced_bindings(key, names);
            collect_referenced_bindings(value, names);
        }
        HirExpr::Call(callee, args) => {
            collect_referenced_bindings(callee, names);
            for arg in args {
                collect_referenced_bindings(arg, names);
            }
        }
        HirExpr::FunctionCallWithThis(callee, this_arg, args, _, _) => {
            collect_referenced_bindings(callee, names);
            collect_referenced_bindings(this_arg, names);
            for arg in args {
                collect_referenced_bindings(arg, names);
            }
        }
        HirExpr::FunctionBindThis(callee, this_arg, args, _, _) => {
            collect_referenced_bindings(callee, names);
            collect_referenced_bindings(this_arg, names);
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
        | HirExpr::JsonAsNative(value, _)
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
        HirExpr::RecursiveClosure(_, _, closure) => collect_referenced_bindings(closure, names),
        HirExpr::TypedClosure(_, closure) => collect_referenced_bindings(closure, names),
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
        HirExpr::ThrowValue(error, fallback) => {
            collect_referenced_bindings(error, names);
            collect_referenced_bindings(fallback, names);
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
        HirExpr::JsonObjectLit(fields, _) => {
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
        | HirExpr::FunctionRef(..)
        | HirExpr::MethodRef(..) => {}
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
        | HirExpr::JsonIndex(left, right)
        | HirExpr::JsonKey(left, right)
        | HirExpr::JsonDelete(left, right) => contains_await(left) || contains_await(right),
        HirExpr::Conditional(test, consequent, alternate, _) => {
            contains_await(test) || contains_await(consequent) || contains_await(alternate)
        }
        HirExpr::JsonSet(object, key, value, _, _)
        | HirExpr::JsonIndexSet(object, key, value) => {
            contains_await(object) || contains_await(key) || contains_await(value)
        }
        HirExpr::Call(callee, args) => contains_await(callee) || args.iter().any(contains_await),
        HirExpr::FunctionCallWithThis(callee, this_arg, args, _, _) => {
            contains_await(callee) || contains_await(this_arg) || args.iter().any(contains_await)
        }
        HirExpr::FunctionBindThis(callee, this_arg, args, _, _) => {
            contains_await(callee) || contains_await(this_arg) || args.iter().any(contains_await)
        }
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
        | HirExpr::JsonAsNative(value, _)
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
        HirExpr::ObjectLit(fields) | HirExpr::JsonObjectLit(fields, _) => {
            fields.iter().any(|(_, value)| contains_await(value))
        }
        HirExpr::ThrowValue(error, fallback) => contains_await(error) || contains_await(fallback),
        HirExpr::Block(stmts) => stmts.iter().any(stmt_contains_await),
        // A closure body runs only when the closure is invoked, not when the
        // function value is evaluated at this expression boundary.
        HirExpr::RecursiveClosure(..)
        | HirExpr::TypedClosure(..)
        | HirExpr::Lambda(..)
        | HirExpr::FunctionRef(..)
        | HirExpr::MethodRef(..)
        | HirExpr::Lit(_)
        | HirExpr::OptionalNone(_)
        | HirExpr::NullableNone(_)
        | HirExpr::NullishNull(_)
        | HirExpr::NullishUndefined(_)
        | HirExpr::Var(_)
        | HirExpr::EnvVar(_)
        | HirExpr::ObjectAlloc(_)
        | HirExpr::PostfixUpdate(_, _) => false,
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

fn async_arrow_has_only_tail_await_returns(statements: &[HirStmt]) -> bool {
    fn visit(statements: &[HirStmt], saw_tail_await: &mut bool) -> bool {
        statements.iter().all(|statement| match statement {
            HirStmt::Return(Some(HirExpr::AwaitPromise(_, _) | HirExpr::Await(_))) => {
                *saw_tail_await = true;
                true
            }
            HirStmt::Return(_) => false,
            HirStmt::If(condition, then_body, else_body) => {
                !contains_await(condition)
                    && visit(then_body, saw_tail_await)
                    && visit(else_body, saw_tail_await)
            }
            HirStmt::While(condition, body) => {
                !contains_await(condition) && visit(body, saw_tail_await)
            }
            // Adopting a returned promise would move its rejection outside the
            // surrounding catch, so try/catch needs the general async frame path.
            HirStmt::Try(body, _, catch) => {
                !body.iter().any(stmt_contains_await) && !catch.iter().any(stmt_contains_await)
            }
            other => !stmt_contains_await(other),
        })
    }

    let mut saw_tail_await = false;
    visit(statements, &mut saw_tail_await) && saw_tail_await
}

fn strip_async_arrow_tail_awaits(statements: Vec<HirStmt>) -> Vec<HirStmt> {
    statements
        .into_iter()
        .map(|statement| match statement {
            HirStmt::Return(Some(HirExpr::AwaitPromise(promise, _))) => {
                HirStmt::Return(Some(*promise))
            }
            HirStmt::Return(Some(HirExpr::Await(promise))) => HirStmt::Return(Some(*promise)),
            HirStmt::If(condition, then_body, else_body) => HirStmt::If(
                condition,
                strip_async_arrow_tail_awaits(then_body),
                strip_async_arrow_tail_awaits(else_body),
            ),
            HirStmt::While(condition, body) => {
                HirStmt::While(condition, strip_async_arrow_tail_awaits(body))
            }
            other => other,
        })
        .collect()
}

fn rewrite_async_arrow_returns(
    statements: Vec<HirStmt>,
    resolve: &str,
    resolved: &HirType,
) -> Result<Vec<HirStmt>, String> {
    let mut rewritten = Vec::new();
    for statement in statements {
        match statement {
            HirStmt::Return(value) => {
                let arguments = match (value, resolved) {
                    (Some(value), _) => vec![value],
                    (None, HirType::Void) => Vec::new(),
                    (None, _) => return Err("non-void async arrow return needs a value".into()),
                };
                rewritten.push(HirStmt::Expr(HirExpr::Call(
                    Box::new(HirExpr::Var(resolve.to_string())),
                    arguments,
                )));
                rewritten.push(HirStmt::Return(None));
            }
            HirStmt::If(condition, then_body, else_body) => rewritten.push(HirStmt::If(
                condition,
                rewrite_async_arrow_returns(then_body, resolve, resolved)?,
                rewrite_async_arrow_returns(else_body, resolve, resolved)?,
            )),
            HirStmt::While(condition, body) => rewritten.push(HirStmt::While(
                condition,
                rewrite_async_arrow_returns(body, resolve, resolved)?,
            )),
            HirStmt::Try(body, binding, catch) => rewritten.push(HirStmt::Try(
                rewrite_async_arrow_returns(body, resolve, resolved)?,
                binding,
                rewrite_async_arrow_returns(catch, resolve, resolved)?,
            )),
            other => rewritten.push(other),
        }
    }
    Ok(rewritten)
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
    inject_before_target_continue(stmts, 0, &[HirStmt::Expr(update.clone())])
}

fn inject_for_advance_before_continue(
    stmts: Vec<HirStmt>,
    advance: &[HirStmt],
) -> Vec<HirStmt> {
    inject_before_target_continue(stmts, 0, advance)
}

fn inject_before_target_continue(
    stmts: Vec<HirStmt>,
    nested_depth: usize,
    injected: &[HirStmt],
) -> Vec<HirStmt> {
    let mut out = Vec::new();
    for stmt in stmts {
        match stmt {
            HirStmt::Continue => {
                if nested_depth == 0 {
                    out.extend_from_slice(injected);
                }
                out.push(HirStmt::Continue);
            }
            HirStmt::ContinueDepth(depth) => {
                if depth == nested_depth {
                    out.extend_from_slice(injected);
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
    inject_before_target_continue(stmts, 0, std::slice::from_ref(guard))
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
        HirType::F64 => Some("number"),
        HirType::I64 => Some("bigint"),
        HirType::Undefined => Some("undefined"),
        HirType::Null => Some("object"),
        HirType::Str | HirType::StrLiteral(_) => Some("string"),
        HirType::Symbol => Some("symbol"),
        HirType::Bool => Some("boolean"),
        HirType::Function(_, _) => Some("function"),
        HirType::CallableFunction(..) => Some("function"),
        HirType::Array(_)
        | HirType::Bytes
        | HirType::Tuple(_)
        | HirType::Object(_)
        | HirType::Json
        | HirType::Dictionary(_)
        | HirType::Map(_, _)
        | HirType::Set(_)
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

fn equivalent_union_members(left: &[HirType], right: &[HirType]) -> bool {
    left.len() == right.len()
        && left.iter().all(|member| right.contains(member))
        && right.iter().all(|member| left.contains(member))
}
