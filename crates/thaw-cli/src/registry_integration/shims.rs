/// One `--use`d package, resolved and classified, but with no shim text
/// generated yet -- collision resolution (see `generate_registry_shims`)
/// needs to see every package's declared names *before* deciding how any
/// individual one should be rendered, so this is deliberately kept as an
/// intermediate step rather than folded into one pass.
struct ResolvedPackage {
    name: String,
    commonjs_export_name: Option<String>,
    functions: Vec<thaw_bridge::DtsFunction>,
    classes: Vec<thaw_bridge::DtsClass>,
    classifications: Vec<(String, thaw_bridge::Classification)>,
    native_lib: Option<PathBuf>,
    native_addon: Option<PathBuf>,
    bundle_js: Option<String>,
}

fn jit_numeric_export(
    source: &str,
    export_name: &str,
    allow_default: bool,
    function: &thaw_bridge::DtsFunction,
) -> Option<String> {
    use thaw_parser::ast::{
        AssignOp, AssignTarget, BinaryOp, Decl, Expr, Ident, Lit, ModuleItem, Pat, Prop, PropName,
        PropOrSpread, SimpleAssignTarget, Stmt, UnaryOp,
    };

    fn encode_expression(
        expression: &Expr,
        left: &str,
        right: &str,
        locals: &std::collections::HashMap<String, Vec<String>>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        match expression {
            Expr::Ident(identifier) if locals.contains_key(identifier.sym.as_ref()) => {
                output.extend(locals.get(identifier.sym.as_ref())?.iter().cloned());
            }
            Expr::Ident(identifier) if identifier.sym == left => output.push("x".into()),
            Expr::Ident(identifier) if identifier.sym == right => output.push("y".into()),
            Expr::Lit(Lit::Num(number)) => {
                output.push(format!("c{:016x}", number.value.to_bits()));
            }
            Expr::Unary(unary)
                if matches!(unary.op, UnaryOp::Plus | UnaryOp::Minus)
                    && matches!(unary.arg.as_ref(), Expr::Lit(Lit::Num(_))) =>
            {
                let Expr::Lit(Lit::Num(number)) = unary.arg.as_ref() else {
                    unreachable!()
                };
                let value = if unary.op == UnaryOp::Minus {
                    -number.value
                } else {
                    number.value
                };
                output.push(format!("c{:016x}", value.to_bits()));
            }
            Expr::Paren(parenthesized) => {
                encode_expression(parenthesized.expr.as_ref(), left, right, locals, output)?;
            }
            Expr::Bin(binary)
                if matches!(
                    binary.op,
                    BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div
                ) =>
            {
                encode_expression(binary.left.as_ref(), left, right, locals, output)?;
                encode_expression(binary.right.as_ref(), left, right, locals, output)?;
                output.push(
                    match binary.op {
                        BinaryOp::Add => "+",
                        BinaryOp::Sub => "-",
                        BinaryOp::Mul => "*",
                        BinaryOp::Div => "/",
                        _ => unreachable!(),
                    }
                    .into(),
                );
            }
            Expr::Cond(conditional) => {
                encode_condition(conditional.test.as_ref(), left, right, locals, output)?;
                encode_expression(conditional.cons.as_ref(), left, right, locals, output)?;
                encode_expression(conditional.alt.as_ref(), left, right, locals, output)?;
                output.push("?".into());
            }
            _ => return None,
        }
        (output.len() <= 128).then_some(())
    }

    fn encode_condition(
        expression: &Expr,
        left: &str,
        right: &str,
        locals: &std::collections::HashMap<String, Vec<String>>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        let Expr::Bin(binary) = expression else {
            return None;
        };
        let operator = match binary.op {
            BinaryOp::Lt => "<",
            BinaryOp::LtEq => "<=",
            BinaryOp::Gt => ">",
            BinaryOp::GtEq => ">=",
            BinaryOp::EqEq | BinaryOp::EqEqEq => "==",
            BinaryOp::NotEq | BinaryOp::NotEqEq => "!=",
            _ => return None,
        };
        encode_expression(binary.left.as_ref(), left, right, locals, output)?;
        encode_expression(binary.right.as_ref(), left, right, locals, output)?;
        output.push(operator.into());
        (output.len() <= 128).then_some(())
    }

    enum NumericBody<'a> {
        Expression(&'a Expr),
        Conditional {
            test: &'a Expr,
            consequent: &'a Expr,
            alternate: &'a Expr,
        },
    }

    fn returned_expression(statement: &Stmt) -> Option<&Expr> {
        match statement {
            Stmt::Return(returned) => returned.arg.as_deref(),
            Stmt::Block(block) => {
                let [statement] = block.stmts.as_slice() else {
                    return None;
                };
                returned_expression(statement)
            }
            _ => None,
        }
    }

    fn numeric_body(statements: &[Stmt]) -> Option<NumericBody<'_>> {
        match statements {
            [Stmt::Return(returned)] => {
                Some(NumericBody::Expression(returned.arg.as_deref()?))
            }
            [Stmt::If(branch), Stmt::Return(alternate)] if branch.alt.is_none() => {
                Some(NumericBody::Conditional {
                    test: branch.test.as_ref(),
                    consequent: returned_expression(branch.cons.as_ref())?,
                    alternate: alternate.arg.as_deref()?,
                })
            }
            [Stmt::If(branch)] => Some(NumericBody::Conditional {
                test: branch.test.as_ref(),
                consequent: returned_expression(branch.cons.as_ref())?,
                alternate: returned_expression(branch.alt.as_deref()?)?,
            }),
            _ => None,
        }
    }

    fn split_numeric_body(statements: &[Stmt]) -> Option<(Vec<(&Ident, &Expr)>, NumericBody<'_>)> {
        let mut locals = Vec::new();
        let mut offset = 0;
        while let Some(Stmt::Decl(Decl::Var(declaration))) = statements.get(offset) {
            for declarator in &declaration.decls {
                let Pat::Ident(name) = &declarator.name else {
                    return None;
                };
                locals.push((&name.id, declarator.init.as_deref()?));
            }
            offset += 1;
        }
        Some((locals, numeric_body(&statements[offset..])?))
    }

    fn encode_numeric_body(
        body: NumericBody<'_>,
        left: &str,
        right: &str,
        locals: &std::collections::HashMap<String, Vec<String>>,
        output: &mut Vec<String>,
    ) -> Option<()> {
        match body {
            NumericBody::Expression(body) => {
                encode_expression(body, left, right, locals, output)?;
            }
            NumericBody::Conditional {
                test,
                consequent,
                alternate,
            } => {
                encode_condition(test, left, right, locals, output)?;
                encode_expression(consequent, left, right, locals, output)?;
                encode_expression(alternate, left, right, locals, output)?;
                output.push("?".into());
            }
        }
        Some(())
    }

    if function.generic.is_some()
        || function.required_params != function.params.len()
        || !(1..=2).contains(&function.params.len())
        || function.rest_param.is_some()
        || !function
            .params
            .iter()
            .all(|(_, ty)| matches!(ty, thaw_bridge::DtsType::Native(thaw_hir::HirType::F64)))
        || !matches!(
            &function.ret,
            thaw_bridge::DtsType::Native(thaw_hir::HirType::F64)
        )
    {
        return None;
    }

    let module = thaw_parser::parse_javascript(source).ok()?;
    let [ModuleItem::Stmt(Stmt::Expr(statement))] = module.body.as_slice() else {
        return None;
    };
    let Expr::Assign(assignment) = statement.expr.as_ref() else {
        return None;
    };
    if assignment.op != AssignOp::Assign {
        return None;
    }
    let AssignTarget::Simple(SimpleAssignTarget::Member(target)) = &assignment.left else {
        return None;
    };
    let is_module_exports = matches!(target.obj.as_ref(), Expr::Ident(module) if module.sym == "module")
        && matches!(&target.prop, thaw_parser::ast::MemberProp::Ident(property) if property.sym == "exports")
    ;
    let callable = if is_module_exports {
        if let Expr::Object(object) = assignment.right.as_ref() {
            let mut selected = None;
            for property in &object.props {
                let PropOrSpread::Prop(property) = property else {
                    return None;
                };
                let Prop::KeyValue(property) = property.as_ref() else {
                    return None;
                };
                if !matches!(property.value.as_ref(), Expr::Fn(_) | Expr::Arrow(_)) {
                    return None;
                }
                let is_target = match &property.key {
                    PropName::Ident(identifier) => identifier.sym == export_name,
                    PropName::Str(string) => string.value.to_string_lossy() == export_name,
                    _ => return None,
                };
                if is_target {
                    selected = Some(property.value.as_ref());
                }
            }
            selected?
        } else {
            if !allow_default {
                return None;
            }
            assignment.right.as_ref()
        }
    } else if let Expr::Member(object) = target.obj.as_ref() {
        if matches!(object.obj.as_ref(), Expr::Ident(module) if module.sym == "module")
            && matches!(&object.prop, thaw_parser::ast::MemberProp::Ident(property) if property.sym == "exports")
        {
            let name = match &target.prop {
                thaw_parser::ast::MemberProp::Ident(property) => property.sym.as_ref(),
                _ => return None,
            };
            (name == export_name).then_some(assignment.right.as_ref())?
        } else {
            return None;
        }
    } else if matches!(target.obj.as_ref(), Expr::Ident(exports) if exports.sym == "exports") {
        let name = match &target.prop {
            thaw_parser::ast::MemberProp::Ident(property) => property.sym.as_ref(),
            _ => return None,
        };
        (name == export_name).then_some(assignment.right.as_ref())?
    } else {
        return None;
    };

    let (params, local_initializers, body): (Vec<&Pat>, Vec<(&Ident, &Expr)>, NumericBody<'_>) =
        match callable {
        Expr::Fn(function) if !function.function.is_async && !function.function.is_generator => {
            let body = function.function.body.as_ref()?;
            let (locals, body) = split_numeric_body(&body.stmts)?;
            (
                function.function.params.iter().map(|param| &param.pat).collect(),
                locals,
                body,
            )
        }
        Expr::Arrow(function) if !function.is_async && !function.is_generator => {
            let (locals, body) = match function.body.as_ref() {
                thaw_parser::ast::ArrowFunctionBody::Expr(body) => {
                    (Vec::new(), NumericBody::Expression(body.as_ref()))
                }
                thaw_parser::ast::ArrowFunctionBody::FunctionBody(body) => {
                    split_numeric_body(&body.stmts)?
                }
            };
            (function.params.iter().collect(), locals, body)
        }
        _ => return None,
    };
    let (left, right) = match params.as_slice() {
        [Pat::Ident(left)] => (left.id.sym.as_ref(), ""),
        [Pat::Ident(left), Pat::Ident(right)] => {
            (left.id.sym.as_ref(), right.id.sym.as_ref())
        }
        _ => return None,
    };
    let mut expression = Vec::new();
    let mut locals = std::collections::HashMap::new();
    for (name, initializer) in local_initializers {
        if name.sym == left || name.sym == right || locals.contains_key(name.sym.as_ref()) {
            return None;
        }
        let mut encoded = Vec::new();
        encode_expression(initializer, left, right, &locals, &mut encoded)?;
        locals.insert(name.sym.to_string(), encoded);
    }
    encode_numeric_body(body, left, right, &locals, &mut expression)?;
    validated_jit_expression(expression)
}

fn validated_jit_expression(expression: Vec<String>) -> Option<String> {
    let mut depth = 0usize;
    let mut maximum_depth = 0usize;
    for token in &expression {
        if matches!(
            token.as_str(),
            "+" | "-" | "*" | "/" | "<" | "<=" | ">" | ">=" | "==" | "!="
        ) {
            if depth < 2 {
                return None;
            }
            depth -= 1;
        } else if token == "?" {
            if depth < 3 {
                return None;
            }
            depth -= 2;
        } else {
            depth += 1;
            maximum_depth = maximum_depth.max(depth);
        }
    }
    if depth != 1 || maximum_depth > 6 {
        return None;
    }
    Some(format!("expr:{}", expression.join(",")))
}

fn jit_numeric_declaration(
    package: &str,
    function: &thaw_bridge::DtsFunction,
    operation: &str,
) -> (String, String) {
    let runtime_key = format!("{operation}:{package}::{}", function.name);
    let encoded = runtime_key
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let symbol = format!("__thaw_typed_jit_{encoded}");
    let params = function
        .params
        .iter()
        .map(|(name, _)| format!("{name}: number"))
        .collect::<Vec<_>>()
        .join(", ");
    let declaration = format!("declare function {symbol}({params}): number;\n");
    (symbol, declaration)
}

/// A valid JS/Thaw identifier fragment from an arbitrary package name --
/// `@hapi/hoek` -> `_hapi_hoek`. Used to build a package-qualified alias
/// identifier (`generate_registry_shims`'s collision resolution); doesn't
/// need to be reversible or collision-free against unrelated packages
/// with a similar sanitized form, since it's always combined with the
/// original function name too.
fn sanitize_identifier(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

fn render_dynamic_type(ty: &thaw_hir::HirType) -> Option<String> {
    match ty {
        thaw_hir::HirType::F64 => Some("number".into()),
        thaw_hir::HirType::Str => Some("string".into()),
        thaw_hir::HirType::Bool => Some("boolean".into()),
        thaw_hir::HirType::Void => Some("void".into()),
        thaw_hir::HirType::Json => Some("Json".into()),
        thaw_hir::HirType::JsValue => Some("JsValue".into()),
        thaw_hir::HirType::Optional(payload) => {
            render_dynamic_type(payload).map(|payload| format!("{payload} | undefined"))
        }
        thaw_hir::HirType::Nullable(payload) => {
            render_dynamic_type(payload).map(|payload| format!("{payload} | null"))
        }
        thaw_hir::HirType::Nullish(payload) => render_dynamic_type(payload)
            .map(|payload| format!("{payload} | null | undefined")),
        thaw_hir::HirType::Array(element) => {
            render_dynamic_type(element).map(|rendered| {
                if matches!(
                    element.as_ref(),
                    thaw_hir::HirType::Optional(_)
                        | thaw_hir::HirType::Nullable(_)
                        | thaw_hir::HirType::Nullish(_)
                        | thaw_hir::HirType::Function(_, _)
                ) {
                    format!("({rendered})[]")
                } else {
                    format!("{rendered}[]")
                }
            })
        }
        thaw_hir::HirType::Tuple(elements) => elements
            .iter()
            .map(render_dynamic_type)
            .collect::<Option<Vec<_>>>()
            .map(|elements| format!("[{}]", elements.join(", "))),
        thaw_hir::HirType::Object(fields) => fields
            .iter()
            .map(|(name, ty)| render_dynamic_type(ty).map(|ty| format!("{name}: {ty}")))
            .collect::<Option<Vec<_>>>()
            .map(|fields| format!("{{ {} }}", fields.join("; "))),
        thaw_hir::HirType::Function(params, ret) => {
            let params = params
                .iter()
                .enumerate()
                .map(|(index, ty)| render_dynamic_type(ty).map(|ty| format!("arg{index}: {ty}")))
                .collect::<Option<Vec<_>>>()?;
            let ret = render_dynamic_type(ret)?;
            Some(format!("({}) => {ret}", params.join(", ")))
        }
        thaw_hir::HirType::CallableFunction(params, optional, None, ret) => {
            let params = params
                .iter()
                .enumerate()
                .map(|(index, ty)| {
                    render_dynamic_type(ty).map(|ty| {
                        format!(
                            "arg{index}{}: {ty}",
                            if optional.contains(index) { "?" } else { "" }
                        )
                    })
                })
                .collect::<Option<Vec<_>>>()?;
            let ret = render_dynamic_type(ret)?;
            Some(format!("({}) => {ret}", params.join(", ")))
        }
        _ => None,
    }
}

fn typed_dynamic_callable_adapter(
    encoded: &str,
    target: String,
    mut declarations: String,
    params: &[(String, String)],
    required_params: usize,
    napi: bool,
    ret: &thaw_hir::HirType,
) -> Option<(String, String)> {
    let (callback_params, optional, callback_ret) = match ret {
        thaw_hir::HirType::Function(params, ret) => {
            (params, thaw_hir::HirOptionalMask::default(), ret.as_ref())
        }
        thaw_hir::HirType::CallableFunction(params, optional, None, ret) => {
            (params, optional.clone(), ret.as_ref())
        }
        _ => return Some((target, declarations)),
    };
    let convert = match callback_ret {
        thaw_hir::HirType::Str => "String",
        thaw_hir::HirType::F64 => "Number",
        thaw_hir::HirType::Bool => "Boolean",
        thaw_hir::HirType::Json => "",
        _ => return Some((target, declarations)),
    };
    let callback_types = callback_params
        .iter()
        .map(render_dynamic_type)
        .collect::<Option<Vec<_>>>()?;
    let adapter = format!("__thaw_typed_callable_{encoded}");
    let outer_params = params
        .iter()
        .enumerate()
        .map(|(index, (name, ty))| {
            format!(
                "{name}{}: {ty}",
                if index >= required_params { "?" } else { "" }
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let outer_args = params
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let callback_signature = callback_types
        .iter()
        .enumerate()
        .map(|(index, ty)| {
            format!(
                "arg{index}{}: {ty}",
                if optional.contains(index) { "?" } else { "" }
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let callback_type = render_dynamic_type(ret)?;
    let call_value = if napi {
        "callNativeAddonValue"
    } else {
        "callDynamicValue"
    };
    let required = (0..callback_params.len())
        .take_while(|index| !optional.contains(*index))
        .count();
    declarations.push_str(&format!(
        "function {adapter}({outer_params}): {callback_type} {{\n    const callable: JsValue = {target}({outer_args});\n    const invoke: {callback_type} = ({callback_signature}): {} => {{\n",
        render_dynamic_type(callback_ret)?
    ));
    for arity in (required + 1..=callback_params.len()).rev() {
        let condition = format!("arg{} !== undefined", arity - 1);
        let args = (0..arity)
            .map(|index| format!("arg{index}"))
            .collect::<Vec<_>>()
            .join(", ");
        let call = format!("{call_value}(callable, JSON.parse(JSON.stringify([{args}])))");
        declarations.push_str(&format!(
            "        if ({condition}) return {convert}({call});\n"
        ));
    }
    let args = (0..required)
        .map(|index| format!("arg{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let json_args = if required == 0 {
        "JSON.parse(\"[]\")".to_string()
    } else {
        format!("JSON.parse(JSON.stringify([{args}]))")
    };
    declarations.push_str(&format!(
        "        return {convert}({call_value}(callable, {json_args}));\n    }};\n    return invoke;\n}}\n"
    ));
    Some((adapter, declarations))
}

fn supported_json_collection_element(ty: &thaw_hir::HirType) -> bool {
    match ty {
        thaw_hir::HirType::F64
        | thaw_hir::HirType::Str
        | thaw_hir::HirType::Bool
        | thaw_hir::HirType::Json => true,
        thaw_hir::HirType::Optional(payload)
        | thaw_hir::HirType::Nullable(payload)
        | thaw_hir::HirType::Nullish(payload) => supported_json_collection_element(payload),
        thaw_hir::HirType::Array(element) => supported_json_collection_element(element),
        thaw_hir::HirType::Tuple(elements) => {
            elements.iter().all(supported_json_collection_element)
        }
        thaw_hir::HirType::Object(fields) => fields
            .iter()
            .all(|(_, field)| supported_json_collection_element(field)),
        _ => false,
    }
}

fn typed_dynamic_declaration(
    package: &str,
    function: &thaw_bridge::DtsFunction,
    napi: bool,
) -> Option<(String, String)> {
    let runtime_key = if napi {
        function.name.clone()
    } else {
        format!("{package}::{}", function.name)
    };
    let encoded = runtime_key
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let base_symbol = format!(
        "__thaw_typed_{}_{}",
        if napi { "napi" } else { "js" },
        encoded
    );
    if let Some(generic) = &function.generic {
        if !generic.param_types.iter().all(|ty| {
            generic.type_params.iter().any(|(name, _)| name == ty)
                || ty.starts_with('(')
                || matches!(ty.as_str(), "number" | "string" | "boolean" | "Json" | "JsValue")
        }) {
            return None;
        }
        let type_params = generic
            .type_params
            .iter()
            .map(|(name, constraint)| match constraint {
                Some(constraint) => format!("{name} extends {constraint}"),
                None => name.clone(),
            })
            .collect::<Vec<_>>()
            .join(", ");
        let params = function
            .params
            .iter()
            .zip(&generic.param_types)
            .enumerate()
            .map(|(index, ((name, _), ty))| {
                format!(
                    "{name}{}: {ty}",
                    if index >= function.required_params {
                        "?"
                    } else {
                        ""
                    }
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        return Some((
            base_symbol.clone(),
            format!("declare function {base_symbol}<{type_params}>({params}): JsValue;\n"),
        ));
    }
    let params = function
        .params
        .iter()
        .map(|(name, ty)| match ty {
            thaw_bridge::DtsType::Native(ty) => {
                render_dynamic_type(ty).map(|ty| (name.clone(), ty))
            }
            thaw_bridge::DtsType::Unsupported(_) => None,
        })
        .collect::<Option<Vec<_>>>()?;
    let thaw_bridge::DtsType::Native(ret) = &function.ret else {
        return None;
    };
    // JavaScript function values cross the host boundary as retained handles,
    // not as native Thaw function pointers. Callers can invoke the returned
    // value through callDynamicValue/callDynamicValueHandle.
    let ret = if matches!(
        ret,
        thaw_hir::HirType::Function(_, _) | thaw_hir::HirType::CallableFunction(..)
    ) {
        "JsValue".to_string()
    } else {
        render_dynamic_type(ret)?
    };
    let render_params = |arity: usize| {
        params[..arity]
            .iter()
            .map(|(name, ty)| format!("{name}: {ty}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    if function.required_params == params.len() {
        let declarations = format!(
            "declare function {base_symbol}({}): {ret};\n",
            render_params(params.len())
        );
        return typed_dynamic_callable_adapter(
            &encoded,
            base_symbol,
            declarations,
            &params,
            function.required_params,
            napi,
            match &function.ret {
                thaw_bridge::DtsType::Native(ret) => ret,
                thaw_bridge::DtsType::Unsupported(_) => unreachable!(),
            },
        );
    }
    let wrapper = format!(
        "__thaw_typed_wrapper_{}_{}",
        if napi { "napi" } else { "js" },
        encoded
    );
    let mut declarations = String::new();
    for arity in function.required_params..=params.len() {
        declarations.push_str(&format!(
            "declare function {base_symbol}__arity_{arity}({}): {ret};\n",
            render_params(arity)
        ));
    }
    let wrapper_params = params
        .iter()
        .enumerate()
        .map(|(index, (name, ty))| {
            format!(
                "{name}{}: {ty}",
                if index >= function.required_params {
                    "?"
                } else {
                    ""
                }
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    declarations.push_str(&format!("function {wrapper}({wrapper_params}): {ret} {{\n"));
    for arity in (function.required_params + 1..=params.len()).rev() {
        let condition = &params[arity - 1].0;
        let arguments = params[..arity]
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        declarations.push_str(&format!(
            "    if ({condition} !== undefined) return {base_symbol}__arity_{arity}({arguments});\n"
        ));
    }
    let arguments = params[..function.required_params]
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    declarations.push_str(&format!(
        "    return {base_symbol}__arity_{}({arguments});\n}}\n",
        function.required_params
    ));
    typed_dynamic_callable_adapter(
        &encoded,
        wrapper,
        declarations,
        &params,
        function.required_params,
        napi,
        match &function.ret {
            thaw_bridge::DtsType::Native(ret) => ret,
            thaw_bridge::DtsType::Unsupported(_) => unreachable!(),
        },
    )
}

/// `(package, name, alias)` -- see `rewrite_qualified_calls`. `package`
/// here is the *qualifier identifier* (`qualifier_identifier`), not
/// necessarily the real package name.
type QualifiedCallRewrite = (String, String, String);
/// `(qualifier, class, [(argument_count, helper, parameter_types)])`.
type ClassConstructorRewrite = (
    String,
    String,
    Vec<(usize, String, Vec<thaw_hir::HirType>)>,
);
/// `(class, method, helper, argument_count, has_callback, parameter_types)`.
type ClassMethodRewrite = (String, String, String, usize, bool, Vec<thaw_hir::HirType>);
/// `(class, property, helper)` for an instance getter.
type ClassGetterRewrite = (String, String, String);
/// `(class, property, helper, value_type)` for an instance setter.
type ClassSetterRewrite = (String, String, String, thaw_hir::HirType);
/// `(qualifier, class, property, helper)` for a static getter.
type StaticClassGetterRewrite = (String, String, String, String);
/// `(qualifier, class, property, helper, value_type)` for a static setter.
type StaticClassSetterRewrite = (String, String, String, String, thaw_hir::HirType);
/// `(qualifier, class, method, helper, argument_count, has_callback, parameter_types)`.
type StaticClassMethodRewrite = (
    String,
    String,
    String,
    String,
    usize,
    bool,
    Vec<thaw_hir::HirType>,
);

fn supported_class_method_param(ty: &thaw_bridge::DtsType, index: usize, len: usize) -> bool {
    matches!(
        ty,
        thaw_bridge::DtsType::Native(
            thaw_hir::HirType::F64
                | thaw_hir::HirType::Str
                | thaw_hir::HirType::Bool
                | thaw_hir::HirType::Json
                | thaw_hir::HirType::Object(_)
        )
    ) || matches!(
        ty,
        thaw_bridge::DtsType::Native(
            thaw_hir::HirType::Optional(payload)
                | thaw_hir::HirType::Nullable(payload)
                | thaw_hir::HirType::Nullish(payload)
        )
            if supported_json_collection_element(payload)
    ) || matches!(
        ty,
        thaw_bridge::DtsType::Native(thaw_hir::HirType::Array(element))
            if supported_json_collection_element(element)
    ) || matches!(
        ty,
        thaw_bridge::DtsType::Native(thaw_hir::HirType::Tuple(elements))
            if elements.iter().all(|element| render_dynamic_type(element).is_some())
    ) || matches!(
        ty,
        thaw_bridge::DtsType::Native(thaw_hir::HirType::Function(params, ret))
            if index + 1 == len
                && params.len() <= 2
                && params.iter().all(|param| *param == thaw_hir::HirType::Json)
                && matches!(**ret, thaw_hir::HirType::Json | thaw_hir::HirType::Void)
    )
}

fn supported_class_method_return(ty: &thaw_bridge::DtsType) -> bool {
    matches!(
        ty,
        thaw_bridge::DtsType::Native(
            thaw_hir::HirType::F64
                | thaw_hir::HirType::Str
                | thaw_hir::HirType::Bool
                | thaw_hir::HirType::Json
                | thaw_hir::HirType::Void
                | thaw_hir::HirType::Object(_)
        )
    ) || matches!(
        ty,
        thaw_bridge::DtsType::Native(
            thaw_hir::HirType::Optional(payload)
                | thaw_hir::HirType::Nullable(payload)
                | thaw_hir::HirType::Nullish(payload)
        )
            if supported_json_collection_element(payload)
    ) || matches!(
        ty,
        thaw_bridge::DtsType::Native(thaw_hir::HirType::Array(element))
            if supported_json_collection_element(element)
    ) || matches!(
        ty,
        thaw_bridge::DtsType::Native(thaw_hir::HirType::Tuple(elements))
            if elements.iter().all(|element| render_dynamic_type(element).is_some())
    )
}

fn generate_napi_class_constructors(
    class: &thaw_bridge::DtsClass,
    shim: &mut String,
) -> Vec<(usize, String, Vec<thaw_hir::HirType>)> {
    if !class.constructible {
        return Vec::new();
    }
    let mut helpers = Vec::new();
    for (overload_index, constructor) in class.constructors.iter().enumerate() {
        if !constructor.params.iter().all(|(_, ty)| {
            matches!(ty, thaw_bridge::DtsType::Native(native) if render_dynamic_type(native).is_some())
        }) {
            continue;
        }
        for arity in constructor.required_params..=constructor.params.len() {
            let params = &constructor.params[..arity];
            let parameter_types = params
                .iter()
                .filter_map(|(_, ty)| match ty {
                    thaw_bridge::DtsType::Native(ty) => Some(ty.clone()),
                    thaw_bridge::DtsType::Unsupported(_) => None,
                })
                .collect::<Vec<_>>();
            if helpers.iter().any(|(existing_arity, _, existing_types)| {
                *existing_arity == arity && existing_types == &parameter_types
            }) {
                continue;
            }
            let rendered = params
                .iter()
                .map(|(name, ty)| match ty {
                    thaw_bridge::DtsType::Native(ty) => {
                        format!("{name}: {}", render_dynamic_type(ty).unwrap())
                    }
                    thaw_bridge::DtsType::Unsupported(_) => unreachable!(),
                })
                .collect::<Vec<_>>()
                .join(", ");
            let overload = if class.constructors.len() > 1 {
                format!("$overload{overload_index}")
            } else {
                String::new()
            };
            let runtime_key = format!("$new${}$arity{arity}{overload}", class.name);
            let encoded = runtime_key
                .as_bytes()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            let symbol = format!("__thaw_typed_napi_{encoded}");
            shim.push_str(&format!(
                "declare function {symbol}({rendered}): JsValue;\n"
            ));
            helpers.push((arity, symbol, parameter_types));
        }
    }
    if class.constructors.is_empty() {
        let runtime_key = format!("$new${}$arity0", class.name);
        let encoded = runtime_key
            .as_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let symbol = format!("__thaw_typed_napi_{encoded}");
        shim.push_str(&format!(
            "declare function {symbol}(): JsValue;\n"
        ));
        helpers.push((0, symbol, Vec::new()));
    }
    helpers
}

fn generate_napi_class_property_getter(
    class: &str,
    property: &str,
    ty: &thaw_bridge::DtsType,
    is_static: bool,
    shim: &mut String,
) -> Option<String> {
    let thaw_bridge::DtsType::Native(ty) = ty else {
        return None;
    };
    let rendered = render_dynamic_type(ty)?;
    let prefix = if is_static { "staticgetter" } else { "getter" };
    let runtime_key = format!("${prefix}${class}${property}");
    let encoded = runtime_key
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let symbol = format!("__thaw_typed_napi_{encoded}");
    shim.push_str(&format!(
        "declare function {symbol}({}): {rendered};\n",
        if is_static { "" } else { "receiver: JsValue" }
    ));
    Some(symbol)
}

fn generate_napi_class_property_setter(
    class: &str,
    property: &str,
    ty: &thaw_bridge::DtsType,
    is_static: bool,
    shim: &mut String,
) -> Option<(String, thaw_hir::HirType)> {
    let thaw_bridge::DtsType::Native(ty) = ty else {
        return None;
    };
    let rendered = render_dynamic_type(ty)?;
    let prefix = if is_static { "staticsetter" } else { "setter" };
    let runtime_key = format!("${prefix}${class}${property}");
    let encoded = runtime_key
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let symbol = format!("__thaw_typed_napi_{encoded}");
    shim.push_str(&format!(
        "declare function {symbol}({}value: {rendered}): {rendered};\n",
        if is_static { "" } else { "receiver: JsValue, " }
    ));
    Some((symbol, ty.clone()))
}

fn generate_napi_class_method_overloads(
    class: &thaw_bridge::DtsClass,
    is_static: bool,
    observed_arities: &std::collections::HashMap<String, std::collections::BTreeSet<usize>>,
    shim: &mut String,
) -> Vec<(String, String, usize, bool, Vec<thaw_hir::HirType>)> {
    let mut generated = Vec::new();
    let mut method_names = std::collections::HashSet::new();
    for method in &class.methods {
        if method.is_static != is_static
            || method.kind != thaw_bridge::DtsMethodKind::Method
            || !method_names.insert(method.name.clone())
        {
            continue;
        }
        let overloads = class
            .methods
            .iter()
            .filter(|candidate| {
                candidate.name == method.name
                    && candidate.is_static == is_static
                    && candidate.kind == thaw_bridge::DtsMethodKind::Method
            })
            .filter(|candidate| {
                candidate.params.iter().enumerate().all(|(index, (_, ty))| {
                    supported_class_method_param(ty, index, candidate.params.len())
                }) && candidate
                    .rest_param
                    .as_ref()
                    .is_none_or(|(_, ty)| supported_class_method_param(ty, 0, 1))
                    && supported_class_method_return(&candidate.ret)
            })
            .collect::<Vec<_>>();
        for (overload_index, overload) in overloads.into_iter().enumerate() {
            let thaw_bridge::DtsType::Native(return_type) = &overload.ret else {
                continue;
            };
            let Some(return_type) = (if *return_type == thaw_hir::HirType::Void {
                Some("Json".to_string())
            } else {
                render_dynamic_type(return_type)
            }) else {
                continue;
            };
            let argument_counts = if overload.rest_param.is_some() {
                observed_arities
                    .get(&method.name)
                    .into_iter()
                    .flatten()
                    .copied()
                    .filter(|count| *count >= overload.required_params)
                    .collect::<Vec<_>>()
            } else {
                (overload.required_params..=overload.params.len()).collect()
            };
            for argument_count in argument_counts {
                let fixed_count = argument_count.min(overload.params.len());
                let mut included_params = overload.params[..fixed_count]
                    .iter()
                    .filter_map(|(name, ty)| match ty {
                        thaw_bridge::DtsType::Native(ty) => Some((name.clone(), ty.clone())),
                        thaw_bridge::DtsType::Unsupported(_) => None,
                    })
                    .collect::<Vec<_>>();
                if argument_count > overload.params.len() {
                    let Some((name, thaw_bridge::DtsType::Native(rest_type))) =
                        &overload.rest_param
                    else {
                        continue;
                    };
                    included_params.extend(
                        (overload.params.len()..argument_count)
                            .map(|index| (format!("{name}{index}"), rest_type.clone())),
                    );
                }
                let params = (if is_static {
                    Vec::new()
                } else {
                    vec!["receiver: JsValue".to_string()]
                })
                .into_iter()
                .chain(included_params.iter().map(|(name, ty)| {
                    format!(
                        "{name}: {}",
                        render_dynamic_type(ty).expect("filtered above")
                    )
                }))
                .collect::<Vec<_>>()
                .join(", ");
                let has_callback = matches!(
                    included_params.last(),
                    Some((_, thaw_hir::HirType::Function(_, _)))
                );
                let runtime_key = format!(
                    "{}{}${}$overload{overload_index}$arity{argument_count}",
                    match (is_static, &overload.ret) {
                        (true, thaw_bridge::DtsType::Native(thaw_hir::HirType::Void)) => {
                            "$staticmethodvoid$"
                        }
                        (true, _) => "$staticmethod$",
                        (false, thaw_bridge::DtsType::Native(thaw_hir::HirType::Void)) => {
                            "$methodvoid$"
                        }
                        (false, _) => "$method$",
                    },
                    class.name,
                    method.name
                );
                let encoded = runtime_key
                    .as_bytes()
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>();
                let symbol = format!("__thaw_typed_napi_{encoded}");
                shim.push_str(&format!(
                    "declare function {symbol}({params}): {return_type};\n"
                ));
                generated.push((
                    method.name.clone(),
                    symbol,
                    argument_count,
                    has_callback,
                    included_params.into_iter().map(|(_, ty)| ty).collect(),
                ));
            }
        }
    }
    generated
}
type ExternalExports = std::collections::HashMap<String, std::collections::HashMap<String, String>>;
type RegistryShims = (
    String,
    Vec<PathBuf>,
    Vec<QualifiedCallRewrite>,
    Vec<ClassConstructorRewrite>,
    Vec<ClassMethodRewrite>,
    Vec<StaticClassMethodRewrite>,
    Vec<ClassGetterRewrite>,
    Vec<ClassSetterRewrite>,
    Vec<StaticClassGetterRewrite>,
    Vec<StaticClassSetterRewrite>,
    ExternalExports,
);

/// The identifier a user writes as the object in `pkg.name(...)`
/// qualified-call syntax for a `--use`d package. A scoped package's real
/// name (`@hapi/hoek`) isn't a valid identifier at all (`@`, `/`), so
/// this uses its last path segment (`hoek`) instead -- an unscoped name
/// has no `/` to split on and passes through unchanged. Two different
fn qualifier_identifier(package: &str) -> &str {
    package.rsplit('/').next().unwrap_or(package)
}

fn package_qualifier_identifiers<'a>(
    packages: impl IntoIterator<Item = &'a str>,
) -> std::collections::HashMap<String, String> {
    let packages = packages
        .into_iter()
        .map(str::to_string)
        .collect::<std::collections::BTreeSet<_>>();
    let mut counts = std::collections::HashMap::new();
    for package in &packages {
        *counts
            .entry(qualifier_identifier(package).to_string())
            .or_insert(0usize) += 1;
    }
    let candidates = packages
        .into_iter()
        .map(|package| {
            let base = qualifier_identifier(&package);
            let qualifier = if counts[base] == 1 {
                base.to_string()
            } else {
                sanitize_identifier(&package)
            };
            (package, qualifier)
        })
        .collect::<Vec<_>>();
    let mut candidate_counts = std::collections::HashMap::new();
    for (_, candidate) in &candidates {
        *candidate_counts.entry(candidate.clone()).or_insert(0usize) += 1;
    }
    candidates
        .into_iter()
        .map(|(package, candidate)| {
            if candidate_counts[candidate.as_str()] == 1 {
                (package, candidate)
            } else {
                let encoded = package
                    .as_bytes()
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>();
                (package, format!("{candidate}__{encoded}"))
            }
        })
        .collect()
}

fn is_native_builtin(package: &str) -> bool {
    matches!(package, "node:fs" | "node:http")
}

fn observed_member_call_arities(
    source: &str,
) -> Result<std::collections::HashMap<String, std::collections::BTreeSet<usize>>, String> {
    use swc_ecma_visit::{Visit, VisitWith};
    use thaw_parser::ast::{CallExpr, Callee, Expr, MemberProp};

    #[derive(Default)]
    struct Finder {
        arities: std::collections::HashMap<String, std::collections::BTreeSet<usize>>,
    }

    impl Visit for Finder {
        fn visit_call_expr(&mut self, call: &CallExpr) {
            if let Callee::Expr(callee) = &call.callee {
                if let Expr::Member(member) = callee.as_ref() {
                    if let MemberProp::Ident(method) = &member.prop {
                        self.arities
                            .entry(method.sym.to_string())
                            .or_default()
                            .insert(call.args.len());
                    }
                }
            }
            call.visit_children_with(self);
        }
    }

    let module = thaw_parser::parse_typescript(source)?;
    let mut finder = Finder::default();
    module.visit_with(&mut finder);
    Ok(finder.arities)
}

fn commonjs_export_name(source: &str) -> Result<Option<String>, String> {
    use thaw_parser::ast::{Expr, ModuleDecl, ModuleItem};

    let module = thaw_parser::parse_typescript(source)?;
    Ok(module.body.iter().find_map(|item| match item {
        ModuleItem::ModuleDecl(ModuleDecl::TsExportAssignment(export)) => {
            if let Expr::Ident(identifier) = export.expr.as_ref() {
                Some(identifier.sym.to_string())
            } else {
                None
            }
        }
        _ => None,
    }))
}

/// Resolves each `--use`d package against the local registry
/// (thaw-registry; `registry_dir` defaults to `thaw_modules/`),
/// generating its callable surface exactly like `generate_bridge_shims`
/// does for a standalone `.d.ts` -- but additionally auto-linking the
/// package's `native.a` if it ships one (replacing a manual `--link`),
/// and collecting its `bundle.js` (if any) into a single generated
/// `__thaw_module_init` (thaw-bridge's `generate_module_init`) so it's
/// auto-loaded before user code runs (replacing a manual `loadScript`
/// call). Returns the generated shim text, the native lib paths to
/// link, and any cross-package name-collision rewrites the caller must
/// also apply to the user's own source (`rewrite_qualified_calls`).
fn generate_registry_shims(
    registry_dir: &Path,
    use_packages: &[String],
    user_source: &str,
) -> Result<RegistryShims, String> {
    let observed_arities = observed_member_call_arities(user_source)?;
    let mut resolved = Vec::new();
    for name in use_packages {
        let package = if name.starts_with("node:") {
            thaw_registry::resolve_builtin(name)?
        } else {
            thaw_registry::resolve(registry_dir, name)?
        };
        let functions = thaw_bridge::parse_dts(&package.dts_source)
            .map_err(|e| format!("failed to parse `{name}`'s package.d.ts: {e}"))?;
        let classes = thaw_bridge::parse_dts_classes(&package.dts_source)
            .map_err(|e| format!("failed to parse `{name}`'s package.d.ts classes: {e}"))?;
        let commonjs_export_name = commonjs_export_name(&package.dts_source)
            .map_err(|e| format!("failed to parse `{name}`'s CommonJS export: {e}"))?;
        // Whether there's actually a `native.a` to link a FastPath
        // signature against -- without one, a fully-primitive real npm
        // function (e.g. date-fns's `daysToWeeks(days: number): number`)
        // would still classify FastPath on type shape alone and produce
        // an unresolvable `declare function`, even though a working JS
        // implementation is sitting right there in `bundle.js`. See
        // `thaw_bridge::effective_classifications`'s doc comment.
        let native_lib_available = package.native_lib.is_some() || is_native_builtin(&package.name);
        let classifications =
            thaw_bridge::effective_classifications(&functions, native_lib_available);
        resolved.push(ResolvedPackage {
            name: package.name.clone(),
            commonjs_export_name,
            functions,
            classes,
            classifications,
            native_lib: package.native_lib,
            native_addon: package.native_addon,
            bundle_js: package.bundle_js,
        });
    }

    // Every generated top-level name -- FastPath ambient declaration or
    // Fallback wrapper alike -- would otherwise land in the *same* flat
    // global scope (QuickJS-NG globals for Fallback, the LLVM module's
    // own symbol table for FastPath). Two different packages exporting
    // the same name (e.g. `qs` and `@hapi/hoek` both export `stringify`)
    // used to silently collide: whichever package's shim/binding ran
    // last won, with no error -- exactly the kind of order-dependent
    // surprise this project has treated as a bug to catch loudly every
    // other time it showed up (see thaw-bridge's `classify_all`, for the
    // same problem one level down, *within* one `.d.ts`'s own
    // overloads). A collision where every involved package classifies
    // the name as Fallback is auto-resolved below by dropping the bare
    // name for it (`QualifiedFallback::suppress_bare`), forcing
    // qualified syntax (`qs.stringify(x)`, rewritten to a package-
    // qualified alias -- see `rewrite_qualified_calls`); a
    // FastPath-involved collision is a real native-symbol clash this
    // can't paper over, so it stays a hard error.
    let mut declared_by: std::collections::HashMap<String, Vec<(String, bool)>> =
        std::collections::HashMap::new();
    for pkg in &resolved {
        for (name, classification) in &pkg.classifications {
            let is_fast_path = matches!(classification, thaw_bridge::Classification::FastPath(_));
            declared_by
                .entry(name.clone())
                .or_default()
                .push((pkg.name.clone(), is_fast_path));
        }
    }
    for (name, packages) in &declared_by {
        if packages.len() < 2 {
            continue;
        }
        if let Some((fast_path_pkg, _)) = packages.iter().find(|(_, is_fast_path)| *is_fast_path) {
            let other_pkg = packages
                .iter()
                .map(|(p, _)| p.as_str())
                .find(|p| *p != fast_path_pkg)
                .unwrap_or(fast_path_pkg);
            return Err(format!(
                "`{name}` is declared by both `{fast_path_pkg}` and `{other_pkg}` -- automatic \
                 resolution only covers Fallback functions, not a Fast path native symbol clash"
            ));
        }
    }
    let colliding: std::collections::HashSet<&String> = declared_by
        .iter()
        .filter(|(_, pkgs)| pkgs.len() > 1)
        .map(|(name, _)| name)
        .collect();
    let qualifier_by_package =
        package_qualifier_identifiers(resolved.iter().map(|package| package.name.as_str()));

    // Every Fallback name of every `--use`d package also gets a package-
    // qualified alias -- not just names that actually collide -- so
    // `pkg.name(...)` syntax works consistently for any `--use`d
    // package's function, whether or not `name` happens to collide with
    // some other package (see `thaw_bridge::QualifiedFallback`'s doc
    // comment: `suppress_bare` is the only thing collision status
    // changes). `rewrites` is `(package, name, alias)`, for rewriting
    // `pkg.name(...)` call syntax in the user's own source (see
    // `rewrite_qualified_calls`).
    let mut qualified_by_package: std::collections::HashMap<
        String,
        Vec<thaw_bridge::QualifiedFallback>,
    > = std::collections::HashMap::new();
    let mut rewrites: Vec<QualifiedCallRewrite> = Vec::new();
    for pkg in &resolved {
        for (name, classification) in &pkg.classifications {
            if !matches!(classification, thaw_bridge::Classification::Fallback { .. }) {
                continue;
            }
            let alias = format!("{}_{name}", sanitize_identifier(&pkg.name));
            let qualified_key = format!("{}::{name}", pkg.name);
            qualified_by_package
                .entry(pkg.name.clone())
                .or_default()
                .push(thaw_bridge::QualifiedFallback {
                    name: name.clone(),
                    alias: alias.clone(),
                    qualified_key,
                    suppress_bare: colliding.contains(name),
                });
            rewrites.push((qualifier_by_package[&pkg.name].clone(), name.clone(), alias));
        }
    }

    /// `(package_name, js_source, fallback_function_names, qualified_aliases)`
    /// -- kept as owned data so the borrowed `ModuleBundle`s built from it
    /// below can outlive the loop that collects it.
    type PendingBundle = (String, String, Vec<String>, Vec<(String, String)>);

    let mut shim = String::new();
    let mut typed_targets: std::collections::HashMap<(String, String), String> =
        std::collections::HashMap::new();
    let mut jit_targets = std::collections::HashSet::new();
    let mut class_targets: std::collections::HashMap<(String, String), String> =
        std::collections::HashMap::new();
    let mut class_rewrites = Vec::new();
    let mut class_method_rewrites = Vec::new();
    let mut static_class_method_rewrites = Vec::new();
    let mut class_getter_rewrites = Vec::new();
    let mut class_setter_rewrites = Vec::new();
    let mut static_class_getter_rewrites = Vec::new();
    let mut static_class_setter_rewrites = Vec::new();
    let mut native_libs = Vec::new();
    let mut bundles: Vec<PendingBundle> = Vec::new();
    let mut native_addons: Vec<(String, Vec<u8>, Option<String>)> = Vec::new();
    let no_qualified: Vec<thaw_bridge::QualifiedFallback> = Vec::new();

    for pkg in &resolved {
        let native_lib_available = pkg.native_lib.is_some() || is_native_builtin(&pkg.name);
        let qualified = qualified_by_package.get(&pkg.name).unwrap_or(&no_qualified);
        if pkg.native_addon.is_some() {
            for class in &pkg.classes {
                let helpers = generate_napi_class_constructors(class, &mut shim);
                if helpers.is_empty() {
                    continue;
                }
                class_targets.insert(
                    (pkg.name.clone(), class.name.clone()),
                    helpers[0].1.clone(),
                );
                class_rewrites.push((
                    qualifier_by_package[&pkg.name].clone(),
                    class.name.clone(),
                    helpers,
                ));

                for (method, symbol, argument_count, has_callback, parameter_types) in
                    generate_napi_class_method_overloads(class, false, &observed_arities, &mut shim)
                {
                    class_method_rewrites.push((
                        class.name.clone(),
                        method,
                        symbol,
                        argument_count,
                        has_callback,
                        parameter_types,
                    ));
                }
                for getter in class.methods.iter().filter(|method| {
                    !method.is_static
                        && method.kind == thaw_bridge::DtsMethodKind::Getter
                        && method.params.is_empty()
                        && supported_class_method_return(&method.ret)
                }) {
                    let thaw_bridge::DtsType::Native(return_type) = &getter.ret else {
                        continue;
                    };
                    let Some(return_type) = render_dynamic_type(return_type) else {
                        continue;
                    };
                    let runtime_key =
                        format!("$getter${}{}", class.name, format_args!("${}", getter.name));
                    let encoded = runtime_key
                        .as_bytes()
                        .iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect::<String>();
                    let symbol = format!("__thaw_typed_napi_{encoded}");
                    shim.push_str(&format!(
                        "declare function {symbol}(receiver: JsValue): {return_type};\n"
                    ));
                    class_getter_rewrites.push((class.name.clone(), getter.name.clone(), symbol));
                }
                for setter in class.methods.iter().filter(|method| {
                    !method.is_static
                        && method.kind == thaw_bridge::DtsMethodKind::Setter
                        && method.params.len() == 1
                        && supported_class_method_param(&method.params[0].1, 0, 1)
                }) {
                    let thaw_bridge::DtsType::Native(value_type) = &setter.params[0].1 else {
                        continue;
                    };
                    let Some(rendered_type) = render_dynamic_type(value_type) else {
                        continue;
                    };
                    let runtime_key =
                        format!("$setter${}{}", class.name, format_args!("${}", setter.name));
                    let encoded = runtime_key
                        .as_bytes()
                        .iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect::<String>();
                    let symbol = format!("__thaw_typed_napi_{encoded}");
                    shim.push_str(&format!(
                        "declare function {symbol}(receiver: JsValue, value: {rendered_type}): {rendered_type};\n"
                    ));
                    class_setter_rewrites.push((
                        class.name.clone(),
                        setter.name.clone(),
                        symbol,
                        value_type.clone(),
                    ));
                }
                for getter in class.methods.iter().filter(|method| {
                    method.is_static
                        && method.kind == thaw_bridge::DtsMethodKind::Getter
                        && method.params.is_empty()
                        && supported_class_method_return(&method.ret)
                }) {
                    let thaw_bridge::DtsType::Native(return_type) = &getter.ret else {
                        continue;
                    };
                    let Some(return_type) = render_dynamic_type(return_type) else {
                        continue;
                    };
                    let runtime_key = format!(
                        "$staticgetter${}{}",
                        class.name,
                        format_args!("${}", getter.name)
                    );
                    let encoded = runtime_key
                        .as_bytes()
                        .iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect::<String>();
                    let symbol = format!("__thaw_typed_napi_{encoded}");
                    shim.push_str(&format!("declare function {symbol}(): {return_type};\n"));
                    static_class_getter_rewrites.push((
                        qualifier_by_package[&pkg.name].clone(),
                        class.name.clone(),
                        getter.name.clone(),
                        symbol,
                    ));
                }
                for setter in class.methods.iter().filter(|method| {
                    method.is_static
                        && method.kind == thaw_bridge::DtsMethodKind::Setter
                        && method.params.len() == 1
                        && supported_class_method_param(&method.params[0].1, 0, 1)
                }) {
                    let thaw_bridge::DtsType::Native(value_type) = &setter.params[0].1 else {
                        continue;
                    };
                    let Some(rendered_type) = render_dynamic_type(value_type) else {
                        continue;
                    };
                    let runtime_key = format!(
                        "$staticsetter${}{}",
                        class.name,
                        format_args!("${}", setter.name)
                    );
                    let encoded = runtime_key
                        .as_bytes()
                        .iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect::<String>();
                    let symbol = format!("__thaw_typed_napi_{encoded}");
                    shim.push_str(&format!(
                        "declare function {symbol}(value: {rendered_type}): {rendered_type};\n"
                    ));
                    static_class_setter_rewrites.push((
                        qualifier_by_package[&pkg.name].clone(),
                        class.name.clone(),
                        setter.name.clone(),
                        symbol,
                        value_type.clone(),
                    ));
                }
                for property in &class.properties {
                    let has_getter = class.methods.iter().any(|method| {
                        method.name == property.name
                            && method.is_static == property.is_static
                            && method.kind == thaw_bridge::DtsMethodKind::Getter
                    });
                    if !has_getter {
                        if let Some(symbol) = generate_napi_class_property_getter(
                            &class.name,
                            &property.name,
                            &property.ty,
                            property.is_static,
                            &mut shim,
                        ) {
                            if property.is_static {
                                static_class_getter_rewrites.push((
                                    qualifier_by_package[&pkg.name].clone(),
                                    class.name.clone(),
                                    property.name.clone(),
                                    symbol,
                                ));
                            } else {
                                class_getter_rewrites.push((
                                    class.name.clone(),
                                    property.name.clone(),
                                    symbol,
                                ));
                            }
                        }
                    }

                    let has_setter = class.methods.iter().any(|method| {
                        method.name == property.name
                            && method.is_static == property.is_static
                            && method.kind == thaw_bridge::DtsMethodKind::Setter
                    });
                    if !property.readonly && !has_setter {
                        if let Some((symbol, value_type)) = generate_napi_class_property_setter(
                            &class.name,
                            &property.name,
                            &property.ty,
                            property.is_static,
                            &mut shim,
                        ) {
                            if property.is_static {
                                static_class_setter_rewrites.push((
                                    qualifier_by_package[&pkg.name].clone(),
                                    class.name.clone(),
                                    property.name.clone(),
                                    symbol,
                                    value_type,
                                ));
                            } else {
                                class_setter_rewrites.push((
                                    class.name.clone(),
                                    property.name.clone(),
                                    symbol,
                                    value_type,
                                ));
                            }
                        }
                    }
                }
                for (method, symbol, argument_count, has_callback, parameter_types) in
                    generate_napi_class_method_overloads(class, true, &observed_arities, &mut shim)
                {
                    static_class_method_rewrites.push((
                        qualifier_by_package[&pkg.name].clone(),
                        class.name.clone(),
                        method,
                        symbol,
                        argument_count,
                        has_callback,
                        parameter_types,
                    ));
                }
            }
        }
        for function in &pkg.functions {
            let is_fallback = pkg.classifications.iter().any(|(name, classification)| {
                name == &function.name
                    && matches!(classification, thaw_bridge::Classification::Fallback { .. })
            });
            if is_fallback {
                let jit_operation = pkg
                    .bundle_js
                    .as_deref()
                    .filter(|_| pkg.native_addon.is_none())
                    .and_then(|source| {
                        jit_numeric_export(
                            source,
                            &function.name,
                            pkg.commonjs_export_name.as_deref() == Some(&function.name)
                                || pkg.functions.len() == 1,
                            function,
                        )
                    });
                let declaration = jit_operation
                    .as_deref()
                    .map(|operation| jit_numeric_declaration(&pkg.name, function, operation))
                    .or_else(|| {
                        typed_dynamic_declaration(
                        &pkg.name,
                        function,
                        pkg.native_addon.is_some() && pkg.bundle_js.is_none(),
                    )
                    });
                if let Some((symbol, declaration)) = declaration {
                    shim.push_str(&declaration);
                    typed_targets.insert((pkg.name.clone(), function.name.clone()), symbol);
                    if jit_operation.is_some() {
                        jit_targets.insert((pkg.name.clone(), function.name.clone()));
                    }
                }
            }
        }
        if pkg.native_addon.is_some() && pkg.bundle_js.is_none() {
            shim.push_str(&thaw_bridge::generate_native_addon_shim(
                &pkg.functions,
                qualified,
            ));
        } else {
            shim.push_str(&thaw_bridge::generate_shim(
                &pkg.functions,
                native_lib_available,
                qualified,
            ));
        }

        if let Some(native_lib) = &pkg.native_lib {
            native_libs.push(native_lib.clone());
        }
        if let Some(native_addon) = &pkg.native_addon {
            let bytes = std::fs::read(native_addon).map_err(|error| {
                format!(
                    "failed to embed native addon `{}`: {error}",
                    native_addon.display()
                )
            })?;
            native_addons.push((
                pkg.name.clone(),
                bytes,
                pkg.commonjs_export_name.clone().or_else(|| {
                    (pkg.functions.len() == 1).then(|| pkg.functions[0].name.clone())
                }),
            ));
        }
        if let Some(bundle_js) = &pkg.bundle_js {
            // Only Fallback functions need binding inside the loaded
            // script (see `ModuleBundle::fallback_names`'s doc comment);
            // FastPath functions are real FFI calls and never touch
            // QuickJS-NG at all.
            let fallback_names: Vec<String> = pkg
                .classifications
                .iter()
                .filter_map(|(name, classification)| match classification {
                    thaw_bridge::Classification::Fallback { .. }
                        if !jit_targets.contains(&(pkg.name.clone(), name.clone())) =>
                    {
                        Some(name.clone())
                    }
                    thaw_bridge::Classification::FastPath(_) => None,
                    thaw_bridge::Classification::Fallback { .. } => None,
                })
                .collect();
            let qualified_aliases = qualified
                .iter()
                .map(|q| (q.name.clone(), q.qualified_key.clone()))
                .collect();
            if !fallback_names.is_empty() || pkg.native_addon.is_some() {
                bundles.push((
                    pkg.name.clone(),
                    bundle_js.clone(),
                    fallback_names,
                    qualified_aliases,
                ));
            }
        }
    }

    let module_bundles: Vec<thaw_bridge::ModuleBundle> = bundles
        .iter()
        .map(
            |(name, js, fallback_names, qualified_aliases)| thaw_bridge::ModuleBundle {
                package_name: name.as_str(),
                js_source: js.as_str(),
                fallback_names,
                qualified_aliases,
            },
        )
        .collect();
    shim.push_str(&thaw_bridge::generate_module_init(&module_bundles));
    let native_addons: Vec<thaw_bridge::NativeAddon<'_>> = native_addons
        .iter()
        .map(|(name, bytes, root_export)| thaw_bridge::NativeAddon {
            package_name: name,
            bytes,
            root_export: root_export.as_deref(),
        })
        .collect();
    shim.push_str(&thaw_bridge::generate_native_addon_init(&native_addons));

    let mut external_exports = ExternalExports::new();
    for pkg in &resolved {
        let mut package_exports = std::collections::HashMap::new();
        for (name, classification) in &pkg.classifications {
            let target = if matches!(classification, thaw_bridge::Classification::Fallback { .. }) {
                typed_targets
                    .get(&(pkg.name.clone(), name.clone()))
                    .cloned()
                    .unwrap_or_else(|| format!("{}_{name}", sanitize_identifier(&pkg.name)))
            } else {
                name.clone()
            };
            package_exports.insert(name.clone(), target.clone());
        }
        for class in &pkg.classes {
            if let Some(target) = class_targets.get(&(pkg.name.clone(), class.name.clone())) {
                package_exports.insert(class.name.clone(), target.clone());
            }
        }
        if let Some(target) = pkg
            .commonjs_export_name
            .as_ref()
            .and_then(|name| package_exports.get(name))
            .cloned()
        {
            package_exports.insert("default".to_string(), target);
        } else if package_exports.len() == 1 {
            let target = package_exports.values().next().unwrap().clone();
            package_exports.insert("default".to_string(), target);
        }
        external_exports.insert(pkg.name.clone(), package_exports);
    }

    Ok((
        shim,
        native_libs,
        rewrites,
        class_rewrites,
        class_method_rewrites,
        static_class_method_rewrites,
        class_getter_rewrites,
        class_setter_rewrites,
        static_class_getter_rewrites,
        static_class_setter_rewrites,
        external_exports,
    ))
}
