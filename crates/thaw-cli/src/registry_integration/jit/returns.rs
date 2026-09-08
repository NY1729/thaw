macro_rules! jit_returns {
    () => {
    fn encode_return_expression(
        expression: &Expr,
        ty: &thaw_hir::HirType,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
    ) -> Option<JitExport> {
        if let Expr::Paren(parenthesized) = expression {
            return encode_return_expression(
                parenthesized.expr.as_ref(),
                ty,
                parameters,
                locals,
                context,
            );
        }
        if let thaw_hir::HirType::Optional(payload)
        | thaw_hir::HirType::Nullable(payload)
        | thaw_hir::HirType::Nullish(payload) = ty
        {
            if matches!(expression, Expr::Lit(Lit::Null(_))) {
                return matches!(ty, thaw_hir::HirType::Nullable(_) | thaw_hir::HirType::Nullish(_))
                    .then_some(JitExport::Null);
            }
            if matches!(expression, Expr::Ident(identifier) if identifier.sym == "undefined") {
                return matches!(ty, thaw_hir::HirType::Optional(_) | thaw_hir::HirType::Nullish(_))
                    .then_some(JitExport::Undefined);
            }
            if let Expr::Cond(conditional) = expression {
                let mut condition = Vec::new();
                encode_condition(
                    conditional.test.as_ref(),
                    parameters,
                    locals,
                    context,
                    &mut condition,
                )?;
                return Some(JitExport::Conditional(
                    validated_jit_expression(condition, JitKind::Boolean)?,
                    Box::new(encode_return_expression(
                        conditional.cons.as_ref(),
                        ty,
                        parameters,
                        locals,
                        context,
                    )?),
                    Box::new(encode_return_expression(
                        conditional.alt.as_ref(),
                        ty,
                        parameters,
                        locals,
                        context,
                    )?),
                ));
            }
            if matches!(payload.as_ref(), thaw_hir::HirType::Object(_) | thaw_hir::HirType::Tuple(_)) {
                return encode_return_expression(expression, payload, parameters, locals, context);
            }
        }
        if matches!(ty, thaw_hir::HirType::Union(elements) if elements.iter().any(|element| matches!(element, thaw_hir::HirType::Object(_) | thaw_hir::HirType::Tuple(_))))
        {
            if let Expr::Cond(conditional) = expression {
                let mut output = Vec::new();
                encode_condition(
                    conditional.test.as_ref(),
                    parameters,
                    locals,
                    context,
                    &mut output,
                )?;
                let JitExport::Value(consequent) = encode_return_expression(
                    conditional.cons.as_ref(),
                    ty,
                    parameters,
                    locals,
                    context,
                )?
                else {
                    return None;
                };
                let JitExport::Value(alternate) = encode_return_expression(
                    conditional.alt.as_ref(),
                    ty,
                    parameters,
                    locals,
                    context,
                )?
                else {
                    return None;
                };
                output.push("if".into());
                output.extend(consequent.strip_prefix("expr:")?.split(',').map(str::to_owned));
                output.push("else".into());
                output.extend(alternate.strip_prefix("expr:")?.split(',').map(str::to_owned));
                output.push("end".into());
                return Some(JitExport::Value(validated_jit_expression(
                    output,
                    JitKind::Dynamic,
                )?));
            }
        }
        if matches!(
            ty,
            thaw_hir::HirType::Object(_)
                | thaw_hir::HirType::Dictionary(_)
                | thaw_hir::HirType::Tuple(_)
        ) {
            let Expr::Cond(conditional) = expression else {
                return encode_nonconditional_return_expression(
                    expression,
                    ty,
                    parameters,
                    locals,
                    context,
                );
            };
            let mut condition = Vec::new();
            encode_condition(
                conditional.test.as_ref(),
                parameters,
                locals,
                context,
                &mut condition,
            )?;
            let consequent = encode_return_expression(
                conditional.cons.as_ref(),
                ty,
                parameters,
                locals,
                context,
            )?;
            let alternate = encode_return_expression(
                conditional.alt.as_ref(),
                ty,
                parameters,
                locals,
                context,
            )?;
            return Some(JitExport::Conditional(
                validated_jit_expression(condition, JitKind::Boolean)?,
                Box::new(consequent),
                Box::new(alternate),
            ));
        }
        encode_nonconditional_return_expression(expression, ty, parameters, locals, context)
    }


    fn encode_nonconditional_return_expression(
        expression: &Expr,
        ty: &thaw_hir::HirType,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
    ) -> Option<JitExport> {
        match ty {
            thaw_hir::HirType::Union(elements)
                if matches!(expression, Expr::Array(_))
                    && elements
                        .iter()
                        .any(|element| matches!(element, thaw_hir::HirType::Tuple(_))) =>
            {
                let types = elements.iter().find_map(|element| match element {
                    thaw_hir::HirType::Tuple(types) => Some(types.as_slice()),
                    _ => None,
                })?;
                let mut output = encode_fixed_tuple_value(
                    expression,
                    types,
                    parameters,
                    locals,
                    context,
                )?;
                output.push("tagtuple".into());
                Some(JitExport::Value(validated_jit_expression(
                    output,
                    JitKind::Dynamic,
                )?))
            }
            thaw_hir::HirType::Union(elements)
                if object_literal(expression).is_some()
                    && elements
                        .iter()
                        .any(|element| matches!(element, thaw_hir::HirType::Object(_))) =>
            {
                let fields = elements.iter().find_map(|element| match element {
                    thaw_hir::HirType::Object(fields) => Some(fields.as_slice()),
                    _ => None,
                })?;
                encode_fixed_object_union_return(
                    object_literal(expression)?,
                    fields,
                    parameters,
                    locals,
                    context,
                )
            }
            thaw_hir::HirType::Object(fields) => encode_object_return(
                object_literal(expression)?,
                fields,
                parameters,
                locals,
                context,
            ),
            thaw_hir::HirType::Dictionary(element) => {
                if let Some(object) = object_literal(expression) {
                    encode_dictionary_return(object, element, parameters, locals, context)
                } else {
                    let mut encoded = Vec::new();
                    encode_expression(expression, parameters, locals, context, &mut encoded)?;
                    Some(JitExport::Value(validated_jit_expression(
                        encoded,
                        JitKind::Dictionary,
                    )?))
                }
            }
            thaw_hir::HirType::Tuple(types) => {
                let expression = match expression {
                    Expr::Array(array) => array,
                    _ => return None,
                };
                if expression.elems.len() != types.len() {
                    return None;
                }
                expression
                    .elems
                    .iter()
                    .zip(types)
                    .map(|(element, ty)| {
                        let element = element.as_ref()?;
                        if element.spread.is_some() {
                            return None;
                        }
                        encode_return_expression(
                            element.expr.as_ref(),
                            ty,
                            parameters,
                            locals,
                            context,
                        )
                    })
                    .collect::<Option<Vec<_>>>()
                    .map(JitExport::Tuple)
            }
            _ => {
                let mut encoded = Vec::new();
                encode_expression(expression, parameters, locals, context, &mut encoded)?;
                if matches!(ty, thaw_hir::HirType::Union(elements) if elements.iter().any(|element| matches!(element, thaw_hir::HirType::Object(_) | thaw_hir::HirType::Tuple(_))))
                    && encoded
                        .iter()
                        .any(|token| matches!(token.as_str(), "tagdn" | "tagdb" | "tagds"))
                {
                    return None;
                }
                Some(JitExport::Value(validated_jit_expression(
                    encoded,
                    jit_return_kind(ty)?,
                )?))
            }
        }
    }


    fn math_method(
        call: &CallExpr,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
    ) -> Option<&'static str> {
        if parameters.contains_key("Math") || locals.contains_key("Math") {
            return None;
        }
        let Callee::Expr(callee) = &call.callee else {
            return None;
        };
        let Expr::Member(member) = callee.as_ref() else {
            return None;
        };
        if !matches!(member.obj.as_ref(), Expr::Ident(object) if object.sym == "Math") {
            return None;
        }
        let MemberProp::Ident(property) = &member.prop else {
            return None;
        };
        if call.args.iter().any(|argument| argument.spread.is_some()) {
            match property.sym.as_ref() {
                "min" | "max" | "hypot" => {}
                _ => return None,
            }
        }
        match property.sym.as_ref() {
            "acos" => Some("acos"),
            "acosh" => Some("acosh"),
            "abs" => Some("abs"),
            "asin" => Some("asin"),
            "asinh" => Some("asinh"),
            "atan" => Some("atan"),
            "atan2" => Some("atan2"),
            "atanh" => Some("atanh"),
            "cbrt" => Some("cbrt"),
            "ceil" => Some("ceil"),
            "clz32" => Some("clz32"),
            "cos" => Some("cos"),
            "cosh" => Some("cosh"),
            "exp" => Some("exp"),
            "expm1" => Some("expm1"),
            "floor" => Some("floor"),
            "fround" => Some("fround"),
            "hypot" => Some("hypot"),
            "imul" => Some("imul"),
            "log" => Some("log"),
            "log1p" => Some("log1p"),
            "log2" => Some("log2"),
            "log10" => Some("log10"),
            "min" => Some("min"),
            "max" => Some("max"),
            "pow" => Some("pow"),
            "round" => Some("round"),
            "random" => Some("random"),
            "sign" => Some("sign"),
            "sin" => Some("sin"),
            "sinh" => Some("sinh"),
            "sqrt" => Some("sqrt"),
            "tan" => Some("tan"),
            "tanh" => Some("tanh"),
            "trunc" => Some("trunc"),
            _ => None,
        }
    }

    fn time_method(
        call: &CallExpr,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
    ) -> Option<&'static str> {
        if !call.args.is_empty() {
            return None;
        }
        let Callee::Expr(callee) = &call.callee else {
            return None;
        };
        let Expr::Member(member) = callee.as_ref() else {
            return None;
        };
        let Expr::Ident(object) = member.obj.as_ref() else {
            return None;
        };
        if parameters.contains_key(object.sym.as_ref()) || locals.contains_key(object.sym.as_ref()) {
            return None;
        }
        let MemberProp::Ident(property) = &member.prop else {
            return None;
        };
        match (object.sym.as_ref(), property.sym.as_ref()) {
            ("Date", "now") => Some("datenow"),
            ("performance", "now") => Some("performancenow"),
            ("process", "uptime") => Some("processuptime"),
            _ => None,
        }
    }

    fn number_predicate(
        call: &CallExpr,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        helpers: &std::collections::HashMap<String, NumericCallable<'_>>,
    ) -> Option<(&'static str, bool)> {
        if call.args.iter().any(|argument| argument.spread.is_some()) {
            return None;
        }
        let Callee::Expr(callee) = &call.callee else {
            return None;
        };
        match callee.as_ref() {
            Expr::Ident(identifier)
                if !parameters.contains_key(identifier.sym.as_ref())
                    && !locals.contains_key(identifier.sym.as_ref())
                    && !helpers.contains_key(identifier.sym.as_ref()) =>
            {
                match identifier.sym.as_ref() {
                    "isNaN" => Some(("isnan", true)),
                    "isFinite" => Some(("isfinite", true)),
                    _ => None,
                }
            }
            Expr::Member(member)
                if !parameters.contains_key("Number") && !locals.contains_key("Number") =>
            {
                let Expr::Ident(receiver) = member.obj.as_ref() else {
                    return None;
                };
                if receiver.sym != "Number" {
                    return None;
                }
                let MemberProp::Ident(property) = &member.prop else {
                    return None;
                };
                match property.sym.as_ref() {
                    "isNaN" => Some(("isnan", false)),
                    "isFinite" => Some(("isfinite", false)),
                    "isInteger" => Some(("isinteger", false)),
                    "isSafeInteger" => Some(("issafeinteger", false)),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn number_parser(
        call: &CallExpr,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        helpers: &std::collections::HashMap<String, NumericCallable<'_>>,
    ) -> Option<&'static str> {
        if call.args.iter().any(|argument| argument.spread.is_some()) {
            return None;
        }
        let Callee::Expr(callee) = &call.callee else {
            return None;
        };
        let name = match callee.as_ref() {
            Expr::Ident(identifier)
                if !parameters.contains_key(identifier.sym.as_ref())
                    && !locals.contains_key(identifier.sym.as_ref())
                    && !helpers.contains_key(identifier.sym.as_ref()) =>
            {
                identifier.sym.as_ref()
            }
            Expr::Member(member)
                if !parameters.contains_key("Number") && !locals.contains_key("Number") =>
            {
                let Expr::Ident(receiver) = member.obj.as_ref() else {
                    return None;
                };
                let MemberProp::Ident(property) = &member.prop else {
                    return None;
                };
                (receiver.sym == "Number").then_some(property.sym.as_ref())?
            }
            _ => return None,
        };
        match name {
            "parseFloat" => Some("parsefloat"),
            "parseInt" => Some("parseint"),
            _ => None,
        }
    }

    fn array_predicate<'a>(
        call: &'a CallExpr,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        helpers: &std::collections::HashMap<String, NumericCallable<'_>>,
    ) -> Option<&'a Expr> {
        if parameters.contains_key("Array")
            || locals.contains_key("Array")
            || helpers.contains_key("Array")
        {
            return None;
        }
        let [argument] = call.args.as_slice() else {
            return None;
        };
        if argument.spread.is_some() {
            return None;
        }
        let Callee::Expr(callee) = &call.callee else {
            return None;
        };
        let Expr::Member(member) = callee.as_ref() else {
            return None;
        };
        if !matches!(member.obj.as_ref(), Expr::Ident(object) if object.sym == "Array")
            || !matches!(&member.prop, MemberProp::Ident(property) if property.sym == "isArray")
        {
            return None;
        }
        Some(argument.expr.as_ref())
    }

    fn array_constructor<'a>(
        call: &'a CallExpr,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        helpers: &std::collections::HashMap<String, NumericCallable<'_>>,
    ) -> Option<&'a str> {
        if parameters.contains_key("Array")
            || locals.contains_key("Array")
            || helpers.contains_key("Array")
        {
            return None;
        }
        let Callee::Expr(callee) = &call.callee else {
            return None;
        };
        let Expr::Member(member) = callee.as_ref() else {
            return None;
        };
        if !matches!(member.obj.as_ref(), Expr::Ident(object) if object.sym == "Array") {
            return None;
        }
        let MemberProp::Ident(property) = &member.prop else {
            return None;
        };
        matches!(property.sym.as_ref(), "of" | "from").then_some(property.sym.as_ref())
    }

    fn string_static_constructor<'a>(
        call: &'a CallExpr,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        helpers: &std::collections::HashMap<String, NumericCallable<'_>>,
    ) -> Option<&'a str> {
        if parameters.contains_key("String")
            || locals.contains_key("String")
            || helpers.contains_key("String")
            || call.args.iter().any(|argument| argument.spread.is_some())
        {
            return None;
        }
        let Callee::Expr(callee) = &call.callee else {
            return None;
        };
        let Expr::Member(member) = callee.as_ref() else {
            return None;
        };
        if !matches!(member.obj.as_ref(), Expr::Ident(object) if object.sym == "String") {
            return None;
        }
        let MemberProp::Ident(property) = &member.prop else {
            return None;
        };
        matches!(property.sym.as_ref(), "fromCharCode" | "fromCodePoint")
            .then_some(property.sym.as_ref())
    }

    fn object_same_value<'a>(
        call: &'a CallExpr,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        helpers: &std::collections::HashMap<String, NumericCallable<'_>>,
    ) -> Option<(&'a Expr, &'a Expr)> {
        if parameters.contains_key("Object")
            || locals.contains_key("Object")
            || helpers.contains_key("Object")
        {
            return None;
        }
        let [left, right] = call.args.as_slice() else {
            return None;
        };
        if left.spread.is_some() || right.spread.is_some() {
            return None;
        }
        let Callee::Expr(callee) = &call.callee else {
            return None;
        };
        let Expr::Member(member) = callee.as_ref() else {
            return None;
        };
        if !matches!(member.obj.as_ref(), Expr::Ident(object) if object.sym == "Object")
            || !matches!(&member.prop, MemberProp::Ident(property) if property.sym == "is")
        {
            return None;
        }
        Some((left.expr.as_ref(), right.expr.as_ref()))
    }

    fn object_dictionary_call<'a>(
        call: &'a CallExpr,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        helpers: &std::collections::HashMap<String, NumericCallable<'_>>,
    ) -> Option<(&'static str, &'a Expr, Option<&'a Expr>)> {
        if call.args.iter().any(|argument| argument.spread.is_some()) {
            return None;
        }
        let Callee::Expr(callee) = &call.callee else {
            return None;
        };
        let Expr::Member(member) = callee.as_ref() else {
            return None;
        };
        let Expr::Ident(namespace) = member.obj.as_ref() else {
            return None;
        };
        let namespace = namespace.sym.as_ref();
        if parameters.contains_key(namespace)
            || locals.contains_key(namespace)
            || helpers.contains_key(namespace)
        {
            return None;
        }
        let MemberProp::Ident(property) = &member.prop else {
            return None;
        };
        match (namespace, property.sym.as_ref(), call.args.as_slice()) {
            ("Object", "keys" | "getOwnPropertyNames", [object])
            | ("Reflect", "ownKeys", [object]) => {
                Some(("dkeys", object.expr.as_ref(), None))
            }
            ("Object", "values", [object]) => Some(("dvalues", object.expr.as_ref(), None)),
            ("Object", "entries", [object]) => Some(("dentries", object.expr.as_ref(), None)),
            ("Object", "hasOwn", [object, key]) => Some((
                "dhasown",
                object.expr.as_ref(),
                Some(key.expr.as_ref()),
            )),
            _ => None,
        }
    }

    fn object_from_entries_call<'a>(
        call: &'a CallExpr,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        helpers: &std::collections::HashMap<String, NumericCallable<'_>>,
    ) -> Option<&'a Expr> {
        if parameters.contains_key("Object")
            || locals.contains_key("Object")
            || helpers.contains_key("Object")
            || call.args.iter().any(|argument| argument.spread.is_some())
        {
            return None;
        }
        let Callee::Expr(callee) = &call.callee else {
            return None;
        };
        let Expr::Member(member) = callee.as_ref() else {
            return None;
        };
        match (member.obj.as_ref(), &member.prop, call.args.as_slice()) {
            (
                Expr::Ident(object),
                MemberProp::Ident(property),
                [entries],
            ) if object.sym == "Object" && property.sym == "fromEntries" => {
                Some(entries.expr.as_ref())
            }
            _ => None,
        }
    }

    fn object_assign_call<'a>(
        call: &'a CallExpr,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        helpers: &std::collections::HashMap<String, NumericCallable<'_>>,
    ) -> Option<&'a [ExprOrSpread]> {
        if parameters.contains_key("Object")
            || locals.contains_key("Object")
            || helpers.contains_key("Object")
            || call.args.is_empty()
            || call.args.iter().any(|argument| argument.spread.is_some())
        {
            return None;
        }
        let Callee::Expr(callee) = &call.callee else {
            return None;
        };
        let Expr::Member(member) = callee.as_ref() else {
            return None;
        };
        matches!(
            (member.obj.as_ref(), &member.prop),
            (Expr::Ident(object), MemberProp::Ident(property))
                if object.sym == "Object" && property.sym == "assign"
        )
        .then_some(call.args.as_slice())
    }

    fn numeric_constant(
        expression: &Expr,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        helpers: &std::collections::HashMap<String, NumericCallable<'_>>,
    ) -> Option<f64> {
        if let Expr::Ident(identifier) = expression {
            let name = identifier.sym.as_ref();
            if parameters.contains_key(name)
                || locals.contains_key(name)
                || helpers.contains_key(name)
            {
                return None;
            }
            return match name {
                "NaN" => Some(f64::NAN),
                "Infinity" => Some(f64::INFINITY),
                _ => None,
            };
        }
        let Expr::Member(member) = expression else {
            return None;
        };
        let Expr::Ident(object) = member.obj.as_ref() else {
            return None;
        };
        let object = object.sym.as_ref();
        if parameters.contains_key(object)
            || locals.contains_key(object)
            || helpers.contains_key(object)
        {
            return None;
        }
        let MemberProp::Ident(property) = &member.prop else {
            return None;
        };
        match (object, property.sym.as_ref()) {
            ("Math", "E") => Some(std::f64::consts::E),
            ("Math", "LN2") => Some(std::f64::consts::LN_2),
            ("Math", "LN10") => Some(std::f64::consts::LN_10),
            ("Math", "LOG2E") => Some(std::f64::consts::LOG2_E),
            ("Math", "LOG10E") => Some(std::f64::consts::LOG10_E),
            ("Math", "PI") => Some(std::f64::consts::PI),
            ("Math", "SQRT1_2") => Some(std::f64::consts::FRAC_1_SQRT_2),
            ("Math", "SQRT2") => Some(std::f64::consts::SQRT_2),
            ("Number", "EPSILON") => Some(f64::EPSILON),
            ("Number", "MAX_SAFE_INTEGER") => Some(9_007_199_254_740_991.0),
            ("Number", "MIN_SAFE_INTEGER") => Some(-9_007_199_254_740_991.0),
            ("Number", "MAX_VALUE") => Some(f64::MAX),
            ("Number", "MIN_VALUE") => Some(f64::from_bits(1)),
            ("Number", "NaN") => Some(f64::NAN),
            ("Number", "POSITIVE_INFINITY") => Some(f64::INFINITY),
            ("Number", "NEGATIVE_INFINITY") => Some(f64::NEG_INFINITY),
            _ => None,
        }
    }

    fn string_parameter<'a>(
        expression: &Expr,
        parameters: &'a std::collections::HashMap<String, String>,
    ) -> Option<&'a str> {
        let Expr::Ident(identifier) = expression else {
            return None;
        };
        parameters
            .get(identifier.sym.as_ref())
            .filter(|token| token.starts_with('s'))
            .map(String::as_str)
    }

    fn is_string_expression(
        expression: &Expr,
        parameters: &std::collections::HashMap<String, String>,
    ) -> bool {
        match expression {
            Expr::Ident(_) => string_parameter(expression, parameters).is_some(),
            Expr::Member(_) => member_path(expression)
                .and_then(|path| parameters.get(&path))
                .is_some_and(|token| token.starts_with('s')),
            Expr::Lit(Lit::Str(_)) => true,
            Expr::Tpl(template) => template.quasis.len() == template.exprs.len() + 1,
            Expr::Paren(parenthesized) => {
                is_string_expression(parenthesized.expr.as_ref(), parameters)
            }
            Expr::Bin(binary) if binary.op == BinaryOp::Add => true,
            Expr::Call(call) => {
                let Callee::Expr(callee) = &call.callee else {
                    return false;
                };
                if !parameters.contains_key("String")
                    && matches!(callee.as_ref(), Expr::Ident(identifier) if identifier.sym == "String")
                {
                    let [argument] = call.args.as_slice() else {
                        return false;
                    };
                    return argument.spread.is_none();
                }
                if !parameters.contains_key("String")
                    && matches!(callee.as_ref(), Expr::Member(member)
                        if matches!(member.obj.as_ref(), Expr::Ident(object) if object.sym == "String")
                            && matches!(&member.prop, MemberProp::Ident(property)
                                if matches!(property.sym.as_ref(), "fromCharCode" | "fromCodePoint")))
                {
                    return call.args.iter().all(|argument| argument.spread.is_none());
                }
                let Expr::Member(member) = callee.as_ref() else {
                    return false;
                };
                let MemberProp::Ident(property) = &member.prop else {
                    return false;
                };
                let arity_matches = match property.sym.as_ref() {
                    "toLowerCase"
                    | "toUpperCase"
                    | "toWellFormed"
                    | "trim"
                    | "trimStart"
                    | "trimEnd" => {
                        call.args.is_empty()
                    }
                    "normalize" => call.args.len() <= 1,
                    "charAt" => call.args.len() <= 1,
                    "at" => call.args.len() <= 1,
                    "concat" => call.args.iter().all(|argument| argument.spread.is_none()),
                    "repeat" => call.args.len() == 1,
                    "replace" | "replaceAll" => call.args.len() == 2,
                    "padStart" | "padEnd" => (1..=2).contains(&call.args.len()),
                    "slice" | "substring" => call.args.len() <= 2,
                    "split" => (1..=2).contains(&call.args.len()),
                    _ => false,
                };
                arity_matches && is_string_expression(member.obj.as_ref(), parameters)
            }
            _ => false,
        }
    }

    fn string_method<'a>(
        call: &'a CallExpr,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
    ) -> Option<(&'static str, &'a Expr)> {
        if call.args.iter().any(|argument| argument.spread.is_some()) {
            return None;
        }
        let Callee::Expr(callee) = &call.callee else {
            return None;
        };
        let Expr::Member(member) = callee.as_ref() else {
            return None;
        };
        let local_string = member_path(member.obj.as_ref())
            .and_then(|path| locals.get(&path))
            .and_then(|expression| jit_expression_kind(expression))
            .is_some_and(|(kind, _)| matches!(kind, JitKind::String | JitKind::Dynamic));
        let dynamic_parameter = matches!(member.obj.as_ref(), Expr::Ident(identifier) if parameters
            .get(identifier.sym.as_ref())
            .is_some_and(|token| jit_dynamic_argument(token).is_some()));
        if !local_string
            && !dynamic_parameter
            && !is_string_expression(member.obj.as_ref(), parameters)
        {
            return None;
        }
        let MemberProp::Ident(property) = &member.prop else {
            return None;
        };
        let operation = match property.sym.as_ref() {
            "startsWith" => "startswith",
            "endsWith" => "endswith",
            "includes" => "includes",
            "indexOf" => "indexof",
            "lastIndexOf" => "lastindexof",
            "toLowerCase" => "tolowercase",
            "toUpperCase" => "touppercase",
            "trim" => "trim",
            "trimStart" => "trimstart",
            "trimEnd" => "trimend",
            "concat" => "concat",
            "repeat" => "repeat",
            "normalize" => "normalize",
            "replace" => "replace",
            "replaceAll" => "replaceall",
            "charAt" => "charat",
            "charCodeAt" => "charcodeat",
            "localeCompare" => "strcmp",
            "isWellFormed" => "iswellformed",
            "toWellFormed" => "towellformed",
            "at" => "at",
            "codePointAt" => "codepointat",
            "padStart" => "padstart",
            "padEnd" => "padend",
            "slice" => "slice",
            "substring" => "substring",
            "split" => "split",
            _ => return None,
        };
        Some((operation, member.obj.as_ref()))
    }

    fn number_format_method<'a>(
        call: &'a CallExpr,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
    ) -> Option<(&'static str, &'a Expr)> {
        if call.args.iter().any(|argument| argument.spread.is_some()) {
            return None;
        }
        let Callee::Expr(callee) = &call.callee else {
            return None;
        };
        let Expr::Member(member) = callee.as_ref() else {
            return None;
        };
        let MemberProp::Ident(property) = &member.prop else {
            return None;
        };
        if property.sym == "toString"
            && matches!(member.obj.as_ref(), Expr::Ident(receiver) if parameters
                .get(receiver.sym.as_ref())
                .is_some_and(|token| token.starts_with('r'))
                || locals
                    .get(receiver.sym.as_ref())
                    .and_then(|tokens| jit_expression_kind(tokens))
                    .is_some_and(|(kind, _)| kind == JitKind::Array))
        {
            return None;
        }
        let operation = match property.sym.as_ref() {
            "toFixed" => "tofixed",
            "toPrecision" => "toprecision",
            "toString" => "toradix",
            "toExponential" => "toexponential",
            _ => return None,
        };
        Some((operation, member.obj.as_ref()))
    }

    fn primitive_value_of(call: &CallExpr) -> Option<&Expr> {
        if !call.args.is_empty() {
            return None;
        }
        let Callee::Expr(callee) = &call.callee else {
            return None;
        };
        let Expr::Member(member) = callee.as_ref() else {
            return None;
        };
        matches!(&member.prop, MemberProp::Ident(property) if property.sym == "valueOf")
            .then_some(member.obj.as_ref())
    }

    fn array_method<'a>(
        call: &'a CallExpr,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        helpers: &std::collections::HashMap<String, NumericCallable<'_>>,
    ) -> Option<(&'a str, &'a Expr)> {
        if call.args.iter().any(|argument| argument.spread.is_some()) {
            return None;
        }
        let Callee::Expr(callee) = &call.callee else {
            return None;
        };
        let Expr::Member(member) = callee.as_ref() else {
            return None;
        };
        let MemberProp::Ident(property) = &member.prop else {
            return None;
        };
        let parameter_array = match member.obj.as_ref() {
            Expr::Ident(receiver) => parameters.get(receiver.sym.as_ref()).is_some_and(|token| {
                token.starts_with('r')
                    || jit_dynamic_array_argument(token)
                    || jit_typed_array_union_untag(token).is_some()
            }),
            _ => false,
        };
        let local_array = member_path(member.obj.as_ref())
            .and_then(|path| locals.get(&path))
            .is_some_and(|tokens| {
                jit_expression_kind(tokens).is_some_and(|(kind, _)| kind == JitKind::Array)
                    || tokens
                        .first()
                        .and_then(|token| runtime_local_kind(token))
                        .is_some_and(|kind| kind == JitKind::Array)
                    || matches!(tokens.as_slice(), [token] if jit_dynamic_array_argument(token)
                        || jit_typed_array_union_untag(token).is_some())
            });
        let returned_array = match member.obj.as_ref() {
            Expr::Call(receiver) => array_method(receiver, parameters, locals, helpers)
                .is_some_and(|(method, _)| {
                    matches!(
                        method,
                        "slice"
                            | "concat"
                            | "toReversed"
                            | "toSorted"
                            | "reverse"
                            | "sort"
                            | "fill"
                            | "copyWithin"
                            | "splice"
                            | "toSpliced"
                            | "with"
                            | "filter"
                            | "map"
                    )
                }),
            _ => false,
        };
        let constructed_array = match member.obj.as_ref() {
            Expr::Call(receiver) => array_constructor(receiver, parameters, locals, helpers)
                .is_some_and(|constructor| constructor == "of" || constructor == "from"),
            _ => false,
        };
        let dictionary_array = match member.obj.as_ref() {
            Expr::Call(receiver) => object_dictionary_call(receiver, parameters, locals, helpers)
                .is_some_and(|(operation, _, _)| {
                    matches!(operation, "dkeys" | "dvalues" | "dentries")
                }),
            _ => false,
        };
        let split_array = match member.obj.as_ref() {
            Expr::Call(receiver) => string_method(receiver, parameters, locals)
                .is_some_and(|(operation, _)| operation == "split"),
            _ => false,
        };
        let literal_array = matches!(member.obj.as_ref(), Expr::Array(_));
        // A ternary's/short-circuit's own array-ness (both branches agreeing on
        // element type) is only knowable by actually encoding it, which needs
        // `context` and happens anyway right after this gate returns; the
        // downstream `jit_expression_kind` check rejects it there if either side
        // isn't array-shaped. Chaining a method onto any of these requires
        // parenthesizing it (`(a ? b : c).map(...)`, `(a || b).map(...)`), so the
        // receiver is `Expr::Paren` wrapping the branching expression, not that
        // expression itself.
        let conditional_array = match match member.obj.as_ref() {
            Expr::Paren(parenthesized) => parenthesized.expr.as_ref(),
            receiver => receiver,
        } {
            Expr::Cond(_) => true,
            Expr::Bin(binary) => matches!(
                binary.op,
                BinaryOp::LogicalAnd | BinaryOp::LogicalOr | BinaryOp::NullishCoalescing
            ),
            _ => false,
        };
        if !parameter_array
            && !local_array
            && !returned_array
            && !constructed_array
            && !dictionary_array
            && !split_array
            && !literal_array
            && !conditional_array
        {
            return None;
        }
        matches!(
            property.sym.as_ref(),
            "at"
                | "includes"
                | "indexOf"
                | "lastIndexOf"
                | "join"
                | "toString"
                | "slice"
                | "concat"
                | "toReversed"
                | "toSorted"
                | "reverse"
                | "sort"
                | "fill"
                | "copyWithin"
                | "push"
                | "unshift"
                | "pop"
                | "shift"
                | "splice"
                | "toSpliced"
                | "with"
                | "reduce"
                | "reduceRight"
                | "some"
                | "every"
                | "find"
                | "findIndex"
                | "findLast"
                | "findLastIndex"
                | "filter"
                | "map"
        )
            .then_some((property.sym.as_ref(), member.obj.as_ref()))
    }

    fn entry_prefix(expression: &[String]) -> Option<&'static str> {
        expression.iter().find_map(|token| match token.as_str() {
            token if token.starts_with("en") || token == "dnentries" => Some("dn"),
            token if token.starts_with("eb") || token == "dbentries" => Some("db"),
            token if token.starts_with("es") || token == "dsentries" => Some("ds"),
            _ => None,
        })
    }

    fn append_add(
        mut left: Vec<String>,
        mut right: Vec<String>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        let left_kind = jit_expression_kind(&left)?.0;
        let right_kind = jit_expression_kind(&right)?.0;
        if left_kind == JitKind::Dynamic || right_kind == JitKind::Dynamic {
            append_dynamic(left, output)?;
            append_dynamic(right, output)?;
            output.push("dynadd".into());
            return Some(());
        }
        if matches!(left_kind, JitKind::Array | JitKind::Dictionary)
            || matches!(right_kind, JitKind::Array | JitKind::Dictionary)
        {
            return None;
        }
        output.append(&mut left);
        if left_kind != JitKind::String && right_kind == JitKind::String {
            output.push(
                if left_kind == JitKind::Boolean {
                    "boolstr"
                } else {
                    "numstr"
                }
                .into(),
            );
        }
        output.append(&mut right);
        if right_kind != JitKind::String && left_kind == JitKind::String {
            output.push(
                if right_kind == JitKind::Boolean {
                    "boolstr"
                } else {
                    "numstr"
                }
                .into(),
            );
        }
        output.push(
            if left_kind == JitKind::String || right_kind == JitKind::String {
                "concat"
            } else {
                "+"
            }
            .into(),
        );
        Some(())
    }

    fn append_dynamic(mut expression: Vec<String>, output: &mut Vec<String>) -> Option<()> {
        let kind = jit_expression_kind(&expression)?.0;
        output.append(&mut expression);
        match kind {
            JitKind::Number => output.push("tagnum".into()),
            JitKind::Boolean => output.push("tagbool".into()),
            JitKind::String => output.push("tagstr".into()),
            JitKind::Dynamic => {}
            JitKind::Array | JitKind::Dictionary => return None,
        }
        Some(())
    }

    fn append_string(mut expression: Vec<String>, output: &mut Vec<String>) -> Option<()> {
        let kind = jit_expression_kind(&expression)?.0;
        output.append(&mut expression);
        match kind {
            JitKind::Number => output.push("numstr".into()),
            JitKind::Boolean => output.push("boolstr".into()),
            JitKind::String => {}
            JitKind::Dynamic => output.push("dynstr".into()),
            JitKind::Array | JitKind::Dictionary => return None,
        }
        Some(())
    }

    fn append_number(mut expression: Vec<String>, output: &mut Vec<String>) -> Option<()> {
        let kind = jit_expression_kind(&expression)?.0;
        output.append(&mut expression);
        match kind {
            JitKind::String => output.push("strnum".into()),
            JitKind::Dynamic => output.push("dynnum".into()),
            JitKind::Array | JitKind::Dictionary => return None,
            JitKind::Number | JitKind::Boolean => {}
        }
        Some(())
    }

    fn encode_number(
        expression: &Expr,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        let mut encoded = Vec::new();
        encode_expression(expression, parameters, locals, context, &mut encoded)?;
        append_number(encoded, output)
    }

    fn append_boolean(mut expression: Vec<String>, output: &mut Vec<String>) -> Option<()> {
        let kind = jit_expression_kind(&expression)?.0;
        output.append(&mut expression);
        match kind {
            JitKind::Number => output.push("asbool".into()),
            JitKind::String => output.push("strbool".into()),
            JitKind::Boolean => {}
            JitKind::Dynamic => output.push("dynbool".into()),
            JitKind::Array | JitKind::Dictionary => return None,
        }
        Some(())
    }

    fn append_array_element(
        element: &ExprOrSpread,
        parameters: &std::collections::HashMap<String, String>,
        locals: &std::collections::HashMap<String, Vec<String>>,
        context: &mut InlineContext<'_>,
        prefix: &mut Option<&'static str>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        let mut encoded = Vec::new();
        encode_expression(
            element.expr.as_ref(),
            parameters,
            locals,
            context,
            &mut encoded,
        )?;
        if element.spread.is_some() && encoded.len() == 1 {
            if let Some(untag) = jit_typed_array_union_untag(&encoded[0]) {
                encoded.push(untag.into());
            }
        }
        if element.spread.is_none() && matches!(element.expr.as_ref(), Expr::Lit(Lit::Bool(_))) {
            encoded.push("asbool".into());
        }
        let (element_prefix, operation) = if element.spread.is_some() {
            match jit_expression_kind(&encoded)?.0 {
                JitKind::Array => (array_prefix(&encoded)?, "arrayconcat"),
                JitKind::String => {
                    encoded.push("strarray".into());
                    ("rs", "arrayconcat")
                }
                JitKind::Number | JitKind::Boolean | JitKind::Dynamic | JitKind::Dictionary => return None,
            }
        } else {
            let element_prefix = match jit_expression_kind(&encoded)?.0 {
                JitKind::Number => "rn",
                JitKind::String => "rs",
                JitKind::Boolean => "rb",
                JitKind::Dynamic | JitKind::Array | JitKind::Dictionary => return None,
            };
            let operation = match element_prefix {
                "rn" => "rnappend",
                "rs" => "rsappend",
                "rb" => "rbappend",
                _ => unreachable!(),
            };
            (element_prefix, operation)
        };
        if prefix.is_some_and(|prefix| prefix != element_prefix) {
            return None;
        }
        *prefix = Some(element_prefix);
        output.extend(encoded);
        output.push(operation.into());
        Some(())
    }
    };
}
