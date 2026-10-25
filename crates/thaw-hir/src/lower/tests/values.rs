/// `const err: any = e` on a caught exception must keep the caught value's
/// concrete `Str` type, not coerce it to `Json`. Coercing turned
/// `err.name`/`err.message` into plain JSON key lookups on a JSON string
/// (`undefined`), losing the `__thaw_error_name`/`__thaw_error_message`
/// routing a `Str` value gets. Real trigger: `catch (e) { const err: any =
/// e; err.name }` from an rxjs rejection.
#[test]
fn an_any_annotated_copy_of_a_caught_error_keeps_the_string_error_type() {
    let program = lower(
        r#"
            function main(): void {
                try {
                    throw new Error("boom");
                } catch (e) {
                    const err: any = e;
                    console.log(err.name, err.message);
                }
            }
        "#,
    );
    // The caught value is a carrier union now (not a bare `Str`), so the `any` copy widens
    // the framed-error member through `__thaw_json_receiver_string`, which exposes
    // `name`/`message`/`stack` as Json properties (the former `__thaw_error_name` routing
    // applied to a raw `Str`). The property reads therefore are plain Json gets.
    let source = format!("{:?}", program.functions);
    assert!(
        source.contains("__thaw_json_receiver_string"),
        "the caught-error copy should widen the framed error through the error-aware Json receiver:\n{source}"
    );
    assert!(source.contains("\"name\"") && source.contains("\"message\""), "{source}");
}

#[test]
fn flat_map_snapshots_each_shared_callback_array_inside_the_map_loop() {
    let program = lower(
        r#"
            function flat(): number[] {
                const shared: number[] = [1];
                return [0, 1].flatMap((_, index) => {
                    if (index === 1) shared[0] = 9;
                    return shared;
                });
            }
            function plainMap(): number[][] {
                const shared: number[] = [1];
                return [0, 1].map(() => shared);
            }
            function flatLength(): number[] {
                const shared: number[] = [1];
                return [0, 1].flatMap((_, index) => {
                    if (index === 1) shared[1] = 9;
                    return shared;
                });
            }
            function unionReceiver(values: number[] | string[]): number[] {
                return values.flatMap(() => [1]);
            }
            function spreadArgument(): number[] {
                const callback: (value: number) => number[] = value => [value];
                const args: [(value: number) => number[]] = [callback];
                return [1].flatMap(...args);
            }
            function jsonResult(values: any[]): any[] {
                return values.flatMap(value => value);
            }
            function tupleResult(values: number[]): any[] {
                const callback: (value: number) => [number, number] = value => [value, value];
                return values.flatMap(callback);
            }
            function mixedResult(values: number[]): number[] {
                return values.flatMap(value => value === 0 ? [value] : value);
            }
            function scalarResult(values: number[]): number[] {
                return values.flatMap(value => value);
            }
            function voidResult(values: number[]): undefined[] {
                return values.flatMap(value => { console.log(value); });
            }
        "#,
    );

    fn visit_expr(expr: &HirExpr, visit: &mut impl FnMut(&HirExpr)) {
        visit(expr);
        match expr {
            HirExpr::Call(callee, args) => {
                visit_expr(callee, visit);
                for arg in args { visit_expr(arg, visit); }
            }
            HirExpr::FunctionCallWithThis(callee, this_arg, args, _, _) => {
                visit_expr(callee, visit);
                visit_expr(this_arg, visit);
                for arg in args { visit_expr(arg, visit); }
            }
            HirExpr::Lambda(_, _, _, body) | HirExpr::TypedClosure(_, body)
            | HirExpr::NonArrowFunction(body) | HirExpr::RecursiveClosure(_, _, body)
            | HirExpr::NonArrowThis { payload: body, .. } => visit_expr(body, visit),
            HirExpr::Block(statements) => {
                for statement in statements { visit_stmt(statement, visit); }
            }
            HirExpr::Conditional(test, yes, no, _) => {
                visit_expr(test, visit);
                visit_expr(yes, visit);
                visit_expr(no, visit);
            }
            HirExpr::BinOp(_, left, right) | HirExpr::EvalThen(left, right)
            | HirExpr::Index(left, right) | HirExpr::JsonKey(left, right)
            | HirExpr::JsonIndex(left, right) => {
                visit_expr(left, visit);
                visit_expr(right, visit);
            }
            HirExpr::ArrayLit(values) | HirExpr::DynamicCall(_, values) => {
                for value in values { visit_expr(value, visit); }
            }
            HirExpr::ArrayAlloc(length, _) | HirExpr::ArrayLen(length)
            | HirExpr::Await(length) | HirExpr::AwaitPromise(length, _)
            | HirExpr::TypedIndex(length, _, _)
            | HirExpr::JsValueAsJson(length) | HirExpr::JsonAsNumber(length)
            | HirExpr::JsonAsString(length) | HirExpr::JsonAsBool(length)
            | HirExpr::UnionInject(length, _, _) | HirExpr::UnionTag(length, _)
            | HirExpr::UnionValue(length, _, _) | HirExpr::OptionalSome(length, _)
            | HirExpr::OptionalIsNone(length, _) | HirExpr::OptionalValue(length, _)
            | HirExpr::ThrowValue(length, _) => visit_expr(length, visit),
            HirExpr::IndexAssign(array, index, value) => {
                visit_expr(array, visit);
                visit_expr(index, visit);
                visit_expr(value, visit);
            }
            HirExpr::Assign(_, value) | HirExpr::ArraySetLen(value, _, _) => visit_expr(value, visit),
            _ => {}
        }
    }
    fn visit_stmt(statement: &HirStmt, visit: &mut impl FnMut(&HirExpr)) {
        match statement {
            HirStmt::Expr(expr) | HirStmt::Return(Some(expr)) | HirStmt::Throw(expr)
            | HirStmt::Let(_, _, expr) => visit_expr(expr, visit),
            HirStmt::If(test, yes, no) => {
                visit_expr(test, visit);
                for statement in yes.iter().chain(no) { visit_stmt(statement, visit); }
            }
            HirStmt::While(test, body) => {
                visit_expr(test, visit);
                for statement in body { visit_stmt(statement, visit); }
            }
            HirStmt::Try(body, _, catch, _) => {
                for statement in body.iter().chain(catch) { visit_stmt(statement, visit); }
            }
            _ => {}
        }
    }
    fn has_snapshot_allocation(expr: &HirExpr, element: &HirType) -> bool {
        let mut found = false;
        visit_expr(expr, &mut |node| {
            if matches!(node, HirExpr::ArrayAlloc(_, allocated) if allocated == element) { found = true; }
        });
        found
    }
    fn flat_map_loop(function: &HirFunction) {
        let mut found = false;
        visit_expr(&HirExpr::Block(function.body.clone()), &mut |node| {
            let HirExpr::Block(statements) = node else { return };
            for (position, statement) in statements.iter().enumerate() {
                let HirStmt::While(_, body) = statement else { continue };
                let Some(HirStmt::If(_, present, _)) = body.iter().find(|statement|
                    matches!(statement, HirStmt::If(test, _, _) if matches!(test, HirExpr::Call(callee, _) if matches!(callee.as_ref(), HirExpr::Var(name) if name == "__thaw_array_has_index")))
                ) else { continue };
                let Some((callback_position, callback_type)) = present.iter().enumerate().find_map(|(index, statement)| {
                    match statement {
                        HirStmt::Let(name, ty, _) if name.starts_with("__thaw_flat_map_callback_result_") => Some((index, ty)),
                        _ => None,
                    }
                }) else { continue };
                let Some(HirStmt::Expr(HirExpr::IndexAssign(destination, _, snapshot))) = present.get(callback_position + 1) else { continue };
                let HirExpr::Var(destination_name) = destination.as_ref() else { continue };
                let callback_expr = match &present[callback_position] {
                    HirStmt::Let(_, _, expr) => expr,
                    _ => unreachable!(),
                };
                let mut outer_callback = None;
                visit_expr(callback_expr, &mut |node| {
                    if let HirExpr::Call(callee, _) = node {
                        if let HirExpr::Var(name) = callee.as_ref() {
                            if name.starts_with("__thaw_map_callback_") { outer_callback = Some(name.clone()); }
                        }
                    }
                });
                let outer_callback = outer_callback.expect("user callback binding in callback-result initializer");
                let mut calls = 0;
                for statement in present {
                    match statement {
                        HirStmt::Let(_, _, expr) | HirStmt::Expr(expr) => visit_expr(expr, &mut |node| {
                            if matches!(node, HirExpr::Call(callee, _) if matches!(callee.as_ref(), HirExpr::Var(name) if name == &outer_callback)) { calls += 1; }
                        }),
                        _ => {}
                    }
                }
                assert_eq!(calls, 1, "callback must execute exactly once in the present branch: {present:?}");
                let allocation = statements[..position].iter().find_map(|statement| match statement {
                    HirStmt::Let(name, ty, HirExpr::ArrayAlloc(_, element)) if name == destination_name => Some((ty, element)),
                    _ => None,
                }).expect("flatMap destination allocation before its loop");
                let HirType::Array(snapshot_type) = allocation.0 else { continue };
                let HirType::Array(final_element) = snapshot_type.as_ref() else { continue };
                assert_eq!(allocation.1, snapshot_type.as_ref(), "destination allocates snapshots");
                let expected_snapshot_element = match function.name.as_str() {
                    "jsonResult" | "tupleResult" => HirType::Json,
                    "scalarResult" | "mixedResult" => HirType::Optional(Box::new(HirType::F64)),
                    "voidResult" => HirType::Undefined,
                    _ => HirType::F64,
                };
                assert_eq!(**final_element, expected_snapshot_element,
                    "snapshot retains the actual flatMap element type: {}", function.name);
                if **final_element == HirType::Json {
                    assert!(matches!(snapshot.as_ref(), HirExpr::Call(callee, _) if matches!(callee.as_ref(), HirExpr::Var(name) if name == "__thaw_any_array_flat")), "Json snapshot uses runtime array flattening: {snapshot:?}");
                } else {
                    assert!(has_snapshot_allocation(snapshot, final_element.as_ref()), "snapshot allocation has the final element type: {snapshot:?}");
                }
                let expected_public_element = match function.name.as_str() {
                    "jsonResult" | "tupleResult" => HirType::Json,
                    "scalarResult" | "mixedResult" | "flat" | "flatLength" | "unionReceiver" | "spreadArgument" => HirType::F64,
                    "voidResult" => HirType::Undefined,
                    _ => unreachable!(),
                };
                assert_eq!(function.ret, HirType::Array(Box::new(expected_public_element.clone())),
                    "public return keeps the declared array element type: {}", function.name);
                assert!(matches!(body.get(1), Some(HirStmt::Expr(HirExpr::Assign(_, _)))), "source index advances after the callback and snapshot branch");
                assert!(matches!(present.get(callback_position + 1), Some(HirStmt::Expr(HirExpr::IndexAssign(_, _, _)))));
                match function.name.as_str() {
                    "flat" | "flatLength" | "unionReceiver" | "spreadArgument" => assert_eq!(callback_type, &HirType::Array(Box::new(HirType::F64))),
                    "jsonResult" => assert_eq!(callback_type, &HirType::Optional(Box::new(HirType::Json))),
                    "tupleResult" => assert_eq!(callback_type, &HirType::Tuple(vec![HirType::F64, HirType::F64])),
                    "scalarResult" => assert_eq!(callback_type, &HirType::Optional(Box::new(HirType::F64))),
                    "voidResult" => assert_eq!(callback_type, &HirType::Undefined),
                    "mixedResult" => assert!(matches!(callback_type, HirType::Union(members)
                        if members.contains(&HirType::Optional(Box::new(HirType::F64)))
                            && members.contains(&HirType::Array(Box::new(HirType::Optional(Box::new(HirType::F64))))))),
                    _ => unreachable!(),
                }
                found = true;
            }
        });
        assert!(found, "a flatMap callback loop with an in-loop snapshot: {}", function.name);
    }

    for name in ["flat", "flatLength", "unionReceiver", "spreadArgument", "jsonResult", "tupleResult", "mixedResult", "scalarResult", "voidResult"] {
        let function = program.functions.iter().find(|function| function.name == name).unwrap();
        flat_map_loop(function);
    }
    let plain_map = program.functions.iter().find(|function| function.name == "plainMap").unwrap();
    assert_eq!(plain_map.ret, HirType::Array(Box::new(HirType::Array(Box::new(HirType::F64)))));
    let mut direct_map = false;
    visit_expr(&HirExpr::Block(plain_map.body.clone()), &mut |node| {
        let HirExpr::Block(statements) = node else { return };
        for statement in statements {
            let HirStmt::While(_, body) = statement else { continue };
            let Some(HirStmt::If(test, present, absent)) = body.first() else { continue };
            if !matches!(test, HirExpr::Call(callee, _) if matches!(callee.as_ref(), HirExpr::Var(name) if name == "__thaw_array_has_index")) { continue; }
            assert!(matches!(absent.first(), Some(HirStmt::Expr(HirExpr::Call(callee, _))) if matches!(callee.as_ref(), HirExpr::Var(name) if name == "__thaw_array_set_hole")));
            let HirStmt::Expr(HirExpr::IndexAssign(_, _, value)) = present.last().unwrap() else { continue };
            assert!(matches!(value.as_ref(), HirExpr::Call(callee, _) if matches!(callee.as_ref(), HirExpr::Var(name) if name.starts_with("__thaw_map_callback_"))));
            assert!(!present.iter().any(|statement| matches!(statement, HirStmt::Let(name, _, _) if name.starts_with("__thaw_flat_map_callback_result_"))));
            let allocated = statements.iter().find_map(|statement| match statement {
                HirStmt::Let(_, ty, HirExpr::ArrayAlloc(_, element)) if matches!(ty, HirType::Array(_)) => Some((ty, element)),
                _ => None,
            }).unwrap();
            assert_eq!(allocated.0, &HirType::Array(Box::new(HirType::Array(Box::new(HirType::F64)))));
            assert_eq!(allocated.1, &HirType::Array(Box::new(HirType::F64)));
            direct_map = true;
        }
    });
    assert!(direct_map, "plain map stores the callback's Array<F64> directly");
}

#[test]
fn flat_map_normalizes_sparse_wrapped_json_before_runtime_flattening() {
    let program = lower(
        r#"
            function identity(values: Json[]): Json[] {
                return values.flatMap(value => value);
            }
            function mixed(values: Json[]): Json[] {
                return values.flatMap(value => value === null ? [value] : value);
            }
            function naturalNullable(values: Json[]): Json[] {
                return values.flatMap(value => value === undefined ? null : value);
            }
            function naturalNullish(values: Json[]): Json[] {
                return values.flatMap(value => value === null ? undefined : value);
            }
            function nullableTyped(values: Json[]): Json[] {
                const callback: (value: Json) => Json | null = value => value === null ? null : value;
                return values.flatMap(callback);
            }
            function nullishTyped(values: Json[]): Json[] {
                const callback: (value: Json) => Json | null | undefined = value => value === null ? null : value === undefined ? undefined : value;
                return values.flatMap(callback);
            }
            function scalarResult(values: number[]): number[] {
                return values.flatMap(value => value);
            }
            function mixedResult(values: number[]): number[] {
                return values.flatMap(value => value === 0 ? [value] : value);
            }
            function namedMap(values: number[]): number[] {
                const callback: (value: number) => number = value => value + 1;
                return values.map(callback);
            }
            function namedFlatMap(values: number[]): number[] {
                const callback: (value: number) => number = value => value + 1;
                return values.flatMap(callback);
            }
        "#,
    );

    #[derive(Clone)]
    struct CallbackBinding {
        binding_name: String,
        parameters: Vec<HirType>,
        result: HirType,
        actual_parameter: Option<String>,
        actual_parameters: Vec<HirType>,
        actual_result: Option<HirType>,
        actual_source: HirExpr,
    }
    #[derive(Clone)]
    struct NumberAdapter {
        parameter_name: String,
        parameter_type: HirType,
        result_type: HirType,
        source: HirExpr,
        body: HirExpr,
    }
    type MapElementBinding = (String, HirType, Vec<(String, HirType)>);
    #[derive(Clone)]
    struct MapRoute {
        receiver_name: String,
        receiver_type: HirType,
        receiver_source: HirExpr,
        callback_bindings: Vec<CallbackBinding>,
        callback_calls: Vec<(String, Vec<HirExpr>)>,
        typed_indices: Vec<(String, HirType)>,
        map_element_bindings: Vec<MapElementBinding>,
        optional_none_names: Vec<String>,
        optional_value_names: Vec<String>,
    }
    #[derive(Default)]
    struct Observed {
        normalizers: Vec<(HirType, HirType)>,
        typed_indices: Vec<(String, HirType)>,
        map_element_bindings: Vec<MapElementBinding>,
        map_callback_bindings: Vec<CallbackBinding>,
        map_callback_calls: Vec<(String, Vec<HirExpr>)>,
        map_routes: Vec<MapRoute>,
        number_adapters: Vec<NumberAdapter>,
        optional_none_names: Vec<String>,
        optional_value_names: Vec<String>,
        flat_inputs: Vec<(Option<f64>, Vec<HirType>, Option<String>)>,
        flat_map_callback_snapshots: Vec<(String, HirType, String, HirExpr)>,
        flat_result_bindings: Vec<(String, HirType, HirType)>,
        variables: Vec<String>,
    }
    fn visit_expr(expression: &HirExpr, observed: &mut Observed, collect_allocations: bool) {
        if collect_allocations {
            if let HirExpr::ArrayAlloc(_, element) = expression {
                observed.flat_inputs.last_mut().unwrap().1.push(element.clone());
            }
        }
        match expression {
            HirExpr::Var(name) => observed.variables.push(name.clone()),
            HirExpr::Lambda(_, parameters, result, body) => {
                if let Some(parameter) = parameters.first().filter(|parameter| parameter.name.starts_with("__thaw_flat_map_any_normalize_")) {
                    observed.normalizers.push((parameter.ty.clone(), result.clone()));
                }
                visit_expr(body, observed, collect_allocations);
            }
            HirExpr::TypedClosure(_, body) | HirExpr::NonArrowFunction(body) | HirExpr::RecursiveClosure(_, _, body)
            | HirExpr::NonArrowThis { payload: body, .. } => {
                visit_expr(body, observed, collect_allocations);
            }
            HirExpr::Call(callee, arguments) => {
                if let HirExpr::Lambda(_, parameters, result, body) = callee.as_ref() {
                    if let Some(parameter) = parameters.first().filter(|parameter| parameter.name.starts_with("__thaw_number_optional_")) {
                        if arguments.len() == 1 {
                            let source = &arguments[0];
                            observed.number_adapters.push(NumberAdapter {
                                parameter_name: parameter.name.clone(),
                                parameter_type: parameter.ty.clone(),
                                result_type: result.clone(),
                                source: source.clone(),
                                body: body.as_ref().clone(),
                            });
                        }
                    }
                }
                if let HirExpr::Var(name) = callee.as_ref() {
                    if name.starts_with("__thaw_map_callback_") {
                        observed.map_callback_calls.push((name.clone(), arguments.clone()));
                    }
                }
                if matches!(callee.as_ref(), HirExpr::Var(name) if name == "__thaw_any_array_flat") {
                    let depth = arguments.get(1).and_then(|argument| match argument {
                        HirExpr::Lit(HirLit::F64(value)) => Some(*value),
                        _ => None,
                    });
                    let direct_binding = arguments.first().and_then(|input| match input {
                        HirExpr::ArrayLit(elements) if elements.len() == 1 => match &elements[0] {
                            HirExpr::Var(name) => Some(name.clone()),
                            _ => None,
                        },
                        _ => None,
                    });
                    observed.flat_inputs.push((depth, Vec::new(), direct_binding));
                    if let Some(input) = arguments.first() { visit_expr(input, observed, true); }
                    for argument in arguments.iter().skip(1) { visit_expr(argument, observed, collect_allocations); }
                } else {
                    if let HirExpr::Lambda(_, parameters, _, body) = callee.as_ref() {
                        for parameter in parameters {
                            if !parameter.name.starts_with("__thaw_map_callback_") { continue; }
                            let HirType::Function(callback_parameters, callback_result) = &parameter.ty else { continue };
                            if let Some(argument) = arguments.first() {
                                let (actual_parameter, actual_parameters, actual_result) = match argument {
                                    HirExpr::Lambda(_, actual_parameters, actual_result, _) => (
                                        actual_parameters.first().map(|parameter| parameter.name.clone()),
                                        actual_parameters.iter().map(|parameter| parameter.ty.clone()).collect(),
                                        Some(actual_result.clone()),
                                    ),
                                    _ => (None, Vec::new(), None),
                                };
                                observed.map_callback_bindings.push(CallbackBinding {
                                    binding_name: parameter.name.clone(),
                                    parameters: callback_parameters.clone(),
                                    result: callback_result.as_ref().clone(),
                                    actual_parameter,
                                    actual_parameters,
                                    actual_result,
                                    actual_source: argument.clone(),
                                });
                            }
                        }
                        if let Some(receiver) = parameters.iter().find(|parameter| parameter.name.starts_with("__thaw_map_receiver_")) {
                            let mut body_observed = Observed::default();
                            visit_expr(body, &mut body_observed, false);
                            if let Some(source) = arguments.first() {
                                observed.map_routes.push(MapRoute {
                                    receiver_name: receiver.name.clone(),
                                    receiver_type: receiver.ty.clone(),
                                    receiver_source: source.clone(),
                                    callback_bindings: body_observed.map_callback_bindings,
                                    callback_calls: body_observed.map_callback_calls,
                                    typed_indices: body_observed.typed_indices,
                                    map_element_bindings: body_observed.map_element_bindings,
                                    optional_none_names: body_observed.optional_none_names,
                                    optional_value_names: body_observed.optional_value_names,
                                });
                            }
                        }
                    }
                    visit_expr(callee, observed, collect_allocations);
                    for argument in arguments { visit_expr(argument, observed, collect_allocations); }
                }
            }
            HirExpr::FunctionCallWithThis(callee, this_arg, arguments, _, _) => {
                visit_expr(callee, observed, collect_allocations);
                visit_expr(this_arg, observed, collect_allocations);
                for argument in arguments { visit_expr(argument, observed, collect_allocations); }
            }
            HirExpr::Block(statements) => visit_statements(statements, observed, collect_allocations),
            HirExpr::Conditional(test, yes, no, _) => {
                visit_expr(test, observed, collect_allocations);
                visit_expr(yes, observed, collect_allocations);
                visit_expr(no, observed, collect_allocations);
            }
            HirExpr::BinOp(_, test, yes)
            | HirExpr::EvalThen(test, yes)
            | HirExpr::Index(test, yes)
            | HirExpr::JsonKey(test, yes)
            | HirExpr::JsonIndex(test, yes) => {
                visit_expr(test, observed, collect_allocations);
                visit_expr(yes, observed, collect_allocations);
            }
            HirExpr::TypedIndex(array, index, ty) => {
                if let HirExpr::Var(name) = array.as_ref() {
                    observed.typed_indices.push((name.clone(), ty.clone()));
                }
                visit_expr(array, observed, collect_allocations);
                visit_expr(index, observed, collect_allocations);
            }
            HirExpr::ArrayLit(values) | HirExpr::DynamicCall(_, values) => {
                for value in values { visit_expr(value, observed, collect_allocations); }
            }
            HirExpr::ArrayAlloc(length, _) | HirExpr::ArrayLen(length)
            | HirExpr::OptionalSome(length, _)
            | HirExpr::NullableSome(length, _)
            | HirExpr::NullableIsNone(length, _) | HirExpr::NullableValue(length, _)
            | HirExpr::NullishSome(length, _) | HirExpr::NullishIsNull(length, _)
            | HirExpr::NullishIsUndefined(length, _) | HirExpr::NullishIsNone(length, _)
            | HirExpr::NullishValue(length, _) | HirExpr::UnionInject(length, _, _)
            | HirExpr::UnionTag(length, _) | HirExpr::UnionValue(length, _, _)
            | HirExpr::JsValueAsJson(length) | HirExpr::JsonAsNumber(length)
            | HirExpr::JsonAsString(length) | HirExpr::JsonAsBool(length)
            | HirExpr::ThrowValue(length, _) => visit_expr(length, observed, collect_allocations),
            HirExpr::OptionalIsNone(value, _) => {
                if let HirExpr::Var(name) = value.as_ref() { observed.optional_none_names.push(name.clone()); }
                visit_expr(value, observed, collect_allocations);
            }
            HirExpr::OptionalValue(value, _) => {
                if let HirExpr::Var(name) = value.as_ref() { observed.optional_value_names.push(name.clone()); }
                visit_expr(value, observed, collect_allocations);
            }
            HirExpr::IndexAssign(array, index, value) => {
                visit_expr(array, observed, collect_allocations);
                visit_expr(index, observed, collect_allocations);
                visit_expr(value, observed, collect_allocations);
            }
            HirExpr::Assign(_, value) | HirExpr::ArraySetLen(value, _, _) => {
                visit_expr(value, observed, collect_allocations);
            }
            HirExpr::UnionMemberIsEqual(left, right, _, _) | HirExpr::UnionIsEqual(left, right, _) => {
                visit_expr(left, observed, collect_allocations);
                visit_expr(right, observed, collect_allocations);
            }
            _ => {}
        }
    }
    fn visit_statements(statements: &[HirStmt], observed: &mut Observed, collect_allocations: bool) {
        for pair in statements.windows(2) {
            if let (
                HirStmt::Let(name, result_type, _),
                HirStmt::Expr(HirExpr::IndexAssign(destination, _, snapshot)),
            ) = (&pair[0], &pair[1]) {
                if name.starts_with("__thaw_flat_map_callback_result_") {
                    if let HirExpr::Var(destination_name) = destination.as_ref() {
                        observed.flat_map_callback_snapshots.push((name.clone(), result_type.clone(), destination_name.clone(), snapshot.as_ref().clone()));
                    }
                }
            }
        }
        for statement in statements { visit_stmt(statement, observed, collect_allocations); }
    }
    fn visit_stmt(statement: &HirStmt, observed: &mut Observed, collect_allocations: bool) {
        match statement {
            HirStmt::Expr(expression) | HirStmt::Return(Some(expression)) | HirStmt::Throw(expression) => {
                visit_expr(expression, observed, collect_allocations);
            }
            HirStmt::Let(name, ty, expression) => {
                if name.starts_with("__thaw_map_element_") {
                    let mut initializer = Observed::default();
                    visit_expr(expression, &mut initializer, false);
                    observed.map_element_bindings.push((name.clone(), ty.clone(), initializer.typed_indices));
                }
                if name.starts_with("__thaw_flat_result_") {
                    if let HirExpr::ArrayAlloc(_, element) = expression {
                        observed.flat_result_bindings.push((name.clone(), ty.clone(), element.clone()));
                    }
                }
                visit_expr(expression, observed, collect_allocations);
            }
            HirStmt::If(test, yes, no) => {
                visit_expr(test, observed, collect_allocations);
                visit_statements(yes, observed, collect_allocations);
                visit_statements(no, observed, collect_allocations);
            }
            HirStmt::While(test, body) => {
                visit_expr(test, observed, collect_allocations);
                visit_statements(body, observed, collect_allocations);
            }
            HirStmt::Try(body, _, catch, _) => {
                visit_statements(body, observed, collect_allocations);
                visit_statements(catch, observed, collect_allocations);
            }
            HirStmt::Finally(body, _) => {
                visit_statements(body, observed, collect_allocations);
            }
            HirStmt::Return(None) | HirStmt::Break | HirStmt::Continue
            | HirStmt::BreakDepth(_) | HirStmt::ContinueDepth(_) => {}
        }
    }
    fn selected_result_body(body: &HirExpr) -> Option<&HirExpr> {
        match body {
            HirExpr::Block(statements) => match statements.last()? {
                HirStmt::Return(Some(expression)) => Some(expression),
                _ => None,
            },
            HirExpr::Call(..) | HirExpr::Var(..) => Some(body),
            _ => None,
        }
    }
    fn returned_variable(expression: &HirExpr) -> Option<String> {
        match expression {
            HirExpr::Var(name) => Some(name.clone()),
            HirExpr::Call(callee, _) => {
                let HirExpr::Lambda(_, _, _, body) = callee.as_ref() else { return None };
                returned_variable(selected_result_body(body)?)
            }
            HirExpr::TypedClosure(_, body) => returned_variable(body),
            _ => None,
        }
    }
    fn selected_return_flat(expression: &HirExpr) -> Option<(String, HirType, &HirExpr, String, HirType, &HirExpr)> {
        let HirExpr::Call(callee, arguments) = expression else { return None };
        let HirExpr::Lambda(_, parameters, _, body) = callee.as_ref() else { return None };
        let Some(receiver) = parameters.first().filter(|parameter|
            parameter.name.starts_with("__thaw_flat_receiver_")) else {
                return selected_return_flat(selected_result_body(body)?);
            };
        let HirType::Array(inner) = &receiver.ty else { return None };
        let HirType::Array(element) = inner.as_ref() else { return None };
        if arguments.len() != 1 { return None; }
        let HirExpr::Block(statements) = body.as_ref() else { return None };
        let HirExpr::ArraySetLen(result, _, returned_element) = selected_result_body(body)? else {
            return None;
        };
        let HirExpr::Var(result_name) = result.as_ref() else { return None };
        let expected_result_type = HirType::Array(element.clone());
        let matching_result_bindings = statements.iter().filter(|statement| matches!(statement,
            HirStmt::Let(name, ty, HirExpr::ArrayAlloc(_, allocated_element))
                if name == result_name
                    && ty == &expected_result_type
                    && allocated_element == element.as_ref()
        )).count();
        if matching_result_bindings != 1 || returned_element != element.as_ref() {
            return None;
        }
        let source = arguments.first()?;
        Some((receiver.name.clone(), receiver.ty.clone(), source,
            result_name.clone(), element.as_ref().clone(), body.as_ref()))
    }
    fn selected_flat_inside_result_body(expression: &HirExpr) -> Option<(String, HirType, &HirExpr, String, HirType, &HirExpr)> {
        let HirExpr::Call(callee, _) = expression else { return None };
        let HirExpr::Lambda(_, _, _, body) = callee.as_ref() else { return None };
        if matches!(body.as_ref(), HirExpr::Block(statements)
            if matches!(statements.last(), Some(HirStmt::Return(Some(HirExpr::ArraySetLen(..)))))) {
            if let Some(flat) = selected_return_flat(expression) { return Some(flat); }
        }
        selected_flat_inside_result_body(selected_result_body(body)?)
    }

    let nested_result = HirExpr::Call(
        Box::new(HirExpr::Lambda(
            vec![HirParam { name: "capture_decoy".into(), ty: HirType::Json }],
            vec![HirParam { name: "callback".into(), ty: HirType::Json }],
            HirType::Json,
            Box::new(HirExpr::Block(vec![
                HirStmt::Return(Some(HirExpr::Var("prior_decoy".into()))),
                HirStmt::Return(Some(HirExpr::Var("map_result".into()))),
            ])),
        )),
        vec![HirExpr::Var("argument_decoy".into())],
    );
    let nested_bindings = HirExpr::Call(
        Box::new(HirExpr::Lambda(
            vec![HirParam { name: "outer_capture_decoy".into(), ty: HirType::Json }],
            vec![HirParam { name: "receiver".into(), ty: HirType::Json }],
            HirType::Json,
            Box::new(nested_result),
        )),
        vec![HirExpr::Var("receiver_argument_decoy".into())],
    );
    assert_eq!(returned_variable(&nested_bindings).as_deref(), Some("map_result"),
        "nested direct Call result bodies follow only each binding Lambda's returned expression");
    assert_eq!(returned_variable(&HirExpr::Call(
        Box::new(HirExpr::Lambda(Vec::new(), Vec::new(), HirType::Json,
            Box::new(HirExpr::Block(vec![HirStmt::Return(Some(HirExpr::Var("argument_decoy".into()))), HirStmt::Return(None)])))),
        vec![HirExpr::Var("map_result".into())],
    )), None,
        "call arguments cannot supply a result when the final statement does not return a value");
    assert_eq!(returned_variable(&HirExpr::Call(
        Box::new(HirExpr::Lambda(Vec::new(), Vec::new(), HirType::Json,
            Box::new(HirExpr::Block(vec![HirStmt::Return(Some(HirExpr::Lit(HirLit::Undefined)))])))),
        vec![HirExpr::Var("map_result".into())],
    )), None,
        "a non-variable final result does not fall back to an unrelated call argument");

    for name in ["identity", "mixed", "nullableTyped", "nullishTyped", "naturalNullable", "naturalNullish"] {
        let function = program.functions.iter().find(|function| function.name == name).unwrap();
        assert_eq!(function.ret, HirType::Array(Box::new(HirType::Json)));
        let mut observed = Observed::default();
        visit_statements(&function.body, &mut observed, false);
        assert_eq!(observed.flat_inputs.len(), 1, "{name} has one callback-result flatten");
        assert_eq!(observed.flat_inputs[0].0, Some(1.0), "{name} uses depth-one runtime flattening");
        assert_eq!(observed.flat_map_callback_snapshots.len(), 1,
            "{name} binds the user callback result and snapshots it in the immediately following statement");
        let (callback_result_name, callback_result_type, _, snapshot) = &observed.flat_map_callback_snapshots[0];
        let mut snapshot_observed = Observed::default();
        visit_expr(snapshot, &mut snapshot_observed, false);
        assert_eq!(snapshot_observed.normalizers, observed.normalizers,
            "{name} keeps its normalization mapper inside the callback's immediate snapshot");
        match name {
            "naturalNullable" => {
                assert!(observed.normalizers.is_empty(),
                    "the existing undefined-to-null branch materializes raw Json directly");
                assert_eq!(*callback_result_type, HirType::Json);
                assert_eq!(observed.flat_inputs[0].1, Vec::<HirType>::new(),
                    "the direct Json input is an ArrayLit, not a generated map allocation");
                assert_eq!(observed.flat_inputs[0].2.as_deref(), Some(callback_result_name.as_str()),
                    "the depth-one flattener receives the singleton array containing the exact once-bound result");
            }
            "identity" | "naturalNullish" | "mixed" | "nullableTyped" | "nullishTyped" => {
                assert_eq!(observed.normalizers.len(), 1, "{name} has one normalization mapper");
                let (input_type, return_type) = &observed.normalizers[0];
                assert_eq!(callback_result_type, input_type,
                    "{name} normalizes the exact once-bound user callback result type");
                assert_eq!(*return_type, HirType::Json, "{name} normalizer returns Json");
                assert_eq!(observed.flat_inputs[0].1, vec![HirType::Json],
                    "{name} passes the generated Array<Json> map allocation to runtime flattening");
                assert_eq!(observed.flat_inputs[0].2, None,
                    "{name} reaches runtime flattening through the generated normalizer map");
                let routes = snapshot_observed.map_routes.iter().filter(|route|
                    route.callback_bindings.iter().any(|binding|
                        binding.binding_name.starts_with("__thaw_map_callback_")
                            && binding.actual_parameter.as_deref().is_some_and(|parameter| parameter.starts_with("__thaw_flat_map_any_normalize_"))
                            && binding.parameters == vec![input_type.clone()]
                            && binding.result == HirType::Json
                            && binding.actual_parameters == vec![input_type.clone()]
                            && binding.actual_result.as_ref() == Some(&HirType::Json)
                    )
                ).collect::<Vec<_>>();
                assert_eq!(routes.len(), 1, "{name} binds its normalizer Lambda as exactly one generated map callback");
                let route = routes[0];
                assert_eq!(route.receiver_type, HirType::Array(Box::new(input_type.clone())),
                    "{name} supplies the exact Array<E> ABI to that normalization map");
                assert!(matches!(&route.receiver_source, HirExpr::ArrayLit(elements)
                    if elements.as_slice() == [HirExpr::Var(callback_result_name.clone())]),
                    "{name} maps the once-bound callback result as a singleton source");
                assert_eq!(route.typed_indices.iter().filter(|(receiver, ty)| receiver == &route.receiver_name && ty == input_type).count(), 1,
                    "{name} normalization map reads one source element using its exact {input_type:?} ABI");
                let scratches = route.map_element_bindings.iter().filter(|(_, ty, reads)|
                    ty == input_type && reads.iter().any(|(receiver, read_type)| receiver == &route.receiver_name && read_type == input_type)
                ).collect::<Vec<_>>();
                assert_eq!(scratches.len(), 1,
                    "{name} normalization map scratch uses its exact {input_type:?} ABI");
                assert_eq!(observed.flat_map_callback_snapshots[0].1, *input_type,
                    "{name} snapshots the raw value with the same E type passed to the selected map");
                match name {
                    "identity" => assert_eq!(*input_type, HirType::Optional(Box::new(HirType::Json))),
                    "naturalNullish" => assert_eq!(*input_type, HirType::Optional(Box::new(HirType::Optional(Box::new(HirType::Json))))),
                    "mixed" => assert!(matches!(input_type, HirType::Union(members)
                if members.contains(&HirType::Optional(Box::new(HirType::Json)))
                            && members.contains(&HirType::Array(Box::new(HirType::Optional(Box::new(HirType::Json))))))),
                    "nullableTyped" => assert_eq!(*input_type, HirType::Nullable(Box::new(HirType::Json))),
                    "nullishTyped" => assert_eq!(*input_type, HirType::Nullish(Box::new(HirType::Json))),
                    _ => unreachable!(),
                }
            }
            _ => unreachable!(),
        }
    }
    for (name, expected_raw_type) in [
        ("scalarResult", HirType::Optional(Box::new(HirType::F64))),
        ("mixedResult", HirType::Union(vec![
            HirType::Array(Box::new(HirType::Optional(Box::new(HirType::F64)))),
            HirType::Optional(Box::new(HirType::F64)),
        ])),
    ] {
        let function = program.functions.iter().find(|function| function.name == name).unwrap();
        assert_eq!(function.ret, HirType::Array(Box::new(HirType::F64)));
        let mut observed = Observed::default();
        visit_statements(&function.body, &mut observed, false);
        assert_eq!(observed.flat_map_callback_snapshots.len(), 1);
        assert_eq!(observed.flat_map_callback_snapshots[0].1, expected_raw_type,
            "{name} keeps the original raw callback result ABI");
        let final_return = function.body.iter().rev().find_map(|statement| match statement {
            HirStmt::Return(Some(expression)) => Some(expression),
            _ => None,
        }).expect("numeric flatMap function has a final return expression");
        let mut returned = Observed::default();
        visit_expr(final_return, &mut returned, false);
        let routes = returned.map_routes.iter().filter(|route| route.callback_bindings.iter().any(|binding|
            binding.binding_name.starts_with("__thaw_map_callback_")
                && binding.actual_parameter.as_deref().is_some_and(|parameter| parameter.starts_with("__thaw_array_unwrap_"))
                && binding.parameters == vec![HirType::Optional(Box::new(HirType::F64))]
                && binding.result == HirType::F64
                && binding.actual_parameters == vec![HirType::Optional(Box::new(HirType::F64))]
                && binding.actual_result.as_ref() == Some(&HirType::F64)
        )).collect::<Vec<_>>();
        assert_eq!(routes.len(), 1, "{name} uses one actual Optional<F64>-to-F64 return mapper binding");
        let route = routes[0];
        assert_eq!(route.receiver_type, HirType::Array(Box::new(HirType::Optional(Box::new(HirType::F64)))));
        assert_eq!(route.typed_indices, vec![(route.receiver_name.clone(), HirType::Optional(Box::new(HirType::F64)))]);
        assert_eq!(route.map_element_bindings.len(), 1);
        assert_eq!(route.map_element_bindings[0].1, HirType::Optional(Box::new(HirType::F64)));
        assert!(route.map_element_bindings[0].2.contains(&(route.receiver_name.clone(), HirType::Optional(Box::new(HirType::F64)))),
            "{name} return mapper scratch initializes from that map receiver's exact Optional<F64> read");
        let callback_binding = route.callback_bindings.iter().find(|binding|
            binding.binding_name.starts_with("__thaw_map_callback_")
                && binding.actual_parameter.as_deref().is_some_and(|parameter| parameter.starts_with("__thaw_array_unwrap_"))
        ).unwrap();
        let actual_parameter = callback_binding.actual_parameter.as_ref().unwrap();
        let calls = route.callback_calls.iter().filter(|(binding, _)| binding == &callback_binding.binding_name)
            .collect::<Vec<_>>();
        assert_eq!(calls.len(), 1, "{name} invokes the selected return mapper once");
        let arguments = &calls[0].1;
        assert_eq!(arguments.len(), 1, "{name} passes one adapted numeric element");
        assert!(matches!(arguments.as_slice(), [HirExpr::Var(element)] if element == &route.map_element_bindings[0].0),
            "{name} passes the selected map scratch to its callback: {arguments:?}");
        let mut actual_callback = Observed::default();
        visit_expr(&callback_binding.actual_source, &mut actual_callback, false);
        assert_eq!(actual_callback.number_adapters.len(), 1,
            "{name} has one Optional<F64> adapter in the actual return-mapper callback");
        let adapter = &actual_callback.number_adapters[0];
        assert!(adapter.parameter_name.starts_with("__thaw_number_optional_"));
        assert_eq!(adapter.parameter_type, HirType::Optional(Box::new(HirType::F64)));
        assert_eq!(adapter.result_type, HirType::F64);
        assert_eq!(adapter.source, HirExpr::Var(actual_parameter.clone()),
            "{name} passes the exact return-mapper parameter into the number adapter");
        assert!(actual_callback.optional_none_names.contains(&adapter.parameter_name),
            "{name} absence check reads the adapter's exact once-bound Optional<F64> alias");
        assert!(actual_callback.optional_value_names.contains(&adapter.parameter_name),
            "{name} reads the present payload through the same adapted alias");
        let HirExpr::Block(adapter_statements) = &adapter.body else {
            panic!("{name} numeric conversion binds absence and payload handling in a block: {:?}", adapter.body);
        };
        assert!(matches!(adapter_statements.first(), Some(HirStmt::If(test, _, _))
            if matches!(test, HirExpr::OptionalIsNone(value, HirType::F64)
                if matches!(value.as_ref(), HirExpr::Var(name) if name == &adapter.parameter_name))),
            "{name} tests absence on the adapted Optional<F64> alias: {:?}", adapter.body);
        let (flat_receiver_name, flat_receiver_type, flat_input, flat_result_name,
            flat_element_type, flat_body) = selected_flat_inside_result_body(&route.receiver_source)
            .expect("numeric return mapper reaches its returned depth-one flatten loop through result bodies");
        assert_eq!(flat_element_type, HirType::Optional(Box::new(HirType::F64)));
        assert_eq!(flat_receiver_type,
            HirType::Array(Box::new(HirType::Array(Box::new(flat_element_type.clone())))));
        let matching_flat_results = match flat_body {
            HirExpr::Block(statements) => statements.iter().filter(|statement| matches!(statement,
                HirStmt::Let(name, ty, HirExpr::ArrayAlloc(_, allocated_element))
                    if name == &flat_result_name
                        && ty == &HirType::Array(Box::new(flat_element_type.clone()))
                        && allocated_element == &flat_element_type
            )).count(),
            _ => 0,
        };
        assert_eq!(matching_flat_results, 1,
            "{name} ties the final ArraySetLen result to one same-block allocation of its exact element type");
        let mut selected_flat = Observed::default();
        visit_expr(flat_body, &mut selected_flat, false);
        assert!(selected_flat.typed_indices.contains(&(
            flat_receiver_name.clone(), HirType::Array(Box::new(flat_element_type.clone())))),
            "{name} outer flat loop reads from the selected receiver's exact nested-array type");
        let flattened_source = returned_variable(flat_input)
            .expect("outer flat receiver source is a result binding returned by the original map");
        assert_eq!(flattened_source, observed.flat_map_callback_snapshots[0].2,
            "{name} selected outer flat source is the destination populated by its callback snapshot");
        let mut receiver = Observed::default();
        visit_expr(&route.receiver_source, &mut receiver, false);
        let selected_flat_bindings = receiver.flat_result_bindings.iter().filter(|(name, ty, element)|
            name == &flat_result_name
                && ty == &HirType::Array(Box::new(flat_element_type.clone()))
                && element == &flat_element_type
        ).count();
        assert_eq!(selected_flat_bindings, 1,
            "{name} selects the actual returned flat allocation without requiring a unique allocation in the whole receiver graph");
        assert!(receiver.variables.contains(&flat_result_name),
            "{name} final flat return consumes its selected result binding");
        assert_eq!(receiver.flat_map_callback_snapshots.len(), 1,
            "{name} flattened receiver contains the original once-bound callback snapshot");
        assert_eq!(receiver.flat_map_callback_snapshots[0].0, observed.flat_map_callback_snapshots[0].0,
            "{name} return adapter source retains the original callback-result binding");
        assert_eq!(receiver.flat_map_callback_snapshots[0].1, expected_raw_type);
        assert_eq!(receiver.flat_map_callback_snapshots[0].2, observed.flat_map_callback_snapshots[0].2,
            "{name} flattened receiver consumes the same map result that receives callback snapshots");
        assert!(receiver.variables.contains(&receiver.flat_map_callback_snapshots[0].2),
            "{name} flat-one source reads the result array populated by the original callback snapshot");
    }

    for name in ["namedMap", "namedFlatMap"] {
        let function = program.functions.iter().find(|function| function.name == name).unwrap();
        let mut observed = Observed::default();
        visit_statements(&function.body, &mut observed, false);
        let routes = observed.map_routes.iter().filter(|route|
            route.receiver_type == HirType::Array(Box::new(HirType::F64))
                && route.callback_bindings.iter().any(|binding|
                    binding.parameters == vec![HirType::F64]
                        && binding.result == HirType::F64
                        && matches!(&binding.actual_source, HirExpr::Var(source) if source == "callback")
                )
        ).collect::<Vec<_>>();
        assert_eq!(routes.len(), 1, "{name} keeps the original fixed local callback signature");
        let route = routes[0];
        assert!(matches!(&route.receiver_source, HirExpr::Var(source) if source == "values"));
        let selected_receiver_reads = route.typed_indices.iter()
            .filter(|(receiver, _)| receiver == &route.receiver_name)
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(selected_receiver_reads,
            vec![(route.receiver_name.clone(), HirType::F64)],
            "{name} has exactly one F64 typed read from the selected map receiver");
        let scratches = route.map_element_bindings.iter().filter(|(_, ty, reads)|
            *ty == HirType::Optional(Box::new(HirType::F64))
                && reads.contains(&(route.receiver_name.clone(), HirType::F64))
        ).collect::<Vec<_>>();
        assert_eq!(scratches.len(), 1, "{name} reads one sparse F64 element into its optional scratch");
        let callback = route.callback_bindings.iter().find(|binding| matches!(&binding.actual_source, HirExpr::Var(_))).unwrap();
        let calls = route.callback_calls.iter().filter(|(binding, _)| binding == &callback.binding_name).collect::<Vec<_>>();
        assert_eq!(calls.len(), 1, "{name} invokes its local callback once per present element");
        let arguments = &calls[0].1;
        assert_eq!(arguments.len(), 1);
        let HirExpr::Call(adapter, adapter_arguments) = &arguments[0] else {
            panic!("{name} uses the existing numeric argument adapter: {:?}", arguments[0]);
        };
        let HirExpr::Lambda(_, parameters, result, body) = adapter.as_ref() else {
            panic!("{name} adapter has a typed optional-number parameter: {adapter:?}");
        };
        assert_eq!(parameters.len(), 1);
        assert!(parameters[0].name.starts_with("__thaw_number_optional_"));
        assert_eq!(parameters[0].ty, HirType::Optional(Box::new(HirType::F64)));
        assert_eq!(*result, HirType::F64);
        assert!(matches!(adapter_arguments.as_slice(), [HirExpr::Var(name)] if name == &scratches[0].0));
        assert!(route.optional_none_names.contains(&parameters[0].name));
        let HirExpr::Block(adapter_statements) = body.as_ref() else {
            panic!("{name} number adapter binds absence and present handling in a block: {body:?}");
        };
        assert!(matches!(adapter_statements.first(), Some(HirStmt::If(test, _, _))
            if matches!(test, HirExpr::OptionalIsNone(value, HirType::F64)
                if matches!(value.as_ref(), HirExpr::Var(name) if name == &parameters[0].name))));
        assert!(route.optional_value_names.contains(&parameters[0].name));
    }

    let invalid = thaw_parser::parse_typescript(
        "function invalid(values: number[]): number[] { const callback: (value: string) => number = value => value.length; return values.map(callback); }"
    ).unwrap();
    let error = lower_module(&invalid).unwrap_err();
    assert!(error.contains("callback") || error.contains("array mapper") || error.contains("parameter"),
        "a fixed callback with an incompatible parameter remains rejected: {error}");
}

#[test]
fn string_normalize_defaults_undefined_forms_lazily() {
    let program = lower(
        r#"
            function omitted(text: string): string { return text.normalize(); }
            function explicit(text: string): string { return text.normalize(undefined); }
            function optional(text: string, form?: string): string { return text.normalize(form); }
            function unionForm(text: string, form: string | undefined): string { return text.normalize(form); }
            function dynamic(text: string, form: any): string { return text.normalize(form); }
            function supplied(text: string, form: string): string { return text.normalize(form); }
            function nullForm(text: string): string { return text.normalize(null); }
            function literalUndefined(text: string): string { return text.normalize('undefined'); }
            function receiver(): string { return 'é'; }
            function missingForm(): undefined { return undefined; }
            function effectOrder(): string { return receiver().normalize(missingForm()); }
        "#,
    );

    fn function<'a>(program: &'a HirProgram, name: &str) -> &'a HirFunction {
        program.functions.iter().find(|function| function.name == name).unwrap()
    }
    fn form_source(expression: &HirExpr) -> Option<&HirExpr> {
        match expression {
            HirExpr::Call(callee, arguments) => {
                if let HirExpr::Lambda(_, parameters, _, body) = callee.as_ref() {
                    if parameters.iter().any(|parameter| parameter.name.starts_with("__thaw_normalize_form_")) {
                        return arguments.first();
                    }
                    if let Some(source) = form_source(body) {
                        return Some(source);
                    }
                }
                arguments.iter().find_map(form_source)
            }
            HirExpr::Lambda(_, _, _, body) => form_source(body),
            HirExpr::Block(statements) => statements.iter().find_map(|statement| match statement {
                HirStmt::Expr(value) | HirStmt::Return(Some(value)) | HirStmt::Throw(value) => form_source(value),
                HirStmt::Let(_, _, value) => form_source(value),
                HirStmt::If(condition, yes, no) => form_source(condition)
                    .or_else(|| yes.iter().find_map(|statement| form_source_statement(statement)))
                    .or_else(|| no.iter().find_map(|statement| form_source_statement(statement))),
                _ => None,
            }),
            HirExpr::Conditional(_, yes, no, _) => form_source(yes).or_else(|| form_source(no)),
            _ => None,
        }
    }
    fn form_source_statement(statement: &HirStmt) -> Option<&HirExpr> {
        match statement {
            HirStmt::Expr(value) | HirStmt::Return(Some(value)) | HirStmt::Throw(value) => form_source(value),
            HirStmt::Let(_, _, value) => form_source(value),
            HirStmt::If(condition, yes, no) => form_source(condition)
                .or_else(|| yes.iter().find_map(form_source_statement))
                .or_else(|| no.iter().find_map(form_source_statement)),
            _ => None,
        }
    }
    fn form<'a>(program: &'a HirProgram, name: &str) -> &'a HirExpr {
        let body = &function(program, name).body;
        body.iter().find_map(form_source_statement).expect("normalize form binding")
    }
    fn nfc_literal(expression: &HirExpr) -> bool {
        matches!(expression, HirExpr::Lit(HirLit::Str(value)) if value == "NFC")
    }
    fn contains_optional_none(expression: &HirExpr) -> bool {
        match expression {
            HirExpr::OptionalIsNone(_, _) => true,
            HirExpr::Call(callee, arguments) => contains_optional_none(callee)
                || arguments.iter().any(contains_optional_none),
            HirExpr::Lambda(_, _, _, body) | HirExpr::TypedClosure(_, body) => contains_optional_none(body),
            HirExpr::Conditional(test, yes, no, _) => contains_optional_none(test)
                || contains_optional_none(yes) || contains_optional_none(no),
            _ => false,
        }
    }
    fn contains_call(expression: &HirExpr, target: &str) -> bool {
        match expression {
            HirExpr::Call(callee, arguments) => matches!(callee.as_ref(), HirExpr::Var(name) if name == target)
                || contains_call(callee, target) || arguments.iter().any(|argument| contains_call(argument, target)),
            HirExpr::Lambda(_, _, _, body) | HirExpr::TypedClosure(_, body) => contains_call(body, target),
            HirExpr::Conditional(test, yes, no, _) => contains_call(test, target)
                || contains_call(yes, target) || contains_call(no, target),
            HirExpr::Block(statements) => statements.iter().any(|statement| match statement {
                HirStmt::Expr(value) | HirStmt::Return(Some(value)) | HirStmt::Throw(value) => contains_call(value, target),
                HirStmt::Let(_, _, value) => contains_call(value, target),
                HirStmt::If(condition, yes, no) => contains_call(condition, target)
                    || yes.iter().any(|statement| contains_call_statement(statement, target))
                    || no.iter().any(|statement| contains_call_statement(statement, target)),
                _ => false,
            }),
            _ => false,
        }
    }
    fn contains_call_statement(statement: &HirStmt, target: &str) -> bool {
        match statement {
            HirStmt::Expr(value) | HirStmt::Return(Some(value)) | HirStmt::Throw(value) => contains_call(value, target),
            HirStmt::Let(_, _, value) => contains_call(value, target),
            HirStmt::If(condition, yes, no) => contains_call(condition, target)
                || yes.iter().any(|statement| contains_call_statement(statement, target))
                || no.iter().any(|statement| contains_call_statement(statement, target)),
            _ => false,
        }
    }
    fn contains_throw_message(expression: &HirExpr, expected: &str) -> bool {
        match expression {
            HirExpr::Lambda(_, _, _, body) | HirExpr::TypedClosure(_, body) => contains_throw_message(body, expected),
            HirExpr::Block(statements) => statements.iter().any(|statement| match statement {
                HirStmt::Throw(HirExpr::Lit(HirLit::Str(message))) => message == expected,
                HirStmt::Expr(value) | HirStmt::Return(Some(value)) | HirStmt::Throw(value)
                | HirStmt::Let(_, _, value) => contains_throw_message(value, expected),
                HirStmt::If(condition, yes, no) => contains_throw_message(condition, expected)
                    || yes.iter().any(|statement| contains_throw_statement(statement, expected))
                    || no.iter().any(|statement| contains_throw_statement(statement, expected)),
                _ => false,
            }),
            HirExpr::Call(callee, arguments) => contains_throw_message(callee, expected)
                || arguments.iter().any(|argument| contains_throw_message(argument, expected)),
            HirExpr::Conditional(test, yes, no, _) => contains_throw_message(test, expected)
                || contains_throw_message(yes, expected) || contains_throw_message(no, expected),
            _ => false,
        }
    }
    fn contains_throw_statement(statement: &HirStmt, expected: &str) -> bool {
        match statement {
            HirStmt::Throw(HirExpr::Lit(HirLit::Str(message))) => message == expected,
            HirStmt::Expr(value) | HirStmt::Return(Some(value)) | HirStmt::Throw(value)
            | HirStmt::Let(_, _, value) => contains_throw_message(value, expected),
            HirStmt::If(condition, yes, no) => contains_throw_message(condition, expected)
                || yes.iter().any(|statement| contains_throw_statement(statement, expected))
                || no.iter().any(|statement| contains_throw_statement(statement, expected)),
            _ => false,
        }
    }
    fn binding_sources<'a>(expression: &'a HirExpr, sources: &mut Vec<(String, &'a HirExpr)>) {
        if let HirExpr::Call(callee, arguments) = expression {
            if let HirExpr::Lambda(_, parameters, _, body) = callee.as_ref() {
                if let (Some(parameter), Some(source)) = (parameters.first(), arguments.first()) {
                    if parameter.name.starts_with("__thaw_normalize_receiver_")
                        || parameter.name.starts_with("__thaw_native_arg_")
                    {
                        sources.push((parameter.name.clone(), source));
                    }
                }
                binding_sources(body, sources);
            }
        }
    }
    fn call_targets(expression: &HirExpr, target: &str) -> bool {
        matches!(expression, HirExpr::Call(callee, _)
            if matches!(callee.as_ref(), HirExpr::Var(name) | HirExpr::FunctionRef(name, _, _) if name == target))
    }

    assert!(nfc_literal(form(&program, "omitted")));
    assert!(matches!(form(&program, "explicit"),
        HirExpr::Conditional(test, yes, _, HirType::Str)
            if matches!(test.as_ref(), HirExpr::Lit(HirLit::Bool(true))) && nfc_literal(yes)));
    for name in ["optional", "unionForm"] {
        assert!(matches!(form(&program, name),
            HirExpr::Conditional(test, yes, no, HirType::Str)
                if contains_optional_none(test) && nfc_literal(yes) && !nfc_literal(no)),
            "{name} must select NFC for an absent Optional and retain supplied-form conversion");
    }
    assert!(matches!(form(&program, "dynamic"),
        HirExpr::Conditional(test, yes, no, HirType::Str)
            if matches!(test.as_ref(), HirExpr::Call(callee, _)
                if matches!(callee.as_ref(), HirExpr::Var(name) if name == "__thaw_json_is_undefined" || name == "__thaw_is_undefined_dynamic_value"))
                && nfc_literal(yes) && !nfc_literal(no)));
    for name in ["supplied", "nullForm", "literalUndefined"] {
        assert!(matches!(form(&program, name),
            HirExpr::Conditional(test, yes, no, HirType::Str)
                if matches!(test.as_ref(), HirExpr::Lit(HirLit::Bool(false)))
                    && nfc_literal(yes) && !nfc_literal(no)),
            "{name} must keep the supplied invalid or explicit form path");
    }

    let effect = function(&program, "effectOrder");
    let mut sources = Vec::new();
    effect.body.iter().for_each(|statement| match statement {
        HirStmt::Return(Some(value)) | HirStmt::Expr(value) => binding_sources(value, &mut sources),
        _ => {}
    });
    let receiver = sources.iter().position(|(_, source)| call_targets(source, "receiver"));
    let form_call = sources.iter().position(|(_, source)| call_targets(source, "missingForm"));
    let receiver_count = sources.iter().filter(|(_, source)| call_targets(source, "receiver")).count();
    let form_count = sources.iter().filter(|(_, source)| call_targets(source, "missingForm")).count();
    assert!(receiver_count == 1 && form_count == 1
        && receiver.is_some() && form_call.is_some_and(|form| form > receiver.unwrap()),
        "receiver and undefined form producers must each be staged once, receiver first: {sources:?}");
    let normalize_body = &function(&program, "optional").body;
    assert!(normalize_body.iter().any(|statement| match statement {
        HirStmt::Return(Some(value)) | HirStmt::Expr(value) => contains_call(value, "__thaw_string_normalize")
            && contains_call(value, "__thaw_string_is_null")
            && contains_throw_message(value, "The normalization form should be one of NFC, NFD, NFKC, NFKD."),
        _ => false,
    }), "the runtime normalize call and invalid-form error path must remain: {normalize_body:?}");
}

#[test]
fn string_split_defaults_undefined_separator_and_limit_lazily() {
    let program = lower(
        r#"
            function explicit(text: string): string[] { return text.split(undefined, undefined); }
            function optional(text: string, separator?: string, limit?: number): string[] { return text.split(separator, limit); }
            function supplied(text: string, separator: string, limit: number): string[] { return text.split(separator, limit); }
            function spread(text: string, args: [undefined, number]): string[] { return text.split(...args); }
            function nullSeparator(text: string): string[] { return text.split(null, 1); }
            function literalSeparator(text: string): string[] { return text.split('undefined', 1); }
            function regex(text: string, limit?: number): (string | undefined)[] { return text.split(/(,)/, limit); }
            function receiver(): string { return 'aundefinedb'; }
            function separator(): undefined { return undefined; }
            function limit(): number { return 0; }
            function effects(): string[] { return receiver().split(separator(), limit()); }
            declare function opaqueReceiver(): string;
            declare function opaqueSeparator(): JsValue;
            declare function opaqueLimit(): JsValue;
            function opaqueEffects(): string[] { return opaqueReceiver().split(opaqueSeparator(), opaqueLimit()); }
        "#,
    );

    fn function<'a>(program: &'a HirProgram, name: &str) -> &'a HirFunction {
        program.functions.iter().find(|function| function.name == name).unwrap()
    }
    fn visit_expr<'a>(expr: &'a HirExpr, visit: &mut impl FnMut(&'a HirExpr)) {
        visit(expr);
        match expr {
            HirExpr::Call(callee, arguments) => {
                visit_expr(callee, visit);
                for argument in arguments { visit_expr(argument, visit); }
            }
            HirExpr::Lambda(_, _, _, body) | HirExpr::TypedClosure(_, body) => visit_expr(body, visit),
            HirExpr::Conditional(test, yes, no, _) => {
                visit_expr(test, visit);
                visit_expr(yes, visit);
                visit_expr(no, visit);
            }
            HirExpr::Block(statements) => for statement in statements { visit_stmt(statement, visit); },
            HirExpr::BinOp(_, left, right) => { visit_expr(left, visit); visit_expr(right, visit); }
            HirExpr::ArrayLit(values) | HirExpr::DynamicCall(_, values) | HirExpr::FfiCall(_, values) => {
                for value in values { visit_expr(value, visit); }
            }
            HirExpr::JsValueAsJson(value) | HirExpr::JsonAsNumber(value)
            | HirExpr::JsonAsString(value) | HirExpr::JsonAsBool(value)
            | HirExpr::TypedIndex(value, _, _) | HirExpr::JsonIndex(value, _)
            | HirExpr::OptionalValue(value, _) | HirExpr::Await(value) => visit_expr(value, visit),
            HirExpr::EvalThen(first, second) | HirExpr::JsonKey(first, second) => {
                visit_expr(first, visit);
                visit_expr(second, visit);
            }
            _ => {}
        }
    }
    fn visit_stmt<'a>(stmt: &'a HirStmt, visit: &mut impl FnMut(&'a HirExpr)) {
        match stmt {
            HirStmt::Expr(expr) | HirStmt::Return(Some(expr)) | HirStmt::Throw(expr)
            | HirStmt::Let(_, _, expr) => visit_expr(expr, visit),
            HirStmt::If(condition, yes, no) => {
                visit_expr(condition, visit);
                for statement in yes.iter().chain(no) { visit_stmt(statement, visit); }
            }
            _ => {}
        }
    }
    fn has_call(expr: &HirExpr, target: &str) -> bool {
        let mut found = false;
        visit_expr(expr, &mut |node| {
            if matches!(node, HirExpr::Call(callee, _) if matches!(callee.as_ref(), HirExpr::Var(name) if name == target))
                || matches!(node, HirExpr::FfiCall(signature, _) if signature.symbol == target)
            {
                found = true;
            }
        });
        found
    }
    fn has_var(expr: &HirExpr, target: &str) -> bool {
        let mut found = false;
        visit_expr(expr, &mut |node| {
            if matches!(node, HirExpr::Var(name) if name == target) { found = true; }
        });
        found
    }
    fn has_literal(expr: &HirExpr, value: &str) -> bool {
        let mut found = false;
        visit_expr(expr, &mut |node| {
            if matches!(node, HirExpr::Lit(HirLit::Str(text)) if text == value) { found = true; }
        });
        found
    }
    fn conditionals(function: &HirFunction) -> Vec<&HirExpr> {
        let mut found = Vec::new();
        for statement in &function.body {
            visit_stmt(statement, &mut |node| {
                if matches!(node, HirExpr::Conditional(..)) { found.push(node); }
            });
        }
        found
    }

    fn optional_none_bound<'a>(expr: &'a HirExpr, payload: &HirType) -> Option<&'a HirExpr> {
        match expr {
            HirExpr::OptionalIsNone(bound, ty) if ty == payload => Some(bound),
            HirExpr::Conditional(test, yes, no, HirType::Bool)
                if matches!(yes.as_ref(), HirExpr::Lit(HirLit::Bool(true)))
                    && matches!(no.as_ref(), HirExpr::Lit(HirLit::Bool(false))) => optional_none_bound(test, payload),
            _ => None,
        }
    }
    fn optional_missing_guard_matches(test: &HirExpr, value_branch: &HirExpr) -> bool {
        let Some(bound) = optional_none_bound(test, &HirType::F64) else { return false; };
        let mut found = false;
        visit_expr(value_branch, &mut |node| {
            if matches!(node, HirExpr::OptionalValue(value, HirType::F64) if value.as_ref() == bound) {
                found = true;
            }
        });
        found
    }
    fn optional_separator_guard_matches(test: &HirExpr, fallback: &HirExpr, native: &HirExpr) -> bool {
        optional_none_bound(test, &HirType::Str).is_some_and(|bound| {
            matches!(bound, HirExpr::Var(name)
                if has_var(native, name) && !has_call(fallback, "__thaw_string_split"))
        })
    }

    let explicit = conditionals(function(&program, "explicit"));
    assert!(explicit.iter().any(|node| matches!(node,
        HirExpr::Conditional(test, yes, no, HirType::Array(item))
            if **item == HirType::Str
                && has_call(test, "__thaw_math_trunc")
                && matches!(yes.as_ref(), HirExpr::ArrayLit(values) if values.is_empty())
                && matches!(no.as_ref(), HirExpr::ArrayLit(values) if values.len() == 1)
    )), "undefined separator must choose [] at effective zero limit and [receiver] otherwise: {explicit:?}");
    assert!(explicit.iter().any(|node| matches!(node,
        HirExpr::Conditional(test, yes, no, HirType::Array(item))
            if **item == HirType::Str
                && has_call(yes, "__thaw_math_trunc")
                && has_call(no, "__thaw_string_split")
                && matches!(test.as_ref(), HirExpr::Lit(HirLit::Bool(true)))
    )), "literal undefined separator should select its fallback without coercing it to string: {explicit:?}");
    let optional = conditionals(function(&program, "optional"));
    assert!(optional.iter().any(|node| matches!(node,
        HirExpr::Conditional(test, yes, no, HirType::F64)
            if matches!(yes.as_ref(), HirExpr::Lit(HirLit::F64(-1.0)))
                && optional_missing_guard_matches(test, no)
                && !matches!(no.as_ref(), HirExpr::Lit(HirLit::F64(-1.0)))
    )), "optional undefined limit must lazily choose the unlimited sentinel: {optional:?}");
    assert!(conditionals(function(&program, "regex")).iter().any(|node| matches!(node,
        HirExpr::Conditional(test, yes, no, HirType::F64)
            if matches!(yes.as_ref(), HirExpr::Lit(HirLit::F64(-1.0)))
                && optional_missing_guard_matches(test, no)
    )), "regex split must use the same undefined limit default");
    assert!(function(&program, "regex").body.iter().any(|statement| match statement {
        HirStmt::Return(Some(expr)) | HirStmt::Expr(expr) =>
            has_call(expr, "__thaw_regex_split") && has_call(expr, "__thaw_array_is_null"),
        _ => false,
    }), "regex split must retain its native result and invalid-pattern error check");
    assert!(conditionals(function(&program, "optional")).iter().any(|node| matches!(node,
        HirExpr::Conditional(test, yes, no, HirType::Array(item))
            if **item == HirType::Str
                && optional_separator_guard_matches(test, yes, no)
                && has_call(yes, "__thaw_math_trunc")
                && has_call(no, "__thaw_string_split")
    )), "optional undefined separator must select the whole-string fallback lazily");
    for name in ["nullSeparator", "literalSeparator", "supplied"] {
        assert!(conditionals(function(&program, name)).iter().any(|node| matches!(node,
            HirExpr::Conditional(test, _, no, HirType::Array(item))
                if **item == HirType::Str
                    && matches!(test.as_ref(), HirExpr::Lit(HirLit::Bool(false)))
                    && has_call(no, "__thaw_string_split")
        )), "{name} must retain ordinary separator conversion");
    }
    assert!(conditionals(function(&program, "spread")).iter().any(|node| matches!(node,
        HirExpr::Conditional(_, yes, no, HirType::Array(item))
            if **item == HirType::Str
                && has_call(yes, "__thaw_math_trunc")
                && has_call(no, "__thaw_string_split")
    )), "a finite tuple spread must still use the ordinary split branches");

    let effects = function(&program, "effects");
    let mut producers = Vec::new();
    for statement in &effects.body {
        visit_stmt(statement, &mut |node| {
            if let HirExpr::Call(callee, arguments) = node {
                if let HirExpr::Lambda(_, parameters, _, _) = callee.as_ref() {
                    if let (Some(parameter), Some(source)) = (parameters.first(), arguments.first()) {
                        if parameter.name.starts_with("__thaw_split_receiver_")
                            || parameter.name.starts_with("__thaw_native_arg_")
                        {
                            producers.push((parameter.name.as_str(), source));
                        }
                    }
                }
            }
        });
    }
    let receiver = producers.iter().position(|(_, source)| has_call(source, "receiver"));
    let separator = producers.iter().position(|(_, source)| has_call(source, "separator"));
    let limit = producers.iter().position(|(_, source)| has_call(source, "limit"));
    assert!(receiver.is_some() && separator.is_some() && limit.is_some()
        && receiver.unwrap() < separator.unwrap() && separator.unwrap() < limit.unwrap()
        && producers.iter().filter(|(_, source)| has_call(source, "receiver")).count() == 1
        && producers.iter().filter(|(_, source)| has_call(source, "separator")).count() == 1
        && producers.iter().filter(|(_, source)| has_call(source, "limit")).count() == 1,
        "receiver and source arguments must each be staged once in order: {producers:?}");

    let opaque_effects = function(&program, "opaqueEffects");
    let mut opaque_producers = Vec::new();
    for statement in &opaque_effects.body {
        visit_stmt(statement, &mut |node| {
            if let HirExpr::Call(callee, arguments) = node {
                if let HirExpr::Lambda(_, parameters, _, _) = callee.as_ref() {
                    if let (Some(parameter), Some(source)) = (parameters.first(), arguments.first()) {
                        if parameter.name.starts_with("__thaw_split_receiver_")
                            || parameter.name.starts_with("__thaw_native_arg_")
                        {
                            opaque_producers.push((parameter.name.as_str(), source));
                        }
                    }
                }
            }
        });
    }
    let opaque_receiver = opaque_producers.iter().position(|(_, source)| has_call(source, "opaqueReceiver"));
    let opaque_separator = opaque_producers.iter().position(|(_, source)| has_call(source, "opaqueSeparator"));
    let opaque_limit = opaque_producers.iter().position(|(_, source)| has_call(source, "opaqueLimit"));
    assert!(opaque_receiver.is_some() && opaque_separator.is_some() && opaque_limit.is_some()
        && opaque_receiver.unwrap() < opaque_separator.unwrap()
        && opaque_separator.unwrap() < opaque_limit.unwrap()
        && opaque_producers.iter().filter(|(_, source)| has_call(source, "opaqueReceiver")).count() == 1
        && opaque_producers.iter().filter(|(_, source)| has_call(source, "opaqueSeparator")).count() == 1
        && opaque_producers.iter().filter(|(_, source)| has_call(source, "opaqueLimit")).count() == 1,
        "opaque receiver and raw argument producers must each run once, in source order: {opaque_producers:?}");
    let raw_separator = opaque_producers[opaque_separator.unwrap()].0;
    let raw_limit = opaque_producers[opaque_limit.unwrap()].0;
    let defined_branch = conditionals(opaque_effects).into_iter().find_map(|node| match node {
        HirExpr::Conditional(_, _, no, HirType::Array(item))
            if **item == HirType::Str
                && has_call(no, "__thaw_string_split")
                && has_call(no, "__thaw_js_handle_to_string")
                && has_call(no, "callDynamicValue") => Some(no.as_ref()),
        _ => None,
    }).expect("opaque defined-separator branch");
    let HirExpr::Call(limit_lambda, limit_arguments) = defined_branch else {
        panic!("limit conversion must be the outer defined-branch binding: {defined_branch:?}");
    };
    let HirExpr::Lambda(_, limit_parameters, _, separator_binding) = limit_lambda.as_ref() else {
        panic!("expected limit conversion binding lambda: {defined_branch:?}");
    };
    let limit_parameter = limit_parameters.first().expect("limit binding parameter");
    let limit_conversion = limit_arguments.first().expect("limit conversion argument");
    assert!(has_call(limit_conversion, "callDynamicValue")
        && has_call(limit_conversion, "getDynamicValue")
        && has_literal(limit_conversion, "__thaw_unary_plus")
        && has_var(limit_conversion, raw_limit),
        "the saved limit must use its live unary-plus conversion before separator conversion: {limit_conversion:?}");
    let HirExpr::Call(separator_lambda, separator_arguments) = separator_binding.as_ref() else {
        panic!("separator conversion must be nested after limit conversion: {separator_binding:?}");
    };
    let HirExpr::Lambda(_, separator_parameters, _, split_call) = separator_lambda.as_ref() else {
        panic!("expected separator conversion binding lambda: {separator_binding:?}");
    };
    let separator_parameter = separator_parameters.first().expect("separator binding parameter");
    let separator_conversion = separator_arguments.first().expect("separator conversion argument");
    assert!(matches!(separator_conversion,
        HirExpr::Call(callee, arguments)
            if matches!(callee.as_ref(), HirExpr::Var(name) if name == "__thaw_js_handle_to_string")
                && arguments.first().is_some_and(|value| has_var(value, raw_separator))
    ), "the saved separator must use its live string conversion after numeric conversion: {separator_conversion:?}");
    assert!(matches!(split_call.as_ref(), HirExpr::Call(callee, arguments)
        if matches!(callee.as_ref(), HirExpr::Var(name) if name == "__thaw_string_split")
            && arguments.len() == 3
            && matches!(&arguments[1], HirExpr::Var(name) if name == &separator_parameter.name)
            && matches!(&arguments[2], HirExpr::Var(name) if name == &limit_parameter.name)
    ), "the native split call must receive both converted bindings, including at zero limit: {split_call:?}");
}

#[test]
fn rejects_writes_not_representable_by_every_union_array_member() {
    for operation in [
        "value[0] = 1",
        "value[0] += 1",
        "value[0]++",
        "++value[0]",
    ] {
        let module = thaw_parser::parse_typescript(&format!(
            "function bad(value: number[] | string[]): void {{ {operation}; }}"
        ))
        .unwrap();
        let error = lower_module(&module).unwrap_err();
        assert!(
            error.contains("representable by every member"),
            "{operation}: {error}"
        );
    }
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
fn dynamic_computed_object_reads_form_heterogeneous_unions() {
    let program = lower(
        r#"function read(key: string): number | string | null | undefined {
            const mixed = { value: 1, label: "one", empty: null, absent: undefined };
            return mixed[key];
        }"#,
    );
    let HirStmt::Return(Some(HirExpr::DynamicPropAccess(_, _, fields, result))) =
        &program.functions[0].body[1]
    else {
        panic!("expected a dynamic property union return");
    };
    assert_eq!(fields[0].1, HirType::F64);
    assert_eq!(fields[1].1, HirType::Str);
    assert_eq!(
        result,
        &HirType::Union(vec![
            HirType::F64,
            HirType::Str,
            HirType::Null,
            HirType::Undefined
        ])
    );
}

#[test]
fn dynamic_computed_object_reads_flatten_tagged_heterogeneous_fields() {
    let program = lower(
        r#"function read(
            mixed: { value: number | undefined; label: string },
            key: string
        ): void { console.log(mixed[key]); }"#,
    );
    assert!(matches!(
        &program.functions[0].body[0],
        HirStmt::Expr(HirExpr::Call(_, args))
            if matches!(
                &args[0],
                HirExpr::DynamicPropAccess(
                    _, _, _, HirType::Union(members)
                ) if members == &[HirType::F64, HirType::Undefined, HirType::Str]
            )
    ));
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
fn enum_bitwise_initializers_use_javascript_wrapped_int32_values() {
    let program = lower(
        r#"enum Wrap {
               Zero = 4294967296 | 0,
               Negative = -4294967297 | 0,
               Xor = 4294967296 ^ -1,
               And = 4294967297.9 & 3.9,
               Left = 1 << 4294967297,
               NegativeCount = 1 << -31,
               Signed = 4294967295 >> 31,
               Unsigned = -1 >>> 0,
               Not = ~4294967296,
               NonFinite = (1 / 0) ^ (0 / 0)
           }
           enum Auto { Zero = 4294967296 | 0, Next }
           function zero(): number { return Wrap.Zero; }
           function negative(): number { return Wrap.Negative; }
           function xor(): number { return Wrap.Xor; }
           function and(): number { return Wrap.And; }
           function left(): number { return Wrap.Left; }
           function negativeCount(): number { return Wrap.NegativeCount; }
           function signed(): number { return Wrap.Signed; }
           function unsigned(): number { return Wrap.Unsigned; }
           function not(): number { return Wrap.Not; }
           function nonFinite(): number { return Wrap.NonFinite; }
           function next(): number { return Auto.Next; }
           function reverse(index: number): string | undefined { return Auto[index]; }"#,
    );
    for (name, expected) in [
        ("zero", 0.0),
        ("negative", -1.0),
        ("xor", -1.0),
        ("and", 1.0),
        ("left", 2.0),
        ("negativeCount", 2.0),
        ("signed", -1.0),
        ("unsigned", 4_294_967_295.0),
        ("not", -1.0),
        ("nonFinite", 0.0),
        ("next", 1.0),
    ] {
        let function = program.functions.iter().find(|function| function.name == name).unwrap();
        assert!(matches!(
            &function.body[0],
            HirStmt::Return(Some(HirExpr::Lit(HirLit::F64(value)))) if *value == expected
        ), "{name}: {:?}", function.body);
    }
    let reverse = program.functions.iter().find(|function| function.name == "reverse").unwrap();
    assert!(matches!(
        &reverse.body[0],
        HirStmt::Return(Some(HirExpr::EnumReverseLookup(_, entries)))
            if entries == &vec![(0.0, "Zero".into()), (1.0, "Next".into())]
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
fn lowers_common_object_union_properties_and_discriminant_type_narrowing() {
    let program = lower(
        r#"type Result =
               { kind: number; value: number; shared: string } |
               { kind: string; value: string; shared: string };
           function describe(result: Result): string {
               console.log(result.shared);
               if (typeof result.kind === "number") {
                   return String(result.value + 1);
               }
               return result.value + "!";
           }"#,
    );
    let function = &program.functions[0];
    assert!(matches!(
        &function.body[0],
        HirStmt::Expr(HirExpr::Call(callee, arguments))
            if matches!(callee.as_ref(), HirExpr::Var(name) if name == "console.log")
            && matches!(arguments.as_slice(), [HirExpr::Call(lambda, _)]
                if matches!(lambda.as_ref(), HirExpr::Lambda(_, _, HirType::Str, _)))
    ));
    let HirStmt::If(_, then_body, _) = &function.body[1] else {
        panic!("expected the discriminant guard")
    };
    assert!(matches!(
        &then_body[0],
        HirStmt::Return(Some(HirExpr::Call(_, arguments)))
            if matches!(arguments.as_slice(), [HirExpr::BinOp(BinOp::Add, value, _) ]
                if matches!(value.as_ref(), HirExpr::PropAccess(object, _, field)
                    if field == "value" && matches!(object.as_ref(), HirExpr::UnionValue(_, 0, _))))
    ));
    assert!(matches!(
        &function.body[2],
        HirStmt::Return(Some(HirExpr::Call(_, arguments)))
            // `value + "!"` is a ToPrimitive-aware addition now: the string member's `value` read is
            // the single argument of the outer closure instead of a two-argument concat call.
            if matches!(arguments.as_slice(), [HirExpr::PropAccess(object, _, field)]
                if field == "value" && matches!(object.as_ref(), HirExpr::UnionValue(_, 1, _)))
    ));
}

#[test]
fn rejects_properties_missing_from_an_object_union_member() {
    let module = thaw_parser::parse_typescript(
        r#"function read(value: { common: number } | { other: number }): number {
               return value.common;
           }"#,
    )
    .unwrap();
    // A property missing from one union member reads as `undefined` (as in JavaScript), so the
    // access has type `number | undefined` and returning it as `number` is rejected as a type
    // mismatch (formerly rejected earlier with "a union member has no such field").
    let error = lower_module(&module).unwrap_err();
    assert!(
        error.contains("Union([F64, Undefined])") && error.contains("expected F64"),
        "{error}"
    );
}

#[test]
fn flattens_tagged_fields_read_from_object_unions() {
    let program = lower(
        r#"type Mixed =
               { kind: number; value: number | undefined } |
               { kind: string; value: string | null } |
               { kind: boolean; value: boolean | null | undefined };
           function show(value: Mixed): void { console.log(value.value); }"#,
    );
    let HirStmt::Expr(HirExpr::Call(_, arguments)) = &program.functions[0].body[0] else {
        panic!("expected console.log call")
    };
    let [HirExpr::Call(adapter, _)] = arguments.as_slice() else {
        panic!("expected union property adapter")
    };
    assert!(matches!(
        adapter.as_ref(),
        HirExpr::Lambda(
            _,
            _,
            HirType::Union(members),
            _
        ) if members == &vec![
            HirType::F64,
            HirType::Undefined,
            HirType::Str,
            HirType::Null,
            HirType::Bool,
        ]
    ));
}

#[test]
fn narrows_object_unions_by_literal_discriminants() {
    let program = lower(
        r#"type Result =
               { kind: "success"; value: number } |
               { kind: "failure"; value: string } |
               { kind: true; value: boolean };
           function describe(result: Result): string {
               if (result.kind === "success") return String(result.value + 1);
               if ("failure" === result.kind) return result.value + "!";
               return result.value ? "true" : "false";
           }"#,
    );
    let function = &program.functions[0];
    for (statement, expected_index) in function.body[..2].iter().zip([0, 1]) {
        let HirStmt::If(_, branch, _) = statement else {
            panic!("expected discriminant branch")
        };
        assert!(format!("{branch:?}")
            .contains(&format!("UnionValue(Var(\"result\"), {expected_index},")));
    }
    assert!(format!("{:?}", function.body[2]).contains("UnionValue(Var(\"result\"), 2,"));
}

#[test]
fn correlates_destructured_discriminants_with_sibling_unions() {
    let program = lower(
        r#"type Result =
               { kind: "success"; value: number; detail: number } |
               { kind: "failure"; value: string; detail: string };
           function describe(result: Result): string {
               const { kind: tag, value, detail } = result;
               if (tag === "success" && value > 0) {
                   return String(value + detail);
               }
               if ("failure" === tag) return value + detail;
               return "zero";
           }"#,
    );
    let body = format!("{:?}", program.functions[0].body);
    assert!(body.contains("UnionValue(Var(\"value\"), 0,"));
    assert!(body.contains("UnionValue(Var(\"detail\"), 0,"));
    assert!(body.contains("UnionValue(Var(\"value\"), 1,"));
    assert!(body.contains("UnionValue(Var(\"detail\"), 1,"));
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
fn tuple_rest_past_last_element_is_empty_for_bindings_and_assignments() {
    let program = lower(
        r#"function main(value: [number]): void {
            let [first, , ...bound] = value;
            let assigned: number[] = [9];
            [first, , ...assigned] = value;
            console.log(bound.length, assigned.length);
        }"#,
    );
    let main = program.functions.iter().find(|function| function.name == "main").unwrap();
    let source = format!("{:?}", main.body);
    assert!(source.contains("ArrayLit([])"), "empty tuple rest must not index beyond the tuple: {source}");
}

#[test]
fn union_tuple_rest_past_short_member_is_empty() {
    let program = lower(
        r#"function main(value: [number] | [number, number]): void {
            const [first, , ...tail] = value;
            console.log(first, tail.length);
        }"#,
    );
    let main = program.functions.iter().find(|function| function.name == "main").unwrap();
    let source = format!("{:?}", main.body);
    assert!(source.contains("ArrayLit([])"), "short union member must yield an empty rest: {source}");
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
    assert!(matches!(
        &f.body[1],
        HirStmt::Expr(HirExpr::Call(callee, args))
            if matches!(callee.as_ref(), HirExpr::Var(name) if name == "console.log")
                && matches!(args.as_slice(), [HirExpr::Call(_, _)])
    ));
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
    assert!(matches!(&parts[2],
        HirExpr::Call(_, args) if matches!(args.as_slice(),
            [HirExpr::Call(_, slice_args)] if matches!(slice_args.first(), Some(HirExpr::Var(name)) if name == "tail"))));
    assert!(matches!(&parts[3], HirExpr::ArrayLit(values) if values.len() == 1));
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
        unstage_let(&f.body[0]),
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
        unstage_let(&program.functions[0].body[2]),
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
        unstage_let(&program.functions[0].body[1]),
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

    // The nested literal is evaluated once into a spread temporary; `x` is then read from that
    // temporary (not re-evaluated from the inner literal) and `label` is overridden.
    let HirStmt::Let(name, ty, HirExpr::ObjectLit(fields)) = unstage_let(&program.functions[0].body[0]) else {
        panic!("expected the unstaged point binding: {:?}", program.functions[0].body[0]);
    };
    assert_eq!(name, "point");
    assert_eq!(
        ty,
        HirType::Object(vec![("x".into(), HirType::F64), ("label".into(), HirType::Str)])
    );
    assert!(
        matches!(&fields[0], (field, HirExpr::PropAccess(object, _, property))
            if field == "x" && property == "x"
                && matches!(object.as_ref(), HirExpr::Var(binding) if binding.starts_with("__thaw_object_spread_"))),
        "{fields:?}"
    );
    assert_eq!(fields[1], ("label".into(), HirExpr::Lit(HirLit::Str("point".into()))));
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
    // The call result is bound once (as the `__thaw_object_spread_N` temporary) and every spread
    // field is read from that binding, never by re-calling the source.
    let HirStmt::Let(_, _, initializer) = &main.body[0] else {
        panic!("expected the spread object binding, got {:?}", main.body[0]);
    };
    assert_eq!(format!("{initializer:?}").matches("makeConfig").count(), 1, "{initializer:?}");
    // Unstaged, the literal reads `x` from the spread source and overrides `label`.
    let HirExpr::ObjectLit(fields) = unstage_object_literal(initializer, &[]) else {
        panic!("expected an object literal, got {initializer:?}");
    };
    assert!(
        matches!(&fields[0].1, HirExpr::PropAccess(_, _, field) if field == "x")
            && fields[1] == ("label".into(), HirExpr::Lit(HirLit::Str("point".into()))),
        "{fields:?}"
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
    let HirStmt::Let(_, ty, initializer) = &f.body[0] else { panic!("missing p binding"); };
    assert_eq!(*ty, HirType::Object(vec![("x".into(), HirType::F64), ("y".into(), HirType::F64)]));
    // Source field order is the physical layout; the declared `{ x, y }` order is realized
    // by the registered layout view, so assert the (staging-free) fields by name.
    let HirExpr::ObjectLit(fields) = unstage_object_literal(initializer, &[]) else {
        panic!("missing declared layout note: {initializer:?}");
    };
    let mut resolved = fields;
    resolved.sort_by(|left, right| left.0.cmp(&right.0));
    assert_eq!(resolved, vec![
        ("x".to_string(), HirExpr::Lit(HirLit::F64(1.0))),
        ("y".to_string(), HirExpr::Lit(HirLit::F64(2.0))),
    ]);
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
    // The literal keeps its source field order as the physical layout; the declared
    // `{ x, y }` shape is realized by the registered layout view, so compare the
    // fields by name rather than by position.
    let mut dist_args = dist_args.iter().map(|argument| unstage_object_literal(argument, &[])).collect::<Vec<_>>();
    let [HirExpr::ObjectLit(fields)] = dist_args.as_mut_slice() else {
        panic!("expected one object literal argument, got {dist_args:?}");
    };
    fields.sort_by(|left, right| left.0.cmp(&right.0));
    assert_eq!(
        fields,
        &vec![
            ("x".to_string(), HirExpr::Lit(HirLit::F64(1.0))),
            ("y".to_string(), HirExpr::Lit(HirLit::F64(2.0))),
        ]
    );
}

#[test]
fn contextually_types_a_callback_inside_an_object_argument() {
    let program = lower(
        r#"function register(options: { handler: (request: { path: string }) => string }): void {}
        function main(): void {
            register({ handler: (request) => request.path });
        }"#,
    );
    let HirStmt::Expr(HirExpr::Call(_, arguments)) = &program.functions[1].body[0] else {
        panic!("expected register call");
    };
    let arguments = arguments.iter().map(|argument| unstage_object_literal(argument, &[])).collect::<Vec<_>>();
    assert!(matches!(
        arguments.as_slice(),
        [HirExpr::ObjectLit(fields)]
            if matches!(&fields[0].1, HirExpr::Lambda(_, params, HirType::Str, _)
                if params[0].ty == HirType::Object(vec![("path".into(), HirType::Str)]))
    ), "{arguments:?}");
}

#[test]
fn dynamic_storage_object_options_keep_contextual_callback_parameters() {
    let program = lower(
        r#"
        interface Options {
            write?: (chunk: JsValue, encoding: string, done: () => void) => void;
        }
        declare function __thaw_typed_js_7772697465(options: Options): void;
        function main(): void {
            __thaw_typed_js_7772697465({ write(chunk, encoding, done) {
                console.log(String(chunk));
                done();
            } });
            __thaw_typed_js_7772697465({ write: (chunk, encoding, done) => {
                console.log(String(chunk));
                done();
            } });
        }
        "#,
    );
    let body = format!(
        "{:?}",
        program
            .functions
            .iter()
            .find(|function| function.name == "main")
            .unwrap()
            .body
    );
    assert!(
        body.contains("Function([JsValue, Str, Function([], Void)], Void)"),
        "the method and arrow parameters retain the contextual signature: {body}"
    );
}

#[test]
fn json_array_join_uses_the_live_method_while_native_array_join_stays_typed() {
    let program = lower(
        r#"
        function main(): void {
            const dynamic: any = [1, 2];
            console.log(dynamic.join('-'));
            console.log(Object.keys(dynamic).toString());
            const native: number[] = [3, 4];
            console.log(native.join('-'));
        }
        "#,
    );
    let body = format!(
        "{:?}",
        program
            .functions
            .iter()
            .find(|function| function.name == "main")
            .unwrap()
            .body
    );
    assert!(
        body.contains("__thaw_call_selected_dynamic_method"),
        "a Json array must invoke its live join method: {body}"
    );
    assert!(
        body.contains("__thaw_number_array_join"),
        "a native number array must retain its typed join path: {body}"
    );
}

#[test]
fn contextually_types_a_callback_in_an_interface_method_call() {
    let module = thaw_parser::parse_typescript(
        r#"
        interface Source {
            on(event: 'data', callback: (chunk: string) => void): void;
        }
        declare function source(): Source;
        function main(): void {
            source().on('data', (chunk) => console.log(chunk));
            source().on('data', () => console.log('done'));
        }
        "#,
    )
    .unwrap();
    lower_module(&module).unwrap();
}

#[test]
fn contextually_types_a_callback_passed_to_a_dynamic_function() {
    let module = thaw_parser::parse_typescript(
        r#"
        function invoke(subscribe: JsValue): void {
            subscribe((value) => console.log(value));
        }
        function main(): void {}
        "#,
    )
    .unwrap();
    lower_module(&module).unwrap();
}

#[test]
fn dynamic_callback_preserves_explicit_native_signature() {
    let module = thaw_parser::parse_typescript(
        r#"
        function invoke(register: JsValue): void {
            register((value: number): number => value * 2);
        }
        function main(): void {}
        "#,
    )
    .unwrap();
    lower_module(&module).unwrap();
}

#[test]
fn lowers_numeric_typed_array_constructors_through_the_dynamic_host() {
    let program = lower(
        r#"
        function main(): void {
            const bytes = new Uint8Array([1, 2, 255]);
            bytes[1] = 4;
            console.log(bytes.length, bytes[1]);
        }
        "#,
    );
    assert!(format!("{:?}", program.functions[0].body).contains("constructDynamicValue"));
    assert!(format!("{:?}", program.functions[0].body).contains("setDynamicPropertyJson"));
}

#[test]
fn lowers_abort_controller_through_the_dynamic_host() {
    let program = lower(
        r#"
        function main(): void {
            const controller = new AbortController();
            controller.abort("stop");
            console.log(controller.signal.aborted);
        }
        "#,
    );
    assert!(format!("{:?}", program.functions[0].body).contains("constructDynamicValue"));
}

/// `AbortSignal.timeout(ms)`/`AbortSignal.any(signals)` used to fail to
/// compile ("cannot infer the type of ... call to unknown function
/// AbortSignal.timeout") even though the QuickJS Fallback engine
/// already implements both correctly (`platform_globals/workers/
/// abort_timers.js`) -- the native compiler's dynamic-value receiver
/// inference (`infer_member_receiver_type`) recognized `Atomics`/
/// `crypto`/`process` as bridgeable global namespace objects but not
/// `AbortSignal`, and even once that's added, the identifier-lowering
/// match arm that turns a bare `AbortSignal` reference into a real
/// `getDynamicValue("AbortSignal")` call needs the same name added
/// too (found by rebuilding and hitting "unknown variable
/// `AbortSignal`" after only the first fix).
#[test]
fn lowers_abort_signal_static_methods_through_the_dynamic_host() {
    let program = lower(
        r#"
        function main(): void {
            const timeoutSignal = AbortSignal.timeout(50);
            console.log(timeoutSignal.aborted);
            const controller = new AbortController();
            const combined = AbortSignal.any([controller.signal]);
            console.log(combined.aborted);
        }
        "#,
    );
    let body = format!("{:?}", program.functions[0].body);
    assert!(body.contains("getDynamicValue"));
    // `callDynamicMethod` is now the selected-method form (`__thaw_call_selected_dynamic_method_*`).
    assert!(body.contains("__thaw_call_selected_dynamic_method"), "{body}");
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
            const data: string = "not an array";
            data[0] = 1;
        }"#,
    )
    .unwrap();
    assert!(lower_module(&module).is_err());
}

/// `JSON.parse` produces a `Json` value, and assigning a plain native value
/// into one of its indices used to be rejected outright (`coerce_to_declared`
/// had no general native-to-`Json` coercion, so the assigned `1` never
/// matched the declared `Json` element type) even though it's completely
/// ordinary JS. `coerce_to_declared`'s `Json` branch now boxes any
/// `json_convertible_native_type` value the same way `JSON.stringify`
/// already does for its own argument (`wrap_native_value_as_json`), so this
/// compiles like real JS would expect.
#[test]
fn assigns_a_native_value_into_a_json_indexed_slot() {
    let module = thaw_parser::parse_typescript(
        r#"function main(): void {
            const data = JSON.parse("[]");
            data[0] = 1;
        }"#,
    )
    .unwrap();
    assert!(lower_module(&module).is_ok());
}

/// A `Json`-typed receiver (from `JSON.parse`, same as the sibling test
/// above) calling a name this codebase also uses for a native array
/// builtin (`includes`) must still reach `lower_native_instance_
/// builtin`'s own clear "requires ... receiver, got Json" error --
/// not fall through to a generic dynamic-value dispatch path that
/// only actually supports a genuine `JsValue` receiver, which would
/// otherwise regress this into an opaque "call to undeclared function"
/// at codegen time instead. A real regression while fixing
/// `instanceof Date`/Date-method dispatch on a genuine `JsValue`
/// receiver (round 19's Date fix broadened this same gate to `JsValue
/// | Json` at first, catching a `Json` case it was never meant to
/// touch -- found via glob's own async `glob(...)` resolving to a
/// plain JSON array, see `[[project_npm_interop_gaps_20]]`).
/// A real native Date that crosses into `any` retains its private Date
/// identity through the JSON bridge, so `instanceof Date` can recognize it.
#[test]
fn instanceof_date_uses_trusted_json_origin_for_a_real_date() {
    let module = thaw_parser::parse_typescript(
        r#"function main(): boolean {
            const data: any = new Date(1579046400000);
            return data instanceof Date;
        }"#,
    )
    .unwrap();
    assert!(lower_module(&module).is_ok());
}

/// A real Date in an `any` slot can dispatch Date prototype methods
/// after the native-to-JSON bridge preserves its origin.
#[test]
fn date_method_call_promotes_a_real_date_through_json() {
    let module = thaw_parser::parse_typescript(
        r#"function main(): string {
            const data: any = new Date(1579046400000);
            return data.toISOString();
        }"#,
    )
    .unwrap();
    assert!(lower_module(&module).is_ok());
}

/// `Number.isNaN`/`isFinite` on a `Json`-typed value must actually
/// inspect the decoded runtime value (via `JsonAsNumber`, backed by
/// `thaw_json_as_number`), not unconditionally return the compile-time
/// literal `false` the way every other non-`F64` type still does. A
/// bare `lower_module(...).is_ok()` check can't tell these two outcomes
/// apart -- lowering succeeds either way, only the *runtime answer*
/// differs -- so this inspects the compiled `main` body directly,
/// confirming it actually calls the same `__thaw_number_is_nan`
/// intrinsic the `F64` case already uses (via `HirExpr::JsonAsNumber`),
/// not a hardcoded `false` literal.
#[test]
fn number_is_nan_inspects_a_json_valued_operand_at_runtime() {
    let program = lower(
        r#"function main(): boolean {
            const value = JSON.parse('{"$__thaw_non_finite$": "NaN"}');
            return Number.isNaN(value);
        }"#,
    );
    let main = program
        .functions
        .iter()
        .find(|function| function.name == "main")
        .unwrap();
    let body = format!("{:?}", main.body);
    assert!(body.contains("__thaw_number_is_nan"), "{body}");
    assert!(body.contains("JsonAsNumber"), "{body}");
    assert!(!body.contains("Bool(false)"), "{body}");
}

#[test]
fn json_and_jsvalue_includes_use_selected_javascript_methods() {
    let json_program = lower(
        r#"function main(value: Json, needle: any): boolean {
            return value.includes(needle);
        }"#,
    );
    let json_body = format!("{:?}", json_program.functions);
    assert!(json_body.contains("__thaw_call_selected_dynamic_method"), "{json_body}");
    assert!(json_body.contains("__thaw_json_borrowed_handle_id"), "{json_body}");
    assert!(json_body.contains("retainDynamicJson"), "{json_body}");
    assert!(!json_body.contains("__thaw_json_search"), "{json_body}");

    let jsvalue_program = lower(
        r#"function main(value: JsValue, needle: any): boolean {
            return value.includes(needle);
        }"#,
    );
    let jsvalue_body = format!("{:?}", jsvalue_program.functions);
    assert!(jsvalue_body.contains("__thaw_call_selected_dynamic_method"), "{jsvalue_body}");
}

#[test]
fn known_string_and_homogeneous_array_includes_keep_native_lowering() {
    let program = lower(
        r#"function main(text: string, values: number[]): boolean {
            return text.includes("needle") || values.includes(42);
        }"#,
    );
    let body = format!("{:?}", program.functions);
    assert!(body.contains("__thaw_string_includes"), "{body}");
    assert!(body.contains("__thaw_number_array_includes"), "{body}");
    assert!(!body.contains("__thaw_call_selected_dynamic_method"), "{body}");
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
    // The receiver is captured once and the `apply` property is read from that capture and
    // called through a separate callable capture (method-call evaluation order), instead of
    // being called directly off the property access.
    let HirStmt::Expr(HirExpr::Call(_, args)) = &program.functions[0].body[1] else {
        panic!("expected a console call, got {:?}", program.functions[0].body[1]);
    };
    let call = format!("{:?}", args[0]);
    assert!(call.contains("__thaw_method_receiver_"), "{call}");
    assert!(call.contains("__thaw_method_callable_"), "{call}");
    assert!(
        call.contains("PropAccess(Var(\"__thaw_method_receiver_") && call.contains("\"apply\")"),
        "{call}"
    );
}

/// `typeof e` for a `catch (e)` binding reads the preserved exception tag,
/// and a custom property read
/// (`e.status`) dispatches by caught-value tag: legacy framed strings use a
/// typed property projection while live JavaScript errors use JsonGet --
/// real trigger: koa's `onerror` reading an
/// `http-errors` error's `status`. `e.name`/`e.message` keep using the
/// existing tagged-string helpers.
#[test]
fn typeof_and_custom_properties_on_a_caught_error() {
    let program = lower(
        r#"
            function main(): void {
                try {
                    throw new Error("boom");
                } catch (e) {
                    console.log(typeof e, e.name, e.message, e.status);
                }
            }
        "#,
    );
    let source = format!("{:?}", program.functions);
    assert!(source.contains("__thaw_exception_typeof"), "{source}");
    assert!(
        source.contains("__thaw_error_property_json"),
        "legacy caught strings should use typed property projection:\n{source}"
    );
    assert!(source.contains("JsonGet"), "live caught errors should use JsonGet:\n{source}");
    assert!(
        !source.contains("Var(\"__thaw_error_property\")"),
        "catch carrier must not use the string-only helper:\n{source}"
    );
    assert!(source.contains("__thaw_error_name"), "{source}");
    assert!(source.contains("__thaw_error_message"), "{source}");
}

#[test]
fn rejects_unsafe_values_for_mutating_union_array_methods() {
    for expression in [
        "value.push(1)",
        "value.unshift(1)",
        "value.splice(0, 0, 1)",
        "value.fill(1)",
    ] {
        let source = format!(
            "function mutate(value: number[] | string[]): void {{ {expression}; }}"
        );
        let module = thaw_parser::parse_typescript(&source).expect("parse error");
        let error = lower_module(&module).unwrap_err();
        assert!(
            error.contains("not safe for every array union member"),
            "unexpected error for {expression}: {error}"
        );
    }
}


#[test]
fn function_operations_guard_property_access_before_argument_bindings() {
    let program = lower(r#"
        function effect(): number { return 1; }
        function main(): void {
            const f = function (this: number, value: number): number { return value; };
            f.call(effect(), effect());
            f.apply(effect(), [effect()] as [number]);
            f.bind(effect(), effect());
        }
    "#);
    let lowered = format!("{:?}", program.functions);
    for property in ["call", "apply", "bind"] {
        assert!(lowered.contains(&format!("Cannot read properties of undefined (reading '{property}')")),
            "{lowered}");
    }
    assert!(lowered.contains("__thaw_function_operation_target_"), "{lowered}");
    assert!(lowered.contains("__thaw_function_bind_target_"), "{lowered}");
}

#[test]
fn optional_function_calls_check_raw_and_wrapped_pointer_absence() {
    let program = lower(r#"
        function maybe(present: boolean): ((value: number) => number) | undefined {
            if (present) return value => value + 1;
            return undefined;
        }
        function effect(): number { return 1; }
        function main(): void {
            const direct = (value: number): number => value;
            direct?.(effect());
            const wrapped = maybe(false);
            wrapped?.(effect());
            const holder: { handler: (value: number) => number } = { handler: direct };
            holder.handler?.(effect());
        }
    "#);
    let lowered = format!("{:?}", program.functions);
    assert!(lowered.matches("__thaw_optional_callee_").count() >= 3, "{lowered}");
    assert!(lowered.contains("OptionalNone"), "{lowered}");
}

#[test]
fn saved_method_call_captures_callee_then_arguments_before_absence_guard() {
    let program = lower(r#"
        class Box {
            read(value: number): number { return value; }
        }
        function effect(): number { return 1; }
        function main(): number {
            const box = new Box();
            const saved = box.read;
            return saved(effect());
        }
    "#);
    let main = program.functions.iter().find(|function| function.name == "main").unwrap();
    let lowered = format!("{:?}", main.body);
    // A method that does not use `this` is extracted as a closure over its receiver
    // (`__thaw_method_reference_N`), which can never be absent, so the old runtime "Value is not
    // a function" guard is gone. The call still evaluates its argument (`effect()`) exactly once,
    // as the argument of the saved function, after the callee value was captured.
    assert!(lowered.contains("__thaw_method_reference_"), "{lowered}");
    assert!(lowered.contains("__thaw_class_Box_method_read"), "{lowered}");
    assert!(
        matches!(main.body.last(), Some(HirStmt::Return(Some(HirExpr::Call(callee, arguments))))
            if matches!(callee.as_ref(), HirExpr::Var(name) if name == "saved")
                && matches!(arguments.as_slice(), [HirExpr::Call(effect, _)]
                    if matches!(effect.as_ref(), HirExpr::Var(name) if name == "effect"))),
        "{lowered}"
    );
}

#[test]
fn optional_object_method_call_keeps_receiver_parameter() {
    let program = lower(r#"
        function effect(): number { return 1; }
        function main(): number | undefined {
            const object = {
                value: 2,
                read(extra: number): number { return this.value + extra; }
            };
            return object.read?.(effect());
        }
    "#);
    let main = program.functions.iter().find(|function| function.name == "main").unwrap();
    let lowered = format!("{:?}", main.body);
    assert!(lowered.contains("__thaw_optional_method_receiver_"), "{lowered}");
    assert!(lowered.contains("__thaw_object_method_receiver"), "{lowered}");
    assert!(lowered.contains("__thaw_optional_callee_"), "{lowered}");
}

#[test]
fn object_method_returning_this_keeps_the_actual_receiver_value() {
    let program = lower(r#"
        function main(): boolean {
            const o = { n: 0, inc() { this.n++; return this; } };
            return o.inc().inc() === o;
        }
    "#);
    let main = program.functions.iter().find(|function| function.name == "main").unwrap();
    let lowered = format!("{:?}", main.body);
    assert!(lowered.contains("access: Value"), "{lowered}");
    assert!(!lowered.contains("__thaw_method_receiver_copy_"), "{lowered}");
    assert!(lowered.contains("PropAssign"), "{lowered}");
}

#[test]
fn rest_function_bind_checks_missing_property_before_bound_arguments() {
    let program = lower(r#"
        function effect(): number { return 1; }
        function sum(...values: number[]): number { return values[0]; }
        function main(): void {
            const bound = sum.bind(undefined, effect());
            console.log(bound());
        }
    "#);
    let main = program.functions.iter().find(|function| function.name == "main").unwrap();
    let lowered = format!("{:?}", main.body);
    assert!(lowered.contains("__thaw_rest_bind_target_"), "{lowered}");
    assert!(lowered.contains("Cannot read properties of undefined (reading 'bind')"), "{lowered}");
}

#[test]
fn optional_method_and_plain_field_calls_contextually_type_callbacks() {
    let program = lower(r#"
        function main(): void {
            const object = {
                value: 2,
                read(callback: (n: number) => number): number {
                    return callback(this.value);
                }
            };
            const plain: { run: (callback: (n: number) => number) => number } = {
                run: (callback) => callback(3)
            };
            console.log(object.read?.(n => n + 1));
            console.log(plain.run?.(n => n + 1));
        }
    "#);
    let main = program.functions.iter().find(|function| function.name == "main").unwrap();
    let lowered = format!("{:?}", main.body);
    assert!(lowered.contains("__thaw_object_method_receiver"), "{lowered}");
    assert!(lowered.matches("__thaw_optional_callee_").count() >= 2, "{lowered}");
    assert!(lowered.contains("F64"), "{lowered}");
}

#[test]
fn logical_operators_over_different_types_yield_the_union_of_the_operands() {
    for src in [
        "function main(): void { const b: boolean = true; const v: number = 3; console.log(b && v); }",
        "function main(): void { const b: boolean = true; const v: number = 3; console.log(b || v); }",
        "function main(): void { const v: number = 3; const s: string = \"x\"; const r: number | string = v && s; console.log(r); }",
        "async function main(): Promise<void> { console.log(true && (await Promise.reject<number>(41))); }",
    ] {
        let module = thaw_parser::parse_typescript(src).unwrap();
        assert!(lower_module(&module).is_ok(), "{src}");
    }
    let module = thaw_parser::parse_typescript(
        "function main(): void { const v: number = 3; const s: string = v && \"x\"; }",
    ).unwrap();
    assert!(lower_module(&module).is_err(), "the union result must not silently narrow to one operand type");
}

#[test]
fn boolean_logical_expression_keeps_dynamic_rhs_out_of_capture_lambda() {
    use crate::{HirExpr, HirLit, HirStmt, HirType};

    let program = lower(r#"
        function isReady(): boolean { return true; }
        function hasValue(value: JsValue): boolean { return true; }
        function main(value: JsValue): boolean {
            return isReady() && hasValue(value);
        }
    "#);
    let main = program.functions.iter().find(|function| function.name == "main").unwrap();
    let expression = match main.body.as_slice() {
        [HirStmt::Return(Some(expression))] => expression,
        body => panic!("expected one return expression, got {body:#?}"),
    };
    assert!(
        !format!("{expression:#?}").contains("Lambda("),
        "a Bool/Bool logical expression must not capture its JsValue RHS in a temporary lambda: {expression:#?}",
    );
    assert!(matches!(
        expression,
        HirExpr::Conditional(test, _, no, HirType::Bool)
            if matches!(test.as_ref(), HirExpr::Call(callee, _)
                if matches!(callee.as_ref(), HirExpr::Var(name) if name == "isReady"))
            && matches!(no.as_ref(), HirExpr::Lit(HirLit::Bool(false)))
    ), "&& must evaluate the left test once and keep the RHS lazy: {expression:#?}");
}

#[test]
fn array_and_string_end_arguments_default_only_for_undefined() {
    let program = lower(r#"
        function arraySliceOmitted(a: number[]): number[] { return a.slice(); }
        function arraySliceUndefined(a: number[]): number[] { return a.slice(0, undefined); }
        function arraySliceOptional(a: number[], end?: number): number[] { return a.slice(0, end); }
        function arraySliceUnionEnd(a: number[], end: number | string | undefined): number[] { return a.slice(0, end); }
        function arraySliceJson(a: number[], end: any): number[] { return a.slice(0, end); }
        function arraySliceJsValue(a: number[], end: JsValue): number[] { return a.slice(0, end); }
        function arraySliceNull(a: number[]): number[] { return a.slice(0, null); }
        function arraySliceNaN(a: number[]): number[] { return a.slice(0, 0 / 0); }
        function arraySliceUnion(a: number[] | string[], end: number | undefined): number[] | string[] { return a.slice(0, end); }
        function arraySliceSpread(a: number[], args: [number, number | undefined]): number[] { return a.slice(...args); }
        function arraySliceSpreadUnion(a: number[] | string[], args: [number, number | undefined]): number[] | string[] { return a.slice(...args); }
        function fillOmitted(a: number[]): number[] { return a.fill(1); }
        function fillUndefined(a: number[], end: number | undefined): number[] { return a.fill(1, 0, end); }
        function fillLiteralUndefined(a: number[]): number[] { return a.fill(1, 0, undefined); }
        function fillUnion(a: number[] | (number | undefined)[], end?: number): void { a.fill(1, 0, end); }
        function fillSpread(a: number[], args: [number, number, number | undefined]): number[] { return a.fill(...args); }
        function copyOmitted(a: number[]): number[] { return a.copyWithin(0, 0); }
        function copyUndefined(a: number[], end: number | undefined): number[] { return a.copyWithin(0, 0, end); }
        function copyLiteralUndefined(a: number[]): number[] { return a.copyWithin(0, 0, undefined); }
        function copyUnion(a: number[] | string[], end?: number): number[] | string[] { return a.copyWithin(0, 0, end); }
        function copySpread(a: number[], args: [number, number, number | undefined]): number[] { return a.copyWithin(...args); }
        function endsOmitted(s: string): boolean { return s.endsWith("x"); }
        function endsUndefined(s: string, end: any): boolean { return s.endsWith("x", end); }
        function endsLiteralUndefined(s: string): boolean { return s.endsWith("x", undefined); }
        function endsSpread(s: string, args: [string, number | undefined]): boolean { return s.endsWith(...args); }
        function stringSliceOmitted(s: string): string { return s.slice(); }
        function stringSliceUndefined(s: string, end?: number): string { return s.slice(0, end); }
        function stringSliceLiteralUndefined(s: string): string { return s.slice(0, undefined); }
        function stringSliceSpread(s: string, args: [number, number | undefined]): string { return s.slice(...args); }
        function substringOmitted(s: string): string { return s.substring(); }
        function substringUndefined(s: string, end: number | undefined): string { return s.substring(0, end); }
        function substringLiteralUndefined(s: string): string { return s.substring(0, undefined); }
        function substrOmitted(s: string): string { return s.substr(); }
        function substrUndefined(s: string, end: number | undefined): string { return s.substr(0, end); }
        function substrLiteralUndefined(s: string): string { return s.substr(0, undefined); }

        function arrayReceiver(): number[] { return [1]; }
        function arrayStart(): number { return 0; }
        function arrayEnd(): number | undefined { return undefined; }
        function arrayOrder(): number[] { return arrayReceiver().slice(arrayStart(), arrayEnd()); }
        function fillValue(): number { return 1; }
        function fillStart(): number { return 0; }
        function fillEnd(): number | undefined { return undefined; }
        function fillOrder(): number[] { return arrayReceiver().fill(fillValue(), fillStart(), fillEnd()); }
        function copyTarget(): number { return 0; }
        function copyStart(): number { return 0; }
        function copyEnd(): number | undefined { return undefined; }
        function copyOrder(): number[] { return arrayReceiver().copyWithin(copyTarget(), copyStart(), copyEnd()); }
        function stringReceiver(): string { return "abcd"; }
        function stringStart(): number { return 0; }
        function stringEnd(): number | undefined { return undefined; }
        function stringOrder(): string { return stringReceiver().slice(stringStart(), stringEnd()); }
        function endNeedle(): string { return "x"; }
        function endsPosition(): number | undefined { return undefined; }
        function endsOrder(): boolean { return stringReceiver().endsWith(endNeedle(), endsPosition()); }
        function arrayArgs(): [number, number | undefined] { return [arrayStart(), arrayEnd()]; }
        function arraySpreadOrder(): number[] { return arrayReceiver().slice(...arrayArgs()); }
        function fillArgs(): [number, number, number | undefined] { return [fillValue(), fillStart(), fillEnd()]; }
        function fillSpreadOrder(): number[] { return arrayReceiver().fill(...fillArgs()); }
        function copyArgs(): [number, number, number | undefined] { return [copyTarget(), copyStart(), copyEnd()]; }
        function copySpreadOrder(): number[] { return arrayReceiver().copyWithin(...copyArgs()); }
        function stringArgs(): [string, number | undefined] { return [endNeedle(), endsPosition()]; }
        function endsSpreadOrder(): boolean { return stringReceiver().endsWith(...stringArgs()); }
        function stringSliceArgs(): [number, number | undefined] { return [stringStart(), stringEnd()]; }
        function stringSliceSpreadOrder(): string { return stringReceiver().slice(...stringSliceArgs()); }
    "#);

    fn visit_expr(expr: &HirExpr, visit: &mut impl FnMut(&HirExpr)) {
        visit(expr);
        match expr {
            HirExpr::Call(callee, args) => { visit_expr(callee, visit); for arg in args { visit_expr(arg, visit); } }
            HirExpr::FunctionCallWithThis(callee, this_arg, args, _, _) => {
                visit_expr(callee, visit); visit_expr(this_arg, visit); for arg in args { visit_expr(arg, visit); }
            }
            HirExpr::Lambda(_, _, _, body) | HirExpr::TypedClosure(_, body)
            | HirExpr::NonArrowFunction(body) | HirExpr::RecursiveClosure(_, _, body)
            | HirExpr::NonArrowThis { payload: body, .. } => visit_expr(body, visit),
            HirExpr::Block(statements) => for statement in statements { visit_stmt(statement, visit); },
            HirExpr::Conditional(test, yes, no, _) => { visit_expr(test, visit); visit_expr(yes, visit); visit_expr(no, visit); }
            HirExpr::BinOp(_, left, right) | HirExpr::EvalThen(left, right)
            | HirExpr::Index(left, right) | HirExpr::JsonIndex(left, right) | HirExpr::JsonKey(left, right) => {
                visit_expr(left, visit); visit_expr(right, visit);
            }
            HirExpr::ArrayLit(values) | HirExpr::DynamicCall(_, values) => for value in values { visit_expr(value, visit); },
            HirExpr::IndexAssign(array, index, value) => { visit_expr(array, visit); visit_expr(index, visit); visit_expr(value, visit); }
            HirExpr::ArrayAlloc(value, _) | HirExpr::ArrayLen(value) | HirExpr::Await(value)
            | HirExpr::AwaitPromise(value, _) | HirExpr::TypedIndex(value, _, _)
            | HirExpr::OptionalSome(value, _) | HirExpr::OptionalIsNone(value, _)
            | HirExpr::OptionalValue(value, _) | HirExpr::NullableSome(value, _)
            | HirExpr::NullableIsNone(value, _) | HirExpr::NullableValue(value, _)
            | HirExpr::NullishSome(value, _) | HirExpr::NullishIsNull(value, _)
            | HirExpr::NullishIsUndefined(value, _) | HirExpr::NullishIsNone(value, _)
            | HirExpr::NullishValue(value, _) | HirExpr::JsValueAsJson(value)
            | HirExpr::JsonAsNumber(value) | HirExpr::JsonAsString(value) | HirExpr::JsonAsBool(value)
            | HirExpr::UnionInject(value, _, _) | HirExpr::UnionTag(value, _) | HirExpr::UnionValue(value, _, _) => visit_expr(value, visit),
            HirExpr::Assign(_, value) => visit_expr(value, visit),
            _ => {}
        }
    }
    fn visit_stmt(statement: &HirStmt, visit: &mut impl FnMut(&HirExpr)) {
        match statement {
            HirStmt::Expr(expr) | HirStmt::Return(Some(expr)) | HirStmt::Throw(expr) | HirStmt::Let(_, _, expr) => visit_expr(expr, visit),
            HirStmt::If(test, yes, no) => { visit_expr(test, visit); for statement in yes.iter().chain(no) { visit_stmt(statement, visit); } }
            HirStmt::While(test, body) => { visit_expr(test, visit); for statement in body { visit_stmt(statement, visit); } }
            _ => {}
        }
    }
    fn function<'a>(program: &'a HirProgram, name: &str) -> &'a HirFunction {
        program.functions.iter().find(|function| function.name == name).unwrap()
    }
    fn raw_end_guard(function: &HirFunction, predicate: fn(&HirExpr) -> bool) {
        let mut found = false;
        for statement in &function.body {
            visit_stmt(statement, &mut |node| {
                if let HirExpr::Conditional(test, yes, no, HirType::F64) = node {
                    if matches!(yes.as_ref(), HirExpr::Lit(HirLit::F64(value)) if value.is_infinite() && value.is_sign_positive())
                        && predicate(test) && shares_raw_argument(test, no)
                    {
                        found = true;
                    }
                }
            });
        }
        assert!(found, "undefined guard must select +Infinity before number conversion: {}", function.name);
    }
    fn vars(expr: &HirExpr, output: &mut Vec<String>) {
        visit_expr(expr, &mut |node| if let HirExpr::Var(name) = node { output.push(name.clone()); });
    }
    fn tuple_end_slots(expr: &HirExpr, output: &mut Vec<(String, usize, HirType)>) {
        visit_expr(expr, &mut |node| {
            if let HirExpr::TypedIndex(source, offset, ty) = node {
                if let (HirExpr::Var(source_name), HirExpr::Lit(HirLit::F64(slot))) =
                    (source.as_ref(), offset.as_ref())
                {
                    if source_name.starts_with("__thaw_native_spread_")
                        && *slot >= 0.0
                        && slot.fract() == 0.0
                    {
                        output.push((source_name.clone(), *slot as usize, ty.clone()));
                    }
                }
            }
        });
    }
    fn shares_raw_argument(test: &HirExpr, number: &HirExpr) -> bool {
        let mut test_vars = Vec::new();
        let mut number_vars = Vec::new();
        vars(test, &mut test_vars);
        vars(number, &mut number_vars);
        if test_vars.iter().any(|name| name.starts_with("__thaw_native_arg_") && number_vars.contains(name)) {
            return true;
        }
        let mut test_slots = Vec::new();
        let mut number_slots = Vec::new();
        tuple_end_slots(test, &mut test_slots);
        tuple_end_slots(number, &mut number_slots);
        test_slots.iter().any(|slot| number_slots.contains(slot))
    }
    fn shares_tuple_slot(test: &HirExpr, number: &HirExpr, expected_slot: usize) -> bool {
        let mut test_slots = Vec::new();
        let mut number_slots = Vec::new();
        tuple_end_slots(test, &mut test_slots);
        tuple_end_slots(number, &mut number_slots);
        test_slots.iter().any(|(source, slot, ty)| {
            *slot == expected_slot
                && number_slots.contains(&(source.clone(), *slot, ty.clone()))
        })
    }
    fn has_call(expr: &HirExpr, target: &str) -> bool {
        let mut found = false;
        visit_expr(expr, &mut |node| if matches!(node, HirExpr::Call(callee, _) if matches!(callee.as_ref(), HirExpr::Var(name) if name == target)) { found = true; });
        found
    }
    fn has_live_dynamic_undefined_check(expr: &HirExpr) -> bool {
        let mut found = false;
        visit_expr(expr, &mut |node| {
            if let HirExpr::JsonAsBool(value) = node {
                if let HirExpr::Call(callee, arguments) = value.as_ref() {
                    if matches!(callee.as_ref(), HirExpr::Var(name) if name == "callDynamicValueWithValue")
                        && matches!(arguments.as_slice(), [
                            HirExpr::Call(getter, names),
                            _,
                        ] if matches!(getter.as_ref(), HirExpr::Var(name) if name == "getDynamicValue")
                            && matches!(names.as_slice(), [HirExpr::Lit(HirLit::Str(name))] if name == "__thaw_is_undefined_dynamic_value")
                        )
                    {
                        found = true;
                    }
                }
            }
        });
        found
    }
    fn has_optional_guard(expr: &HirExpr) -> bool {
        let mut found = false;
        visit_expr(expr, &mut |node| if matches!(node, HirExpr::OptionalIsNone(..)) { found = true; });
        found
    }
    fn has_union_guard(expr: &HirExpr) -> bool {
        let mut found = false;
        visit_expr(expr, &mut |node| if matches!(node, HirExpr::UnionTag(..)) { found = true; });
        found
    }

    for name in ["arraySliceOptional", "fillUndefined", "copyUndefined", "stringSliceUndefined", "substringUndefined", "substrUndefined"] {
        raw_end_guard(function(&program, name), has_optional_guard);
    }
    raw_end_guard(function(&program, "arraySliceUnionEnd"), |test| has_union_guard(test) || has_optional_guard(test));
    raw_end_guard(function(&program, "arraySliceJson"), |test| has_call(test, "__thaw_json_is_undefined"));
    raw_end_guard(function(&program, "arraySliceJsValue"), has_live_dynamic_undefined_check);
    raw_end_guard(function(&program, "endsUndefined"), |test| has_call(test, "__thaw_json_is_undefined"));
    for name in ["arraySliceUnion", "arraySliceSpread", "arraySliceSpreadUnion", "fillUnion", "fillSpread", "copyUnion", "copySpread", "endsSpread", "stringSliceSpread"] {
        let f = function(&program, name);
        let mut found = false;
        for statement in &f.body {
            visit_stmt(statement, &mut |node| if matches!(node,
                HirExpr::Conditional(test, yes, no, HirType::F64)
                    if matches!(yes.as_ref(), HirExpr::Lit(HirLit::F64(value)) if value.is_infinite() && value.is_sign_positive())
                        && shares_raw_argument(test, no)
            ) { found = true; });
        }
        assert!(found, "union and spread methods retain a shared raw end guard: {name}");
    }
    for (name, expected_slot) in [
        ("arraySliceSpread", 1), ("arraySliceSpreadUnion", 1),
        ("fillSpread", 2), ("copySpread", 2), ("endsSpread", 1),
        ("stringSliceSpread", 1),
    ] {
        let f = function(&program, name);
        let mut found = false;
        for statement in &f.body {
            visit_stmt(statement, &mut |node| if let HirExpr::Conditional(test, _, no, HirType::F64) = node {
                if shares_tuple_slot(test, no, expected_slot) { found = true; }
            });
        }
        assert!(found, "spread end guard uses the same raw tuple slot {expected_slot}: {name}");
    }
    for name in ["arraySliceOmitted", "fillOmitted", "copyOmitted", "endsOmitted", "stringSliceOmitted", "substringOmitted", "substrOmitted"] {
        let f = function(&program, name);
        let mut has_infinity = false;
        for statement in &f.body {
            visit_stmt(statement, &mut |node| if matches!(node, HirExpr::Lit(HirLit::F64(value)) if value.is_infinite() && value.is_sign_positive()) { has_infinity = true; });
        }
        assert!(has_infinity, "omitted end defaults to +Infinity: {name}");
    }
    for name in ["arraySliceUndefined", "fillLiteralUndefined", "copyLiteralUndefined", "endsLiteralUndefined", "stringSliceLiteralUndefined", "substringLiteralUndefined", "substrLiteralUndefined"] {
        let f = function(&program, name);
        let mut explicit_undefined = false;
        for statement in &f.body {
            visit_stmt(statement, &mut |node| if matches!(node,
                HirExpr::Conditional(test, yes, no, HirType::F64)
                    if matches!(test.as_ref(), HirExpr::Lit(HirLit::Bool(true)) )
                        && matches!(yes.as_ref(), HirExpr::Lit(HirLit::F64(value)) if value.is_infinite() && value.is_sign_positive())
                        && matches!(no.as_ref(), HirExpr::Lit(HirLit::F64(value)) if value.is_nan())
            ) { explicit_undefined = true; });
        }
        assert!(explicit_undefined, "explicit undefined selects +Infinity lazily: {name}");
    }
    let null_fn = function(&program, "arraySliceNull");
    let mut null_is_zero = false;
    for statement in &null_fn.body {
        visit_stmt(statement, &mut |node| if matches!(node,
            HirExpr::Conditional(test, yes, no, HirType::F64)
                if matches!(test.as_ref(), HirExpr::Lit(HirLit::Bool(false)))
                    && matches!(yes.as_ref(), HirExpr::Lit(HirLit::F64(value)) if value.is_infinite() && value.is_sign_positive())
                    && matches!(no.as_ref(), HirExpr::Lit(HirLit::F64(0.0)))
        ) { null_is_zero = true; });
    }
    assert!(null_is_zero, "null stays numeric zero");
    let nan_fn = function(&program, "arraySliceNaN");
    let mut nan_is_preserved = false;
    for statement in &nan_fn.body {
        visit_stmt(statement, &mut |node| if matches!(node,
            HirExpr::Conditional(test, yes, no, HirType::F64)
                if matches!(test.as_ref(), HirExpr::Lit(HirLit::Bool(false)))
                    && matches!(yes.as_ref(), HirExpr::Lit(HirLit::F64(value)) if value.is_infinite() && value.is_sign_positive())
                    && matches!(no.as_ref(), HirExpr::Var(_))
        ) { nan_is_preserved = true; });
    }
    assert!(nan_is_preserved, "NaN stays on the provided-number branch");

    fn source_bindings<'a>(expr: &'a HirExpr, output: &mut Vec<(String, &'a HirExpr)>) {
        if let HirExpr::Call(callee, args) = expr {
            if let HirExpr::Lambda(_, params, _, body) = callee.as_ref() {
                for (param, arg) in params.iter().zip(args) {
                    if param.name.starts_with("__thaw_native_arg_") || param.name.starts_with("__thaw_native_spread_")
                        || param.name.contains("receiver")
                    {
                        output.push((param.name.clone(), arg));
                    }
                }
                source_bindings(body, output);
            } else {
                source_bindings(callee, output);
            }
            for arg in args { source_bindings(arg, output); }
        } else if let HirExpr::Block(statements) = expr {
            for statement in statements {
                if let HirStmt::Expr(value) | HirStmt::Return(Some(value)) | HirStmt::Let(_, _, value) = statement {
                    source_bindings(value, output);
                }
            }
        }
    }
    fn call_name(expr: &HirExpr, target: &str) -> bool {
        matches!(expr, HirExpr::Call(callee, _) if matches!(callee.as_ref(), HirExpr::Var(name) if name == target))
    }
    fn assert_source_order(program: &HirProgram, function_name: &str, expected: &[&str]) {
        let mut bindings = Vec::new();
        for statement in &function(program, function_name).body {
            if let HirStmt::Expr(value) | HirStmt::Return(Some(value)) = statement { source_bindings(value, &mut bindings); }
        }
        let positions = expected.iter().map(|target| bindings.iter().position(|(_, source)| call_name(source, target)).unwrap()).collect::<Vec<_>>();
        let counts = expected.iter().map(|target| bindings.iter().filter(|(_, source)| call_name(source, target)).count()).collect::<Vec<_>>();
        assert!(counts.iter().all(|count| *count == 1) && positions.windows(2).all(|pair| pair[0] < pair[1]),
            "receiver and raw call arguments are evaluated once in source order for {function_name}: {bindings:?}");
    }
    assert_source_order(&program, "arrayOrder", &["arrayReceiver", "arrayStart", "arrayEnd"]);
    assert_source_order(&program, "fillOrder", &["arrayReceiver", "fillValue", "fillStart", "fillEnd"]);
    assert_source_order(&program, "copyOrder", &["arrayReceiver", "copyTarget", "copyStart", "copyEnd"]);
    assert_source_order(&program, "stringOrder", &["stringReceiver", "stringStart", "stringEnd"]);
    assert_source_order(&program, "endsOrder", &["stringReceiver", "endNeedle", "endsPosition"]);
    assert_source_order(&program, "arraySpreadOrder", &["arrayReceiver", "arrayArgs"]);
    assert_source_order(&program, "fillSpreadOrder", &["arrayReceiver", "fillArgs"]);
    assert_source_order(&program, "copySpreadOrder", &["arrayReceiver", "copyArgs"]);
    assert_source_order(&program, "endsSpreadOrder", &["stringReceiver", "stringArgs"]);
    assert_source_order(&program, "stringSliceSpreadOrder", &["stringReceiver", "stringSliceArgs"]);
}



#[test]
fn json_backed_dictionary_is_not_treated_as_a_native_symbol_buffer() {
    let program = lower(r#"
        declare function accept(value: JsValue): void;
        function main(value: Record<string, symbol>): void {
            accept(value);
        }
    "#);
    let body = format!("{program:#?}");
    assert!(body.contains("retainDynamicJson"), "{body}");
    assert!(!body.contains("retainDynamicNative"), "{body}");
}

#[test]
fn ordinary_json_and_napi_calls_do_not_select_native_symbol_projection() {
    let json_program = lower(r#"
        declare function accept(value: Json): void;
        function main(value: Json): void { accept(value); }
    "#);
    let json_body = format!("{json_program:#?}");
    assert!(!json_body.contains("retainDynamicNative"), "{json_body}");

    let napi_program = lower(r#"
        declare function __thaw_typed_napi_616363657074(value: Json): void;
        function main(value: Json): void { __thaw_typed_napi_616363657074(value); }
    "#);
    let napi_body = format!("{napi_program:#?}");
    assert!(napi_body.contains("backend: Napi"), "{napi_body}");
    assert!(!napi_body.contains("retainDynamicNative"), "{napi_body}");
}

#[test]
fn symbol_bearing_native_composites_select_projection() {
    let program = lower(r#"
        declare function accept(value: JsValue): void;
        function main(value: { close: symbol[] }, key: symbol): void {
            accept(value);
            accept({ close: [key] });
        }
    "#);
    let body = format!("{program:#?}");
    assert!(body.contains("retainDynamicNative"), "{body}");
}

fn structural_class_argument_source(decorator: &str) -> String {
    format!(
        r#"
            declare function __thaw_typed_js_7368617065(value: {{ value: number }}): string;
            {decorator}class Item {{ value: number = 1; }}
            function main(): void {{ __thaw_typed_js_7368617065(new Item()); }}
        "#
    )
}

#[test]
fn undecorated_class_satisfies_external_structural_argument() {
    let module = thaw_parser::parse_typescript(&structural_class_argument_source(""))
        .expect("parse");
    let result = lower_module(&module);
    assert!(result.is_ok(), "class public source field value: number should satisfy {{ value: number }}; got: {}", result.unwrap_err());
}

#[test]
fn decorated_class_satisfies_external_structural_argument() {
    let module = thaw_parser::parse_typescript(&structural_class_argument_source("declare function Mark(value: any): any;\n@Mark\n"))
        .expect("parse");
    let result = lower_module(&module);
    assert!(result.is_ok(), "decorated class public source field value: number should satisfy {{ value: number }}; got: {}", result.unwrap_err());
}

#[test]
fn external_structural_argument_accepts_class_prototype_method() {
    let module = thaw_parser::parse_typescript(
        r#"
            declare function __thaw_typed_js_7368617065(value: {
                bump(value?: number): number;
                rest(...values: number[]): number;
                asyncBump(): Promise<number>;
            }): number;
            class Item {
                value: number = 1;
                bump(value?: number): number { return this.value; }
                rest(...values: number[]): number { return this.value; }
                async asyncBump(): Promise<number> { return this.value; }
            }
            function main(): void { __thaw_typed_js_7368617065(new Item()); }
        "#,
    ).expect("parse");
    let result = lower_module(&module);
    assert!(result.is_ok(), "class prototype methods should satisfy structural method contracts: {}", result.unwrap_err());
}

#[test]
fn external_structural_argument_rejects_missing_or_incompatible_class_methods() {
    for (class, diagnostic) in [
        ("class Item { value: number = 1; }", "missing required field `bump`"),
        ("class Item { value: number = 1; bump(value?: number): string { return 'wrong'; } }", "incompatible method signature"),
        ("class Item { value: number = 1; bump(value: string): number { return 1; } }", "incompatible method signature"),
    ] {
        let source = format!(
            "declare function __thaw_typed_js_7368617065(value: {{ bump(value?: number): number }}): number;\n{class}\nfunction main(): void {{ __thaw_typed_js_7368617065(new Item()); }}"
        );
        let module = thaw_parser::parse_typescript(&source).expect("parse");
        let error = lower_module(&module).expect_err("missing or incompatible method must be rejected");
        assert!(error.contains("bump"), "{error}");
        assert!(error.contains(diagnostic), "{error}");
        assert!(!error.contains("__thaw_class_identity_"), "{error}");
    }
}

#[test]
fn external_structural_argument_rejects_missing_and_incompatible_fields() {
    for source in [
        r#"
            declare function __thaw_typed_js_7368617065(value: { value: number }): string;
            class Item { other: number = 1; }
            function main(): void { __thaw_typed_js_7368617065(new Item()); }
        "#,
        r#"
            declare function __thaw_typed_js_7368617065(value: { value: number }): string;
            class Item { value: string = "wrong"; }
            function main(): void { __thaw_typed_js_7368617065(new Item()); }
        "#,
    ] {
        let module = thaw_parser::parse_typescript(source).expect("parse");
        let error = lower_module(&module).expect_err("invalid class source shape should be rejected");
        assert!(error.contains("value"), "{error}");
        assert!(!error.contains("__thaw_class_identity_"), "{error}");
    }
}

#[test]
fn external_class_union_mismatch_diagnostic_hides_identity_marker() {
    for target in ["{ name: string } | { count: number }", "Item | { name: string }"] {
        let source = format!(
            "declare function __thaw_typed_js_7368617065(value: {target}): void;\nclass Item {{ value: number = 1; }}\nclass Other {{ value: number = 1; }}\nfunction main(): void {{ __thaw_typed_js_7368617065(new Other()); }}"
        );
        let module = thaw_parser::parse_typescript(&source).expect("parse");
        let error = lower_module(&module).expect_err("incompatible class must not satisfy union target");
        assert!(!error.contains("__thaw_class_identity_"), "{error}");
    }
}

#[test]
fn external_structural_argument_recurses_through_nested_arrays_and_optional_fields() {
    let module = thaw_parser::parse_typescript(
        r#"
            declare function __thaw_typed_js_7368617065(value: {
                child: { value: number };
                children: { value: number }[];
                pair: [number, string];
                choice: number | string;
                absent?: string;
            }): void;
            class Child { value: number = 1; extra: string = "ok"; }
            class Holder {
                child: Child = new Child();
                children: Child[] = [];
                pair: [number, string] = [1, "ok"];
                choice: number | string = 1;
                extra: boolean = true;
            }
            function main(): void {
                const item = new Holder();
                const alias = item;
                __thaw_typed_js_7368617065(alias);
            }
        "#,
    ).expect("parse");
    assert!(lower_module(&module).is_ok(), "nested class fields should satisfy recursive structural targets");
}

#[test]
fn external_dictionary_source_contract_preserves_object_literal_coercion() {
    for (target, argument, succeeds) in [
        ("Record<string, number>", "{ a: 1 }", true),
        ("Record<string, number> | { tag: string }", "{ a: 1 }", true),
        ("Record<string, number> | undefined", "{ a: 1 }", true),
        ("Record<string, number> | null", "{ a: 1 }", true),
        ("Record<string, number> | null | undefined", "{ a: 1 }", true),
        ("Record<string, number>", "{ a: 'wrong' }", false),
        ("Record<string, number> | { tag: string }", "{ a: 'wrong' }", false),
    ] {
        let source = format!(
            "declare function __thaw_typed_js_7368617065(value: {target}): void;\nfunction main(): void {{ __thaw_typed_js_7368617065({argument}); }}"
        );
        let module = thaw_parser::parse_typescript(&source).expect("parse");
        let result = lower_module(&module);
        assert_eq!(result.is_ok(), succeeds, "target={target}, argument={argument}: {result:?}");
    }
}

#[test]
fn external_structural_argument_reports_nested_public_field_on_mismatch() {
    let module = thaw_parser::parse_typescript(
        r#"
            declare function __thaw_typed_js_7368617065(value: { child: { value: number } }): void;
            class Child { value: string = "wrong"; }
            class Holder { child: Child = new Child(); }
            function main(): void { __thaw_typed_js_7368617065(new Holder()); }
        "#,
    ).expect("parse");
    let error = lower_module(&module).expect_err("nested incompatible public field must fail");
    assert!(error.contains("field `child`"), "{error}");
    assert!(error.contains("field `value`"), "{error}");
    assert!(!error.contains("__thaw_class_identity_"), "{error}");
}

#[test]
fn external_structural_argument_preserves_nullable_and_undefined_contracts() {
    let module = thaw_parser::parse_typescript(
        r#"
            declare function __thaw_typed_js_7368617065(value: {
                nullable: string | null;
                optional: string | undefined;
                nullish: string | null | undefined;
                absent?: number;
            }): void;
            class Item {
                nullable: string | null = null;
                optional: string | undefined = undefined;
                nullish: string | null | undefined = undefined;
            }
            function main(): void { __thaw_typed_js_7368617065(new Item()); }
        "#,
    ).expect("parse");
    assert!(lower_module(&module).is_ok(), "matching nullable and undefined source contracts should pass");
}

#[test]
fn external_structural_argument_does_not_drop_optional_source_to_required() {
    let module = thaw_parser::parse_typescript(
        r#"
            declare function __thaw_typed_js_7368617065(value: { value: number }): void;
            class Item { value?: number; }
            function main(): void { __thaw_typed_js_7368617065(new Item()); }
        "#,
    ).expect("parse");
    let error = lower_module(&module).expect_err("optional source cannot satisfy a required field");
    assert!(error.contains("value"), "{error}");
}

#[test]
fn plain_object_does_not_satisfy_nominal_class_argument() {
    let module = thaw_parser::parse_typescript(
        r#"
            declare function __thaw_typed_js_7368617065(value: Item): string;
            class Item { value: number = 1; }
            function main(): void { __thaw_typed_js_7368617065({ value: 1 }); }
        "#,
    ).expect("parse");
    let error = lower_module(&module).expect_err("plain object must not acquire Item identity");
    assert!(!error.contains("__thaw_class_identity_"), "{error}");
}

#[test]
fn another_class_does_not_satisfy_nominal_class_argument() {
    let module = thaw_parser::parse_typescript(
        r#"
            declare function __thaw_typed_js_7368617065(value: Item): string;
            class Item { value: number = 1; }
            class Other { value: number = 1; }
            function main(): void { __thaw_typed_js_7368617065(new Other()); }
        "#,
    ).expect("parse");
    let error = lower_module(&module).expect_err("another class must not satisfy nominal Item");
    assert!(error.contains("Other") || error.contains("Item"), "{error}");
    assert!(!error.contains("__thaw_class_identity_"), "{error}");
}

#[test]
fn getter_only_class_field_satisfies_structural_contract_without_evaluation() {
    let module = thaw_parser::parse_typescript(
        r#"
            declare function __thaw_typed_js_7368617065(value: { value: number }): string;
            class Item { get value(): number { throw new Error("read"); } }
            function main(): void { __thaw_typed_js_7368617065(new Item()); }
        "#,
    ).expect("parse");
    let result = lower_module(&module);
    assert!(result.is_ok(), "getter return type should satisfy structural contract without running getter: {:?}", result.err());
}

#[test]
fn class_array_satisfies_top_level_structural_array_contract() {
    let module = thaw_parser::parse_typescript(
        r#"
            declare function __thaw_typed_js_7368617065(value: { value: number }[]): string;
            class Item { value: number = 1; extra: string = "ok"; }
            function main(): void { __thaw_typed_js_7368617065([new Item()]); }
        "#,
    ).expect("parse");
    assert!(lower_module(&module).is_ok(), "class instances should satisfy structural array elements");
}

/// `{__proto__: v}` sets the prototype (Node semantics): every non-computed key spelling
/// (identifier or string literal) lowers to a `proto` operation in the live object builder,
/// which installs the prototype only for an object or null value.
#[test]
fn object_literal_proto_setter_key_lowers_to_prototype_install() {
    for source in [
        "function main(): void { const p = { x: 1 }; const o = { __proto__: p }; console.log(o); }",
        "function main(): void { const o = { __proto__: null }; console.log(o); }",
        "function main(): void { const o = { '__proto__': null }; console.log(o); }",
        "function main(): void { const k = 'a'; const o = { __proto__: null, [k]: 1 }; console.log(o); }",
    ] {
        let hir = format!("{:?}", lower(source));
        assert!(hir.contains("Str(\"proto\")"), "{source}: {hir}");
    }
}

/// `{__proto__: a, __proto__: b}` is a SyntaxError in JavaScript and stays a compile error.
#[test]
fn object_literal_duplicate_proto_setter_is_compile_error() {
    let module = thaw_parser::parse_typescript(
        "function main(): void { const o = { __proto__: null, __proto__: null }; console.log(o); }",
    )
    .unwrap();
    let error = lower_module(&module).unwrap_err();
    assert!(error.contains("duplicate `__proto__` fields are not allowed"), "{error}");
}

/// Shorthand `{__proto__}` and computed `{["__proto__"]: v}` define an ordinary own
/// property and must keep lowering.
#[test]
fn object_literal_proto_shorthand_and_computed_key_still_lower() {
    let program = lower(
        r#"
            function main(): void {
                const __proto__ = 1;
                const shorthand = { __proto__ };
                const computed = { ["__proto__"]: 1 };
                console.log(shorthand, computed);
            }
        "#,
    );
    assert_eq!(program.functions.len(), 1);
}

/// `err.code = x` on an Error value (a framed `Str`) and every read of its own properties lower
/// to the runtime side table intrinsics instead of a "cannot assign to `.code` on Str" error.
#[test]
fn error_own_properties_lower_to_side_table_intrinsics() {
    let program = lower(
        r#"
            function main(): void {
                const e = new Error("boom") as any;
                e.code = "E_X";
                e["dyn"] = 1;
                console.log(e.code, e.status, Object.keys(e), Object.getOwnPropertyNames(e));
                console.log("code" in e, delete e.code, JSON.stringify(e));
                Object.assign(e, { extra: 1 });
                Error.captureStackTrace(e);
            }
        "#,
    );
    let source = format!("{:?}", program.functions);
    for intrinsic in [
        "__thaw_error_set_prop",
        "__thaw_error_get_prop",
        "__thaw_error_own_keys",
        "__thaw_error_has_prop",
        "__thaw_error_delete_prop",
        "__thaw_error_json_stringify",
    ] {
        assert!(source.contains(intrinsic), "missing {intrinsic}:\n{source}");
    }
}

/// Compound assignment and `++`/`--` on an Error property read and write the side table (they used
/// to be rejected with "not supported yet").
#[test]
fn error_property_compound_assignment_lowers_to_side_table_intrinsics() {
    for body in ["e.count += 1;", "e.count++;", "--e.count;", "e.count ??= 2;", "e[\"k\"] *= 3;"] {
        let source = format!(
            "function main(): void {{ const e = new Error(\"x\") as any; {body} }}"
        );
        let module = thaw_parser::parse_typescript(&source).expect("parse error");
        let program = lower_module(&module).unwrap_or_else(|error| panic!("{body}: {error}"));
        let lowered = format!("{:?}", program.functions);
        assert!(lowered.contains("__thaw_error_set_prop"), "{body}: {lowered}");
        assert!(lowered.contains("__thaw_error_get_prop"), "{body}: {lowered}");
    }
}

/// Reading a built-in string property keeps its dedicated lowering: only unknown names reach the
/// Error side table.
#[test]
fn string_builtin_property_reads_are_unchanged_by_error_properties() {
    let program = lower(
        r#"
            function main(): void {
                const s = "abc";
                console.log(s.length);
            }
        "#,
    );
    let source = format!("{:?}", program.functions);
    assert!(source.contains("__thaw_string_length"), "{source}");
    assert!(!source.contains("__thaw_error_get_prop"), "{source}");
}

/// The side-table intrinsics are typed: set/has/delete are `Bool`, get is `Json`, keys is `Str`.
#[test]
fn error_side_table_intrinsics_are_typed() {
    let program = lower(
        r#"
            function main(): void {
                const e = new Error("x") as any;
                const has: boolean = "k" in e;
                const gone: boolean = delete e.k;
                const keys: string[] = Object.keys(e);
                const text: string = JSON.stringify(e);
                const value: any = e.k;
                console.log(has, gone, keys, text, value);
            }
        "#,
    );
    assert_eq!(program.functions.len(), 1);
}

#[test]
fn closure_call_keeps_a_class_value_argument_live_for_an_any_parameter() {
    let program = lower(
        r#"
        class C { x = 1; }
        const show = (K: any) => typeof K;
        function main(): void { console.log(show(C)); }
        "#,
    );
    let main = program.functions.iter().find(|function| function.name == "main").unwrap();
    assert!(!format!("{:?}", main.body).contains("readDynamicValue"));
    assert!(format!("{:?}", main.body).contains("JsValueAsJson"));
}

#[test]
fn class_value_registers_a_native_constructor_bridge_only_when_eligible() {
    let eligible = format!("{:?}", lower(
        r#"
        class C { x = 1; dbl(): number { return this.x * 2; } }
        function main(): void { const k: any = C; const o = new k(); console.log(o.dbl()); }
        "#,
    ));
    assert!(eligible.contains("__thaw_ctor_bridge_C"));
    assert!(eligible.contains("constructDynamicValue"));
    let defaulted = format!("{:?}", lower(
        r#"
        class P { n: number; constructor(n: number = 1) { this.n = n; } }
        function main(): void { const k: any = P; const o = new k(); console.log(o.n); }
        "#,
    ));
    assert!(!defaulted.contains("__thaw_ctor_bridge_P"));
    let decorated = format!("{:?}", lower(
        r#"
        function tag(target: any): void {}
        @tag class D { x = 1; }
        function main(): void { const k: any = D; const o = new k(); console.log(o.x); }
        "#,
    ));
    assert!(!decorated.contains("__thaw_ctor_bridge_D"));
    let with_static = format!("{:?}", lower(
        r#"
        class S { static count = 0; id: number; constructor() { S.count += 1; this.id = S.count; } }
        class T extends S { }
        function main(): void { const k: any = T; const o = new k(); console.log(o.id); }
        "#,
    ));
    assert!(!with_static.contains("__thaw_ctor_bridge_S"));
    assert!(!with_static.contains("__thaw_ctor_bridge_T"));
}

#[test]
fn instanceof_against_an_any_variable_uses_the_host_instanceof() {
    let program = lower(
        r#"
        class C { x = 1; }
        function main(): void { const k: any = C; const o = new k(); console.log(o instanceof k); }
        "#,
    );
    let main = program.functions.iter().find(|function| function.name == "main").unwrap();
    assert!(format!("{:?}", main.body).contains("__thaw_json_host_instanceof"));
}
