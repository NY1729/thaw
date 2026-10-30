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


thread_local! {
    // Names (variables, parameters, globals) declared `Bytes`, and functions returning
    // `Bytes`, as seen so far in the traversal; the indexing fixups below read them.
    static BYTES_NAMES: std::cell::RefCell<std::collections::HashSet<String>> = Default::default();
    static BYTES_FUNCTIONS: std::cell::RefCell<std::collections::HashSet<String>> = Default::default();
}

fn note_bytes_name(name: &str, ty: &HirType) {
    BYTES_NAMES.with(|names| {
        if *ty == HirType::Bytes {
            names.borrow_mut().insert(name.to_string());
        } else {
            names.borrow_mut().remove(name);
        }
    });
}

/// Whether `expr` (not yet erased) is a byte buffer, from declared types alone.
fn is_bytes_expr(expr: &HirExpr) -> bool {
    match expr {
        HirExpr::Var(name) => BYTES_NAMES.with(|names| names.borrow().contains(name)),
        HirExpr::PropAccess(_, HirType::Object(fields), field) => {
            fields.iter().any(|(name, ty)| name == field && *ty == HirType::Bytes)
        }
        HirExpr::TypedIndex(_, _, ty)
        | HirExpr::Conditional(_, _, _, ty)
        | HirExpr::OptionalValue(_, ty)
        | HirExpr::NullableValue(_, ty)
        | HirExpr::NullishValue(_, ty)
        | HirExpr::JsonAsNative(_, ty) => *ty == HirType::Bytes,
        HirExpr::UnionValue(_, index, types) => types.get(*index) == Some(&HirType::Bytes),
        HirExpr::DynamicCall(sig, _) => sig.ret == HirType::Bytes,
        HirExpr::FunctionCallWithThis(_, _, _, _, ret) => *ret == HirType::Bytes,
        HirExpr::EvalThen(_, second) => is_bytes_expr(second),
        HirExpr::Block(stmts) => matches!(stmts.last(), Some(HirStmt::Return(Some(value))) if is_bytes_expr(value)),
        HirExpr::Call(callee, _) => match callee.as_ref() {
            HirExpr::Var(name) => {
                matches!(
                    name.as_str(),
                    "__thaw_bytes_from_string" | "__thaw_bytes_alloc" | "__thaw_bytes_alloc_fill"
                        | "__thaw_arraybuffer_alloc" | "__thaw_dataview_new" | "__thaw_arraybuffer_transfer"
                        | "__thaw_bytes_from_array" | "__thaw_bytes_concat" | "__thaw_bytes_slice"
                        | "__thaw_bytes_subarray" | "__thaw_bytes_clone" | "__thaw_bytes_as_u8"
                        | "__thaw_bytes_narrow_list" | "__thaw_bytes_buffer"
                        | "__thaw_bytes_from_buffer_like"
                ) || BYTES_FUNCTIONS.with(|names| names.borrow().contains(name))
            }
            HirExpr::Lambda(_, _, ret, _) => *ret == HirType::Bytes,
            _ => false,
        },
        _ => false,
    }
}

fn erase_bytes(program: &mut HirProgram) {
    BYTES_NAMES.with(|names| names.borrow_mut().clear());
    BYTES_FUNCTIONS.with(|names| {
        *names.borrow_mut() = program
            .functions
            .iter()
            .filter(|function| function.ret == HirType::Bytes)
            .map(|function| function.name.to_string())
            .collect();
    });
    for global in &program.globals {
        note_bytes_name(&global.name, &global.ty);
    }
    for global in &mut program.globals {
        erase_global(global);
    }
    for step in &mut program.initializers {
        match step {
            HirInitStep::StoreGlobal(_, value) => erase_expr(value),
            HirInitStep::Statement(stmt) => erase_stmt(stmt),
            HirInitStep::ExecutionBoundary(_) | HirInitStep::ModuleBoundary { .. } => {}
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
    note_bytes_name(&param.name, &param.ty);
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
        erase_dynamic_ty(ty);
    }
    erase_dynamic_ty(&mut sig.ret);
}

// Dynamic calls need the marker to select the Buffer JSON wire shape.
// The value itself still has the same native layout as `number[]`.
fn erase_dynamic_ty(ty: &mut HirType) {
    match ty {
        HirType::Bytes => {}
        HirType::Array(inner)
        | HirType::Promise(inner)
        | HirType::Dictionary(inner)
        | HirType::Set(inner)
        | HirType::WeakSet(inner)
        | HirType::Optional(inner)
        | HirType::Nullable(inner)
        | HirType::Nullish(inner) => erase_dynamic_ty(inner),
        HirType::Map(key, value) | HirType::WeakMap(key, value) => {
            erase_dynamic_ty(key);
            erase_dynamic_ty(value);
        }
        HirType::Tuple(elements) | HirType::Union(elements) => {
            for element in elements {
                erase_dynamic_ty(element);
            }
        }
        HirType::Object(fields) => {
            for (_, field) in fields {
                erase_dynamic_ty(field);
            }
        }
        HirType::Function(params, ret) => {
            for param in params {
                erase_dynamic_ty(param);
            }
            erase_dynamic_ty(ret);
        }
        HirType::CallableFunction(params, _, rest, ret) => {
            for param in params {
                erase_dynamic_ty(param);
            }
            if let Some(rest) = rest {
                erase_dynamic_ty(rest);
            }
            erase_dynamic_ty(ret);
        }
        HirType::OverloadedFunction(public, physical) => {
            for signature in public {
                erase_dynamic_ty(signature);
            }
            erase_dynamic_ty(physical);
        }
        _ => {}
    }
}

fn erase_ty(ty: &mut HirType) {
    match ty {
        HirType::Bytes => *ty = HirType::Array(Box::new(HirType::F64)),
        HirType::Array(inner)
        | HirType::Promise(inner)
        | HirType::Dictionary(inner)
        | HirType::Set(inner)
        | HirType::WeakSet(inner)
        | HirType::Optional(inner)
        | HirType::Nullable(inner)
        | HirType::Nullish(inner) => erase_ty(inner),
        HirType::Map(key, value) | HirType::WeakMap(key, value) => {
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
        HirType::FunctionWithThis(receiver, signature) => {
            erase_ty(receiver);
            erase_ty(signature);
        }
        HirType::NonArrowReceiver(receiver) => erase_ty(receiver),
        HirType::CallableFunction(params, _, rest, ret) => {
            for param in params {
                erase_ty(param);
            }
            if let Some(rest) = rest {
                erase_ty(rest);
            }
            erase_ty(ret);
        }
        HirType::OverloadedFunction(public, physical) => {
            for signature in public {
                erase_ty(signature);
            }
            erase_ty(physical);
        }
        HirType::Recursive(_)
        | HirType::F64
        | HirType::I64 | HirType::BigInt
        | HirType::Bool
        | HirType::Undefined
        | HirType::Null
        | HirType::Void
        | HirType::Str
        | HirType::Symbol
        | HirType::StrLiteral(_)
        | HirType::Json
        | HirType::JsValue
        | HirType::NativeException
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
        HirStmt::Let(name, ty, value) => {
            erase_expr(value);
            note_bytes_name(name, ty);
            erase_ty(ty);
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
        HirStmt::Finally(body, _) => erase_stmts(body),
        HirStmt::Try(body, _, handler, _) => {
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

// The rewrites below live in their own never-inlined functions: `erase_expr` recurses
// once per HIR nesting level, so its stack frame must stay small.

/// `buffer[i]` / `buffer[i] = v` on a byte buffer: the bounds-checked view accessors.
#[inline(never)]
fn erase_bytes_element_access(expr: &mut HirExpr) {
    let taken = std::mem::replace(expr, HirExpr::Lit(HirLit::Undefined));
    *expr = match taken {
        HirExpr::Index(mut object, mut index) | HirExpr::TypedIndex(mut object, mut index, _) => {
            erase_expr(&mut object);
            erase_expr(&mut index);
            HirExpr::Call(Box::new(HirExpr::Var("__thaw_bytes_get_safe".into())), vec![*object, *index])
        }
        HirExpr::IndexAssign(mut object, mut index, mut value) => {
            erase_expr(&mut object);
            erase_expr(&mut index);
            erase_expr(&mut value);
            HirExpr::Call(
                Box::new(HirExpr::Var("__thaw_bytes_set_safe".into())),
                vec![*object, *index, *value],
            )
        }
        other => other,
    };
}

/// A dynamic value decodes into an `f64`-slot array; a `Buffer` is then narrowed into a
/// byte view record.
#[inline(never)]
fn erase_json_as_native(expr: &mut HirExpr) {
    let HirExpr::JsonAsNative(value, ty) = expr else {
        return;
    };
    erase_expr(value);
    let narrow = *ty == HirType::Bytes;
    erase_ty(ty);
    if narrow {
        let decoded = std::mem::replace(expr, HirExpr::Lit(HirLit::Undefined));
        *expr = HirExpr::Call(Box::new(HirExpr::Var("__thaw_bytes_from_array".into())), vec![decoded]);
    }
}

/// Native C code reads `f64`-slot arrays: a `Buffer` argument is widened, a `Buffer`
/// result narrowed.
#[inline(never)]
fn erase_ffi_call(expr: &mut HirExpr) {
    let HirExpr::FfiCall(sig, args) = expr else {
        return;
    };
    for (argument, ty) in args.iter_mut().zip(&sig.params) {
        if *ty == HirType::Bytes {
            let value = std::mem::replace(argument, HirExpr::Lit(HirLit::Undefined));
            *argument = HirExpr::Call(Box::new(HirExpr::Var("__thaw_bytes_to_array".into())), vec![value]);
        }
    }
    let narrow = sig.ret == HirType::Bytes;
    erase_ffi(sig);
    erase_exprs(args);
    if narrow {
        let call = std::mem::replace(expr, HirExpr::Lit(HirLit::Undefined));
        *expr = HirExpr::Call(Box::new(HirExpr::Var("__thaw_bytes_from_array".into())), vec![call]);
    }
}

fn erase_expr(expr: &mut HirExpr) {
    match expr {
        HirExpr::Lit(_)
        | HirExpr::Var(_)
        | HirExpr::EnvVar(_)
        | HirExpr::PostfixUpdate(_, _) => {}

        HirExpr::Index(left, _) if is_bytes_expr(left) => erase_bytes_element_access(expr),
        HirExpr::BinOp(_, left, right)
        | HirExpr::EvalThen(left, right)
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
        | HirExpr::AwaitPromise(value, ty) => {
            erase_expr(value);
            erase_ty(ty);
        }

        HirExpr::JsonAsNative(..) => erase_json_as_native(expr),

        HirExpr::OptionalNone(ty)
        | HirExpr::NullableNone(ty)
        | HirExpr::NullishNull(ty)
        | HirExpr::NullishUndefined(ty)
        | HirExpr::ObjectAlloc(ty)
        | HirExpr::ClassAlloc(ty) => erase_ty(ty),

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

        // `console.log([buf])` needs the `Bytes` marker to print `<Buffer ..>` for nested buffers.
        HirExpr::Call(callee, args)
            if matches!(callee.as_ref(), HirExpr::Var(name) if name == "__thaw_console_typed")
                && matches!(args.as_slice(), [HirExpr::TypedClosure(..)]) =>
        {
            if let [HirExpr::TypedClosure(ty, inner)] = args.as_mut_slice() {
                erase_dynamic_ty(ty);
                erase_expr(inner);
            }
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
        HirExpr::PromiseNew(value, ty, _, _) => {
            erase_expr(value);
            erase_ty(ty);
        }
        HirExpr::PromiseNewMixed(value, ty, resolver) => {
            erase_expr(value);
            erase_ty(ty);
            erase_ty(resolver);
        }
        HirExpr::PromiseThen(source, callback, a, b, _, _) => {
            erase_expr(source);
            erase_expr(callback);
            erase_ty(a);
            erase_ty(b);
        }
        HirExpr::PromiseThenBoth(source, fulfilled, rejected, input, output, _, _) => {
            erase_expr(source);
            if let Some(fulfilled) = fulfilled {
                erase_expr(fulfilled);
            }
            erase_expr(rejected);
            erase_ty(input);
            erase_ty(output);
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
        HirExpr::NonArrowFunction(closure) => erase_expr(closure),
        HirExpr::NonArrowThis { payload, declared, .. } => {
            erase_expr(payload);
            erase_ty(declared);
        }
        HirExpr::NonArrowReceiverPack { value, declared } => {
            erase_expr(value);
            erase_ty(declared);
        }
        HirExpr::FunctionRef(_, types, ret) | HirExpr::MethodRef(_, _, types, ret, _, _) => {
            erase_tys(types);
            erase_ty(ret);
        }
        HirExpr::FunctionRefThis(_, receiver, types, ret) => {
            erase_ty(receiver);
            erase_tys(types);
            erase_ty(ret);
        }
        HirExpr::Block(stmts) => erase_stmts(stmts),

        HirExpr::FfiCall(..) => erase_ffi_call(expr),
        HirExpr::DynamicCall(sig, args) => {
            erase_dyn(sig);
            erase_exprs(args);
        }

        HirExpr::Assign(_, value) => erase_expr(value),

        HirExpr::ArrayLit(items) => erase_exprs(items),

        HirExpr::TypedIndex(object, _, ty) if *ty == HirType::F64 && is_bytes_expr(object) => {
            erase_bytes_element_access(expr)
        }
        HirExpr::TypedIndex(object, index, ty) => {
            erase_expr(object);
            erase_expr(index);
            erase_ty(ty);
        }
        HirExpr::IndexAssign(object, _, _) if is_bytes_expr(object) => erase_bytes_element_access(expr),
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
        // The Json encoder widens a `Bytes` value itself, so the marker stays.
        HirExpr::JsonSet(object, key, value, ty, _) => {
            erase_expr(object);
            erase_expr(key);
            erase_expr(value);
            erase_dynamic_ty(ty);
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

#[cfg(test)]
mod thaw_remaining_traversals_bytes_controls {
    use super::*;

    #[test]
    fn thaw_remaining_bytes_erasure_visits_this_signature_and_callable_reference_types() {
        let mut program = HirProgram {
            globals: vec![crate::HirGlobal {
                name: "callable".into(),
                ty: HirType::FunctionWithThis(
                    Box::new(HirType::Bytes),
                    Box::new(HirType::Function(
                        vec![HirType::Bytes],
                        Box::new(HirType::Bytes),
                    )),
                ),
                init: HirExpr::FunctionRefThis(
                    "callable_with_this".into(),
                    HirType::Bytes,
                    vec![HirType::Bytes],
                    HirType::Bytes,
                ),
                mutable: false,
            }],
            ..HirProgram::default()
        };

        erase_bytes(&mut program);

        let bytes_array = || HirType::Array(Box::new(HirType::F64));
        assert_eq!(
            program.globals[0].ty,
            HirType::FunctionWithThis(
                Box::new(bytes_array()),
                Box::new(HirType::Function(
                    vec![bytes_array()],
                    Box::new(bytes_array()),
                )),
            )
        );
        let HirExpr::FunctionRefThis(_, receiver, params, ret) = &program.globals[0].init else {
            panic!("expected function reference with explicit receiver");
        };
        assert_eq!(receiver, &bytes_array());
        assert_eq!(params, &vec![bytes_array()]);
        assert_eq!(ret, &bytes_array());
    }

    #[test]
    fn thaw_remaining_native_exception_is_preserved_as_an_opaque_type_leaf() {
        let mut ty = HirType::NativeException;

        erase_ty(&mut ty);

        assert_eq!(ty, HirType::NativeException);
    }
}
