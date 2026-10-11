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
    fn form_source<'a>(expression: &'a HirExpr) -> Option<&'a HirExpr> {
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
            HirExpr::ArrayLit(values) | HirExpr::DynamicCall(_, values) => {
                for value in values { visit_expr(value, visit); }
            }
            HirExpr::JsValueAsJson(value) | HirExpr::JsonAsNumber(value)
            | HirExpr::JsonAsString(value) | HirExpr::JsonAsBool(value)
            | HirExpr::TypedIndex(value, _, _) | HirExpr::JsonIndex(value, _)
            | HirExpr::Await(value) => visit_expr(value, visit),
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
            if matches!(node, HirExpr::Call(callee, _) if matches!(callee.as_ref(), HirExpr::Var(name) if name == target)) {
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
    fn conditionals<'a>(function: &'a HirFunction) -> Vec<&'a HirExpr> {
        let mut found = Vec::new();
        for statement in &function.body {
            visit_stmt(statement, &mut |node| {
                if matches!(node, HirExpr::Conditional(..)) { found.push(node); }
            });
        }
        found
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
                && matches!(test.as_ref(), HirExpr::OptionalIsNone(..))
                && !matches!(no.as_ref(), HirExpr::Lit(HirLit::F64(-1.0)))
    )), "optional undefined limit must lazily choose the unlimited sentinel: {optional:?}");
    assert!(conditionals(function(&program, "regex")).iter().any(|node| matches!(node,
        HirExpr::Conditional(test, yes, _, HirType::F64)
            if matches!(yes.as_ref(), HirExpr::Lit(HirLit::F64(-1.0)))
                && matches!(test.as_ref(), HirExpr::OptionalIsNone(..))
    )), "regex split must use the same undefined limit default");
    assert!(function(&program, "regex").body.iter().any(|statement| match statement {
        HirStmt::Return(Some(expr)) | HirStmt::Expr(expr) =>
            has_call(expr, "__thaw_regex_split") && has_call(expr, "__thaw_array_is_null"),
        _ => false,
    }), "regex split must retain its native result and invalid-pattern error check");
    assert!(conditionals(function(&program, "optional")).iter().any(|node| matches!(node,
        HirExpr::Conditional(test, yes, no, HirType::Array(item))
            if **item == HirType::Str
                && matches!(test.as_ref(), HirExpr::OptionalIsNone(..))
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
